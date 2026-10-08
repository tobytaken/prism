//! The switching engine. While a user's linked game runs, it applies the linked profile to each
//! linked GPU through that GPU's operation queue; once a complete scan shows the game ended, it
//! restores exactly the settings automation wrote. Pre-game tuning values live in boot-scoped
//! state stamped with the boot id, so they survive a daemon restart but never a reboot.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use super::backend::DaemonBackend;
use super::cooling::CoolingController;
use super::host::{Clock, UserDirectory};
use super::profiles::{capture_cooling, reading, verified_gpu};
use super::recognition::{GamesByUser, Recognition};
use super::session::SessionPeer;
use super::tuning::apply_unreviewed_profile;
use super::tuning_store::{write_atomically, TuningDocument, TuningStore};
use crate::cooling::CoolingConfiguration;
use crate::protocol::{
    ApplyGpuProfileRequest, AutomaticSwitchingStatus, CurrentProfile, DaemonEvent, GameId,
    GpuSwitchingState, GpuSwitchingStatus, ProfileSetting, ProfileSettingKey, ProfileSettings,
    SwitchingCondition, TuningOutcome, TuningOutcomeStatus,
};

pub(super) const SWITCHING_STATE_FILE_NAME: &str = "switching-state.json";
// Version 1 has numeric baselines only. Preserve and reject older files for this boot rather
// than discard restoration state on upgrade; a normal reboot clears this boot-scoped file.
const SCHEMA_VERSION: u64 = 2;

/// The user's running game, with its linked profile, that automatic switching puts into effect
/// on a GPU.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DrivingGame {
    uid: u32,
    username: String,
    game: GameId,
    profile_id: String,
}

/// Automation's hold on one GPU, from its first automatic write until full restoration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Activation {
    /// Observed absence survives a daemon restart, so return reconciles even the same game.
    #[serde(default)]
    gpu_absent: bool,
    /// The successful application, including the controller's full cooling configuration.
    /// Returning hardware reconciles this snapshot, not edits to the saved profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    applied_settings: Option<ProfileSettings>,
    /// The game automation last put, or tried to put, into effect; none once it restores.
    driving: Option<DrivingGame>,
    /// The pre-game tuning values: the actual value of each setting automation wrote, captured
    /// when it first wrote that setting.
    baseline: BTreeMap<ProfileSettingKey, i64>,
    /// The complete cooling configuration, never a curve's sampled output.
    cooling_baseline: Option<CoolingConfiguration>,
    /// Fan identity at capture, including for a shared configuration. An older record without
    /// these identities is retained but cannot safely restore cooling.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    cooling_baseline_fans: BTreeSet<u32>,
    /// The current profile before automation, which returns after full restoration.
    previous_profile: Option<String>,
    /// Why the latest automatic application or restoration didn't complete. It isn't retried
    /// until what drives the GPU changes.
    failure: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SwitchingDocument {
    schema_version: u64,
    boot_id: String,
    activations: BTreeMap<String, Activation>,
}

type Activations = BTreeMap<String, Activation>;

/// The boot-scoped activations. Only a GPU's own queue changes its activation.
struct SwitchingState {
    path: PathBuf,
    boot_id: String,
    /// Unusable when the boot-scoped file can't be read; automation is then blocked this boot.
    activations: Mutex<Result<Activations, String>>,
    /// UUIDs identifiable in an unreadable record; never inferred from a slot or replacement.
    recovery_gpus: BTreeSet<String>,
    /// GPUs with a reconciliation queued that hasn't started yet.
    queued: Mutex<BTreeSet<String>>,
    /// Serializes status pushes, so an older status is never pushed after a newer one.
    publishing: Mutex<()>,
    /// The status last pushed to each user.
    published: Mutex<HashMap<(u32, String), AutomaticSwitchingStatus>>,
}

impl SwitchingState {
    fn activations(&self) -> Result<Activations, String> {
        self.activations.lock().unwrap().clone()
    }

    /// Changes the GPU's activation, or removes it when none is left, and writes the result.
    /// The change is kept in memory even when it can't be written.
    fn set(&self, gpu_id: &str, activation: Option<Activation>) -> Result<(), String> {
        let mut state = self.activations.lock().unwrap();
        let activations = state.as_mut().map_err(|error| error.clone())?;
        match activation {
            Some(activation) => activations.insert(gpu_id.into(), activation),
            None => activations.remove(gpu_id),
        };
        let document = SwitchingDocument {
            schema_version: SCHEMA_VERSION,
            boot_id: self.boot_id.clone(),
            activations: activations.clone(),
        };
        write_atomically(&self.path, &serde_json::to_vec_pretty(&document).unwrap())
            .map_err(|error| error.to_string())
    }
}

fn load(path: &Path, boot_id: &str) -> Result<Activations, String> {
    if boot_id.is_empty() {
        return Err("The kernel boot id is unavailable.".into());
    }
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Activations::new()),
        Err(error) => return Err(error.to_string()),
    };
    // Read the lifetime before the payload: old schemas or damaged activations from a
    // positively identified previous boot must not become this boot's recovery state.
    let envelope: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if envelope
        .get("bootId")
        .and_then(|id| id.as_str())
        .is_some_and(|id| !id.is_empty() && id != boot_id)
    {
        return Ok(Activations::new());
    }
    let document: SwitchingDocument =
        serde_json::from_value(envelope).map_err(|error| error.to_string())?;
    if document.schema_version != SCHEMA_VERSION {
        return Err("It has an unsupported schema version.".into());
    }
    if document.boot_id != boot_id {
        return Err("The boot state's boot id is unavailable.".into());
    }
    Ok(document.activations)
}

/// The switching engine, shared by the scan thread and the GPU queues it reconciles on.
#[derive(Clone)]
pub(super) struct Switching {
    store: Arc<TuningStore>,
    cooling: Arc<CoolingController>,
    recognition: Arc<Mutex<Recognition>>,
    users: Arc<dyn UserDirectory>,
    state: Arc<SwitchingState>,
    clock: Arc<dyn Clock>,
    grace_until_ms: u64,
}

impl Switching {
    pub fn open(
        boot_dir: &Path,
        boot_id: &str,
        store: Arc<TuningStore>,
        cooling: Arc<CoolingController>,
        recognition: Arc<Mutex<Recognition>>,
        users: Arc<dyn UserDirectory>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let path = boot_dir.join(SWITCHING_STATE_FILE_NAME);
        let activations = load(&path, boot_id).map_err(|error| {
            eprintln!(
                "Automatic switching is blocked because {} can't be read: {error}",
                path.display()
            );
            error
        });
        let recovery_gpus = if activations.is_err() {
            fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                .and_then(|document| {
                    document
                        .get("activations")?
                        .as_object()
                        .map(|gpus| gpus.keys().cloned().collect())
                })
                .unwrap_or_else(|| {
                    // If even the keys are unreadable, conservatively mark GPUs known to the
                    // tuning store. Newly discovered devices inherit no recovery state.
                    store
                        .read(|document| {
                            document
                                .users
                                .iter()
                                .flat_map(|user| user.profiles.iter())
                                .map(|profile| profile.gpu_id.clone())
                                .chain(document.current_profiles.keys().cloned())
                                .collect()
                        })
                        .unwrap_or_default()
                })
        } else {
            BTreeSet::new()
        };
        Self {
            grace_until_ms: clock.now_monotonic_ms().saturating_add(5_000),
            clock,
            store,
            cooling,
            recognition,
            users,
            state: Arc::new(SwitchingState {
                path,
                boot_id: boot_id.into(),
                activations: Mutex::new(activations),
                recovery_gpus,
                queued: Mutex::default(),
                publishing: Mutex::default(),
                published: Mutex::default(),
            }),
        }
    }

    /// GPUs whose winning game differs from the one automation last put into effect, each claimed
    /// for one queued reconciliation. Recognition uncertainty and edit holds defer writes, but
    /// still let us remember GPU absence so its return can reconcile later.
    pub fn claim_unsettled(&self, backend: &dyn DaemonBackend) -> Vec<String> {
        let running = self.running().ok();
        let Ok(activations) = self.state.activations() else {
            return vec![];
        };
        let Ok(wanted) = self.store.read(|document| {
            running
                .as_ref()
                .map(|running| winning_games(document, running, self.users.as_ref()))
        }) else {
            return vec![];
        };
        let mut queued = self.state.queued.lock().unwrap();
        let readings = backend.performance();
        let held = self.store.held_gpus();
        let in_grace = self.clock.now_monotonic_ms() < self.grace_until_ms;
        wanted
            .iter()
            .flat_map(|games| games.keys())
            .chain(activations.keys())
            .filter(|gpu_id| {
                let activation = activations.get(*gpu_id);
                let absent = verified_gpu(&readings, gpu_id).is_err();
                let presence_changed = activation.is_some_and(|a| a.gpu_absent != absent);
                (presence_changed && absent)
                    || (!in_grace
                        && !held.contains(*gpu_id)
                        && wanted.as_ref().is_some_and(|games| {
                            presence_changed
                                || games.get(*gpu_id) != activation.and_then(|a| a.driving.as_ref())
                        }))
            })
            .filter(|gpu_id| queued.insert((*gpu_id).clone()))
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    fn running(&self) -> Result<GamesByUser, String> {
        self.recognition.lock().unwrap().running().clone()
    }

    /// Applies the GPU's winning game's profile, or restores the GPU when no game wins,
    /// from the latest scan and saved state. Runs on the GPU's queue.
    pub fn reconcile(&self, backend: &dyn DaemonBackend, gpu_id: &str) {
        self.state.queued.lock().unwrap().remove(gpu_id);

        let Ok(document) = self.store.read(Clone::clone) else {
            return;
        };
        let Ok(mut activations) = self.state.activations() else {
            return;
        };
        let activation = activations.remove(gpu_id);
        // Absence is temporary within a boot. Keep the last driving game and its baseline
        // until this UUID can be reconciled; a device in its old slot owns none of that state.
        if verified_gpu(&backend.performance(), gpu_id).is_err() {
            if let Some(mut activation) = activation.filter(|a| !a.gpu_absent) {
                activation.gpu_absent = true;
                self.record(gpu_id, Some(activation));
            }
            return;
        }
        // Presence bookkeeping above must continue while profile replacement is deferred.
        if self.clock.now_monotonic_ms() < self.grace_until_ms
            || self.store.held_gpus().contains(gpu_id)
        {
            return;
        }
        let Ok(running) = self.running() else {
            return;
        };
        let wanted = winning_games(&document, &running, self.users.as_ref()).remove(gpu_id);
        let returning = activation.as_ref().is_some_and(|a| a.gpu_absent);
        let same_game = wanted.as_ref() == activation.as_ref().and_then(|a| a.driving.as_ref());
        if same_game && !returning {
            return;
        }
        if returning {
            self.cooling.resume_interrupted(backend, gpu_id);
            if same_game && activation.as_ref().is_some_and(|a| a.failure.is_some()) {
                let mut activation = activation.unwrap();
                activation.gpu_absent = false;
                self.record(gpu_id, Some(activation));
                return;
            }
        }
        match (wanted, activation) {
            (Some(game), activation) => {
                let activation = activation.unwrap_or_else(|| Activation {
                    previous_profile: document.current_profiles.get(gpu_id).cloned(),
                    ..Activation::default()
                });
                self.apply(backend, &document, gpu_id, game, activation)
            }
            (None, Some(activation)) => self.restore(backend, gpu_id, activation),
            (None, None) => {}
        }
    }

    fn apply(
        &self,
        backend: &dyn DaemonBackend,
        document: &TuningDocument,
        gpu_id: &str,
        game: DrivingGame,
        mut activation: Activation,
    ) {
        let profile = if activation.gpu_absent && activation.driving.as_ref() == Some(&game) {
            let Some(settings) = activation.applied_settings.clone() else {
                activation.gpu_absent = false;
                return self.abandon(gpu_id, activation, &[],
                    "The previously applied game settings can't be read. The returning GPU was left unchanged.".into());
            };
            settings
        } else {
            document
                .any_profile(&game.profile_id)
                .expect("only an existing profile wins")
                .settings
                .clone()
        };
        let readings = backend.performance();
        let Ok(gpu) = verified_gpu(&readings, gpu_id) else {
            return;
        };
        activation.driving = Some(game.clone());
        activation.failure = None;
        let returning_gpu = activation.gpu_absent;
        activation.gpu_absent = false;
        activation.applied_settings = None;
        // Each setting's pre-game value is captured before automation first writes it, and
        // recorded before any write.
        let mut captured = vec![];
        for key in ProfileSettingKey::ALL {
            if profile.get(key).is_none() || activation.baseline.contains_key(&key) {
                continue;
            }
            let reading = reading(gpu, key);
            let Some(current) = reading.current else {
                let failure = format!(
                    "{}: the pre-game value can't be read, so nothing was applied.",
                    reading.label
                );
                return self.abandon(gpu_id, activation, &captured, failure);
            };
            activation.baseline.insert(key, current);
            captured.push(key);
        }
        let captured_cooling = profile.cooling.is_some() && activation.cooling_baseline.is_none();
        if captured_cooling {
            let mut gpu = gpu.clone();
            gpu.cooling.control = Some(self.cooling.control(gpu_id));
            match capture_cooling(&gpu) {
                Ok(Some(configuration)) => {
                    activation.cooling_baseline = Some(configuration);
                    activation.cooling_baseline_fans =
                        gpu.cooling.fans.iter().map(|fan| fan.index).collect();
                }
                _ => return self.abandon(
                    gpu_id,
                    activation,
                    &captured,
                    "Cooling: the pre-game configuration can't be read, so nothing was applied."
                        .into(),
                ),
            }
        }
        if let Err(error) = self.state.set(gpu_id, Some(activation.clone())) {
            if captured_cooling {
                activation.cooling_baseline = None;
            }
            let failure = format!(
                "The pre-game values couldn't be recorded, so nothing was applied: {error}"
            );
            return self.abandon(gpu_id, activation, &captured, failure);
        }
        // Settings an earlier game profile wrote that this one omits return to their pre-game
        // values.
        let mut settings = profile.clone();
        let returning_cooling = profile.cooling.is_none() && activation.cooling_baseline.is_some();
        if returning_cooling {
            settings.cooling = activation.cooling_baseline.clone();
        }
        let returning: Vec<_> = activation
            .baseline
            .iter()
            .filter(|(key, _)| profile.get(**key).is_none())
            .map(|(key, value)| (*key, *value))
            .collect();
        for (key, value) in &returning {
            settings.set(*key, ProfileSetting::Value { value: *value });
        }
        let outcomes = write_settings(
            backend,
            &self.cooling,
            gpu_id,
            settings,
            (returning_cooling || (returning_gpu && profile.cooling.is_some()))
                .then_some(&activation.cooling_baseline_fans),
        );
        for (key, outcome) in &outcomes.numeric {
            // A setting that was never attempted keeps no pre-game value, so it isn't restored.
            let unwritten =
                captured.contains(key) && outcome.status == TuningOutcomeStatus::Unattempted;
            let returned =
                returning.iter().any(|(returned, _)| returned == key) && succeeded(outcome);
            if unwritten || returned {
                activation.baseline.remove(key);
            }
        }
        if captured_cooling
            && outcomes
                .cooling
                .as_ref()
                .is_some_and(|outcome| outcome.status == TuningOutcomeStatus::Unattempted)
        {
            activation.cooling_baseline = None;
        }
        if returning_cooling && outcomes.cooling.as_ref().is_some_and(succeeded) {
            activation.cooling_baseline = None;
        }
        let full = outcomes.iter().all(succeeded);
        if full {
            let mut applied = profile.clone();
            if applied.cooling.is_some() {
                applied.cooling = self.cooling.control(gpu_id).applied;
            }
            activation.applied_settings = Some(applied);
        }
        if !full {
            activation.failure = Some(failure_message(&outcomes));
        }
        // A fully applied game profile becomes current; a partly applied one leaves no
        // current profile. Neither is recorded as last applied tuning values.
        if outcomes.iter().any(succeeded) {
            self.set_current_profile(gpu_id, full.then_some(game.profile_id));
        }
        self.record(gpu_id, Some(activation));
    }

    fn restore(&self, backend: &dyn DaemonBackend, gpu_id: &str, mut activation: Activation) {
        activation.gpu_absent = false;
        activation.driving = None;
        activation.failure = None;
        if !activation.baseline.is_empty() || activation.cooling_baseline.is_some() {
            let readings = backend.performance();
            if verified_gpu(&readings, gpu_id).is_err() {
                // It may have disappeared since reconciliation's first reading. The saved
                // activation still holds its driving game, so its return will be retried.
                return;
            }
            let mut settings = ProfileSettings {
                cooling: activation.cooling_baseline.clone(),
                ..ProfileSettings::default()
            };
            for (key, value) in &activation.baseline {
                settings.set(*key, ProfileSetting::Value { value: *value });
            }
            let outcomes = write_settings(
                backend,
                &self.cooling,
                gpu_id,
                settings,
                activation
                    .cooling_baseline
                    .as_ref()
                    .map(|_| &activation.cooling_baseline_fans),
            );
            for (key, outcome) in &outcomes.numeric {
                if succeeded(outcome) {
                    activation.baseline.remove(key);
                }
            }
            if outcomes.cooling.as_ref().is_some_and(succeeded) {
                activation.cooling_baseline = None;
            }
            if !activation.baseline.is_empty() || activation.cooling_baseline.is_some() {
                // Partly restored, no profile describes the GPU.
                if outcomes.iter().any(succeeded) {
                    self.set_current_profile(gpu_id, None);
                }
                return self.abandon(gpu_id, activation, &[], failure_message(&outcomes));
            }
        }
        // Fully restored, the previous current profile returns, if it still exists.
        let previous = activation.previous_profile.filter(|id| {
            self.store
                .read(|document| document.any_profile(id).is_some())
                .unwrap_or(false)
        });
        self.set_current_profile(gpu_id, previous);
        self.record(gpu_id, None);
    }

    /// Records why the operation stopped, without the pre-game values it captured but never
    /// wrote.
    fn abandon(
        &self,
        gpu_id: &str,
        mut activation: Activation,
        captured: &[ProfileSettingKey],
        failure: String,
    ) {
        for key in captured {
            activation.baseline.remove(key);
        }
        activation.failure = Some(failure);
        self.record(gpu_id, Some(activation));
    }

    fn set_current_profile(&self, gpu_id: &str, profile_id: Option<String>) {
        if let Err(error) = self.store.update(|document| {
            match profile_id {
                Some(id) => document.current_profiles.insert(gpu_id.into(), id),
                None => document.current_profiles.remove(gpu_id),
            };
        }) {
            eprintln!("failed to record the current profile of {gpu_id}: {error}");
        }
    }

    /// Writes the GPU's activation to boot-scoped state.
    fn record(&self, gpu_id: &str, activation: Option<Activation>) {
        if let Err(error) = self.state.set(gpu_id, activation) {
            eprintln!("failed to record automatic switching state for {gpu_id}: {error}");
        }
    }

    /// The caller's view of automatic switching.
    pub fn status_for(&self, peer: &SessionPeer) -> AutomaticSwitchingStatus {
        self.snapshot().status_for(peer)
    }

    /// Records the status a newly opened session reads, so that only later changes are pushed.
    pub fn opened(&self, peer: &SessionPeer) {
        let _publishing = self.state.publishing.lock().unwrap();
        let status = self.status_for(peer);
        // Another of the user's sessions may not have been pushed a newer status yet; the next
        // push then reaches every session.
        self.state
            .published
            .lock()
            .unwrap()
            .entry((peer.uid, peer.username.clone()))
            .or_insert(status);
    }

    /// Pushes each user's status to their sessions when it changed.
    pub fn publish(&self) {
        let _publishing = self.state.publishing.lock().unwrap();
        let snapshot = self.snapshot();
        // Compare with the previous push, not with another session visited in this push.
        let previous = self.state.published.lock().unwrap().clone();
        self.store.publish_for_users(|peer| {
            let status = snapshot.status_for(peer);
            let mut published = self.state.published.lock().unwrap();
            let user = (peer.uid, peer.username.clone());
            if previous.get(&user) == Some(&status) {
                return None;
            }
            published.insert(user, status.clone());
            Some(DaemonEvent::AutomaticSwitchingChanged(status))
        });
    }

    fn snapshot(&self) -> Snapshot {
        let wanted = self
            .running()
            .ok()
            .and_then(|running| {
                self.store
                    .read(|document| winning_games(document, &running, self.users.as_ref()))
                    .ok()
            })
            .unwrap_or_default();
        Snapshot {
            held: self.store.held_gpus(),
            wanted,
            recognition: self.running().err(),
            activations: self.state.activations(),
            recovery_gpus: self.state.recovery_gpus.clone(),
            document: self.store.read(Clone::clone).ok(),
        }
    }
}

/// Each GPU's winning game: of the running games linked to it by users who opted in, the most
/// recently started, by its earliest process's start time, with ties to the lower pid. A uid's
/// games belong only to the account that has that uid now, so a renamed account's earlier state
/// stays orphaned.
fn winning_games(
    document: &TuningDocument,
    running: &GamesByUser,
    accounts: &dyn UserDirectory,
) -> BTreeMap<String, DrivingGame> {
    let mut wanted: BTreeMap<String, ((u64, Reverse<u32>), DrivingGame)> = BTreeMap::new();
    let mut usernames = HashMap::new();
    let users = document
        .users
        .iter()
        .filter(|user| user.automatic_switching);
    for user in users {
        let Some(games) = running.get(&user.uid) else {
            continue;
        };
        let username = usernames
            .entry(user.uid)
            .or_insert_with(|| accounts.username(user.uid));
        if username.as_deref() != Some(user.username.as_str()) {
            continue;
        }
        for game in games {
            let Some(association) = user.games.iter().find(|linked| linked.game == game.game)
            else {
                continue;
            };
            for (gpu_id, profile_id) in &association.profiles {
                if !user
                    .profiles
                    .iter()
                    .any(|profile| profile.id == *profile_id && profile.gpu_id == *gpu_id)
                {
                    continue;
                }
                let rank = (game.start_time_ticks, Reverse(game.pid));
                if wanted.get(gpu_id).is_some_and(|(best, _)| *best >= rank) {
                    continue;
                }
                let winner = DrivingGame {
                    uid: user.uid,
                    username: user.username.clone(),
                    game: game.game.clone(),
                    profile_id: profile_id.clone(),
                };
                wanted.insert(gpu_id.clone(), (rank, winner));
            }
        }
    }
    wanted
        .into_iter()
        .map(|(gpu_id, (_, winner))| (gpu_id, winner))
        .collect()
}

/// Validates every setting and writes them all or none, never clamping, as startup does. Returns
/// each included setting's outcome.
fn write_settings(
    backend: &dyn DaemonBackend,
    cooling: &CoolingController,
    gpu_id: &str,
    settings: ProfileSettings,
    expected_fans: Option<&BTreeSet<u32>>,
) -> SettingOutcomes {
    // A restart or game exit is not an explicit cooling retry. In particular, restoring a
    // healthy pre-game curve must not replace the automatic fallback of a faulted game curve.
    if settings.cooling.is_some() && cooling.control(gpu_id).suspension.is_some() {
        return SettingOutcomes::unattempted(&settings,
            "Cooling is suspended. Review and explicitly retry cooling before automatic switching can replace it.");
    }
    if let Some(expected) = expected_fans {
        let readings = backend.performance();
        let reported = verified_gpu(&readings, gpu_id).ok().map(|gpu| {
            gpu.cooling
                .fans
                .iter()
                .map(|fan| fan.index)
                .collect::<BTreeSet<_>>()
        });
        if expected.is_empty() || reported.as_ref() != Some(expected) {
            return SettingOutcomes::unattempted(&settings,
                "Cooling: the pre-game fan identities are unavailable or the GPU's fans changed. Nothing was restored.");
        }
    }
    let application = apply_unreviewed_profile(
        backend,
        Some(cooling),
        ApplyGpuProfileRequest {
            gpu_id: gpu_id.into(),
            settings,
        },
    );
    let numeric = [
        (ProfileSettingKey::PowerLimitWatts, application.power),
        (ProfileSettingKey::GraphicsOffsetMhz, application.graphics),
        (ProfileSettingKey::MemoryOffsetMhz, application.memory),
    ]
    .into_iter()
    .filter_map(|(key, outcome)| Some((key, outcome?)))
    .collect();
    SettingOutcomes {
        numeric,
        cooling: application.cooling.map(|application| application.outcome),
    }
}

struct SettingOutcomes {
    numeric: Vec<(ProfileSettingKey, TuningOutcome)>,
    cooling: Option<TuningOutcome>,
}

impl SettingOutcomes {
    fn unattempted(settings: &ProfileSettings, message: &str) -> Self {
        let outcome = || TuningOutcome {
            status: TuningOutcomeStatus::Unattempted,
            error: Some(message.into()),
        };
        Self {
            numeric: ProfileSettingKey::ALL
                .into_iter()
                .filter(|key| settings.get(*key).is_some())
                .map(|key| (key, outcome()))
                .collect(),
            cooling: settings.cooling.as_ref().map(|_| outcome()),
        }
    }

    fn iter(&self) -> impl Iterator<Item = &TuningOutcome> {
        self.numeric
            .iter()
            .map(|(_, outcome)| outcome)
            .chain(self.cooling.iter())
    }
}

fn succeeded(outcome: &TuningOutcome) -> bool {
    outcome.status == TuningOutcomeStatus::Succeeded
}

fn failure_message(outcomes: &SettingOutcomes) -> String {
    outcomes
        .iter()
        .filter_map(|outcome| outcome.error.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join("\n")
}

/// What every user's status is computed from, read once so that no lock is held while pushing.
struct Snapshot {
    held: BTreeSet<String>,
    wanted: BTreeMap<String, DrivingGame>,
    /// Why recognition is unavailable, if it is.
    recognition: Option<String>,
    activations: Result<Activations, String>,
    recovery_gpus: BTreeSet<String>,
    /// None while the tuning store is unhealthy.
    document: Option<TuningDocument>,
}

impl Snapshot {
    fn status_for(&self, peer: &SessionPeer) -> AutomaticSwitchingStatus {
        let blocked = |reason: String| AutomaticSwitchingStatus {
            condition: SwitchingCondition::Blocked { reason },
            gpus: vec![],
        };
        let activations = match &self.activations {
            Ok(activations) => activations,
            Err(error) => {
                let mut status = blocked(format!(
                    "Automatic switching is suspended because Prism's boot state can't be used: \
                     {error}"
                ));
                status.gpus = self.recovery_gpus.iter().map(|gpu_id| GpuSwitchingStatus {
                    gpu_id: gpu_id.clone(),
                    state: GpuSwitchingState::RecoveryRequired {
                        message: "The pre-game tuning values can't be read. The GPU has been left unchanged.".into(),
                    },
                }).collect();
                return status;
            }
        };
        let Some(document) = &self.document else {
            return blocked(
                "Automatic switching is suspended because Prism's tuning store can't be used."
                    .into(),
            );
        };
        let user = document.user(peer.uid, &peer.username);
        let own = |game: &DrivingGame| game.uid == peer.uid && game.username == peer.username;
        let mut gpus: Vec<_> = activations
            .iter()
            .filter_map(|(gpu_id, activation)| {
                let state = match (&activation.failure, &activation.driving) {
                    (Some(message), _) => GpuSwitchingState::Failed {
                        message: message.clone(),
                    },
                    (None, Some(driving)) if own(driving) => {
                        let profile = document.any_profile(&driving.profile_id)?;
                        GpuSwitchingState::Active {
                            game: driving.game.clone(),
                            game_name: user
                                .and_then(|user| {
                                    user.games.iter().find(|linked| linked.game == driving.game)
                                })
                                .map(|linked| linked.name.clone())
                                .unwrap_or_else(|| match &driving.game {
                                    GameId::Steam { app_id } => format!("Steam app {app_id}"),
                                    GameId::Program { path } => path.clone(),
                                }),
                            profile: CurrentProfile {
                                id: profile.id.clone(),
                                name: profile.name.clone(),
                            },
                        }
                    }
                    (None, Some(_)) => GpuSwitchingState::AnotherUser {},
                    (None, None) => return None,
                };
                Some(GpuSwitchingStatus {
                    gpu_id: gpu_id.clone(),
                    state,
                })
            })
            .collect();
        for (gpu_id, driving) in &self.wanted {
            if !self.held.contains(gpu_id)
                || activations.get(gpu_id).and_then(|a| a.driving.as_ref()) == Some(driving)
            {
                continue;
            }
            let state = if own(driving) {
                let Some(profile) = document.any_profile(&driving.profile_id) else {
                    continue;
                };
                GpuSwitchingState::WaitingForEdits {
                    profile: CurrentProfile {
                        id: profile.id.clone(),
                        name: profile.name.clone(),
                    },
                }
            } else {
                GpuSwitchingState::AnotherUser {}
            };
            gpus.retain(|gpu| &gpu.gpu_id != gpu_id);
            gpus.push(GpuSwitchingStatus {
                gpu_id: gpu_id.clone(),
                state,
            });
        }
        gpus.sort_by(|a, b| a.gpu_id.cmp(&b.gpu_id));
        let opted_in = user.is_some_and(|user| user.automatic_switching);
        let holding = activations
            .values()
            .any(|activation| activation.driving.as_ref().is_some_and(own));
        let condition = match &self.recognition {
            Some(reason) if opted_in || holding => SwitchingCondition::RecognitionUnavailable {
                reason: reason.clone(),
            },
            _ if opted_in => SwitchingCondition::Ready {},
            _ => SwitchingCondition::Off {},
        };
        AutomaticSwitchingStatus { condition, gpus }
    }
}
