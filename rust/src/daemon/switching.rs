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
use super::profiles::{reading, verified_gpu};
use super::recognition::{GamesByUser, Recognition};
use super::session::SessionPeer;
use super::tuning::apply_startup_profile;
use super::tuning_store::{write_atomically, TuningDocument, TuningStore};
use crate::protocol::{
    ApplyGpuProfileRequest, AutomaticSwitchingStatus, CurrentProfile, DaemonEvent, GameId,
    GpuSwitchingState, GpuSwitchingStatus, ProfileApplicationResponse, ProfileSetting,
    ProfileSettingKey, ProfileSettings, SwitchingCondition, TuningOutcome, TuningOutcomeStatus,
};

pub(super) const SWITCHING_STATE_FILE_NAME: &str = "switching-state.json";
const SCHEMA_VERSION: u64 = 1;

/// The user's running game, and its linked profile, that drive a GPU.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Driver {
    uid: u32,
    username: String,
    game: GameId,
    profile_id: String,
}

/// Automation's hold on one GPU, from its first automatic write until full restoration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Activation {
    /// What automation last put, or tried to put, into effect; none once it restores.
    driver: Option<Driver>,
    /// The pre-game tuning values: the actual value of each setting automation wrote, captured
    /// when it first wrote that setting.
    baseline: BTreeMap<ProfileSettingKey, i64>,
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
    let document: SwitchingDocument =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if document.schema_version != SCHEMA_VERSION {
        return Err("It has an unsupported schema version.".into());
    }
    // A previous boot's pre-game values are never a restoration target.
    if document.boot_id != boot_id {
        return Ok(Activations::new());
    }
    Ok(document.activations)
}

/// The switching engine, shared by the scan thread and the GPU queues it reconciles on.
#[derive(Clone)]
pub(super) struct Switching {
    store: Arc<TuningStore>,
    recognition: Arc<Mutex<Recognition>>,
    state: Arc<SwitchingState>,
}

impl Switching {
    pub fn open(
        boot_dir: &Path,
        boot_id: &str,
        store: Arc<TuningStore>,
        recognition: Arc<Mutex<Recognition>>,
    ) -> Self {
        let path = boot_dir.join(SWITCHING_STATE_FILE_NAME);
        let activations = load(&path, boot_id).map_err(|error| {
            eprintln!(
                "Automatic switching is blocked because {} can't be read: {error}",
                path.display()
            );
            error
        });
        Self {
            store,
            recognition,
            state: Arc::new(SwitchingState {
                path,
                boot_id: boot_id.into(),
                activations: Mutex::new(activations),
                queued: Mutex::default(),
                publishing: Mutex::default(),
                published: Mutex::default(),
            }),
        }
    }

    /// GPUs whose wanted driver differs from what automation last put into effect, each claimed
    /// for one queued reconciliation. Nothing changes while recognition is unavailable or the
    /// tuning store is unhealthy.
    pub fn claim_unsettled(&self) -> Vec<String> {
        let Ok(running) = self.running() else {
            return vec![];
        };
        let Ok(activations) = self.state.activations() else {
            return vec![];
        };
        let Ok(wanted) = self
            .store
            .read(|document| wanted_drivers(document, &running))
        else {
            return vec![];
        };
        let mut queued = self.state.queued.lock().unwrap();
        wanted
            .keys()
            .chain(activations.keys())
            .filter(|gpu_id| {
                wanted.get(*gpu_id)
                    != activations
                        .get(*gpu_id)
                        .and_then(|activation| activation.driver.as_ref())
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

    /// Applies the GPU's wanted driver's profile, or restores the GPU when nothing drives it,
    /// from the latest scan and saved state. Runs on the GPU's queue.
    pub fn reconcile(&self, backend: &dyn DaemonBackend, gpu_id: &str) {
        self.state.queued.lock().unwrap().remove(gpu_id);
        let Ok(running) = self.running() else {
            return;
        };
        let Ok(document) = self.store.read(Clone::clone) else {
            return;
        };
        let Ok(mut activations) = self.state.activations() else {
            return;
        };
        let activation = activations.remove(gpu_id);
        let wanted = wanted_drivers(&document, &running).remove(gpu_id);
        if wanted.as_ref() == activation.as_ref().and_then(|a| a.driver.as_ref()) {
            return;
        }
        match (wanted, activation) {
            (Some(driver), activation) => {
                let activation = activation.unwrap_or_else(|| Activation {
                    previous_profile: document.current_profiles.get(gpu_id).cloned(),
                    ..Activation::default()
                });
                self.apply(backend, &document, gpu_id, driver, activation)
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
        driver: Driver,
        mut activation: Activation,
    ) {
        let settings = document
            .any_profile(&driver.profile_id)
            .expect("only an existing profile is wanted")
            .settings
            .clone();
        activation.driver = Some(driver.clone());
        activation.failure = None;
        let readings = backend.performance();
        let Ok(gpu) = verified_gpu(&readings, gpu_id) else {
            activation.failure = Some(
                "The GPU is unavailable or its identity can't be verified, so nothing was applied."
                    .into(),
            );
            self.keep(gpu_id, Some(activation));
            return;
        };
        // Each setting's pre-game value is captured before automation first writes it, and
        // recorded before any write.
        let mut captured = vec![];
        for key in ProfileSettingKey::ALL {
            if settings.get(key).is_none() || activation.baseline.contains_key(&key) {
                continue;
            }
            let reading = reading(gpu, key);
            let Some(current) = reading.current else {
                for key in &captured {
                    activation.baseline.remove(key);
                }
                activation.failure = Some(format!(
                    "{}: the pre-game value can't be read, so nothing was applied.",
                    reading.label
                ));
                self.keep(gpu_id, Some(activation));
                return;
            };
            activation.baseline.insert(key, current);
            captured.push(key);
        }
        if let Err(error) = self.state.set(gpu_id, Some(activation.clone())) {
            for key in &captured {
                activation.baseline.remove(key);
            }
            activation.failure = Some(format!(
                "The pre-game values couldn't be recorded, so nothing was applied: {error}"
            ));
            self.keep(gpu_id, Some(activation));
            return;
        }
        let application = apply_startup_profile(
            backend,
            ApplyGpuProfileRequest {
                gpu_id: gpu_id.into(),
                settings,
            },
        );
        let outcomes = outcomes(&application);
        // A setting that was never attempted keeps no pre-game value, so it isn't restored.
        for key in captured {
            if outcomes
                .iter()
                .any(|(attempted, outcome)| *attempted == key && unattempted(outcome))
            {
                activation.baseline.remove(&key);
            }
        }
        let full = outcomes.iter().all(|(_, outcome)| succeeded(outcome));
        let any = outcomes.iter().any(|(_, outcome)| succeeded(outcome));
        if !full {
            activation.failure = Some(failure_message(&outcomes));
        }
        // A fully applied game profile becomes current; a partly applied one leaves no
        // current profile. Neither is recorded as last applied tuning values.
        if any {
            if let Err(error) = self.store.update(|document| {
                if full {
                    document
                        .current_profiles
                        .insert(gpu_id.into(), driver.profile_id.clone());
                } else {
                    document.current_profiles.remove(gpu_id);
                }
            }) {
                eprintln!("failed to record the current profile of {gpu_id}: {error}");
            }
        }
        self.keep(gpu_id, Some(activation));
    }

    fn restore(&self, backend: &dyn DaemonBackend, gpu_id: &str, mut activation: Activation) {
        activation.driver = None;
        activation.failure = None;
        if !activation.baseline.is_empty() {
            let readings = backend.performance();
            if verified_gpu(&readings, gpu_id).is_err() {
                activation.failure = Some(
                    "The GPU is unavailable or its identity can't be verified, so its pre-game \
                     values weren't restored."
                        .into(),
                );
                self.keep(gpu_id, Some(activation));
                return;
            }
            let mut settings = ProfileSettings::default();
            for (key, value) in &activation.baseline {
                settings.set(*key, ProfileSetting::Value { value: *value });
            }
            let application = apply_startup_profile(
                backend,
                ApplyGpuProfileRequest {
                    gpu_id: gpu_id.into(),
                    settings,
                },
            );
            let outcomes = outcomes(&application);
            for (key, outcome) in &outcomes {
                if succeeded(outcome) {
                    activation.baseline.remove(key);
                }
            }
            if !activation.baseline.is_empty() {
                activation.failure = Some(failure_message(&outcomes));
                self.keep(gpu_id, Some(activation));
                return;
            }
        }
        // Fully restored, the previous current profile returns, if it still exists.
        if let Err(error) = self.store.update(|document| {
            match activation
                .previous_profile
                .filter(|id| document.any_profile(id).is_some())
            {
                Some(id) => document.current_profiles.insert(gpu_id.into(), id),
                None => document.current_profiles.remove(gpu_id),
            };
        }) {
            eprintln!("failed to restore the current profile of {gpu_id}: {error}");
        }
        self.keep(gpu_id, None);
    }

    fn keep(&self, gpu_id: &str, activation: Option<Activation>) {
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
        self.state
            .published
            .lock()
            .unwrap()
            .insert((peer.uid, peer.username.clone()), status);
    }

    /// Pushes each user's status to their sessions when it changed.
    pub fn publish(&self) {
        let _publishing = self.state.publishing.lock().unwrap();
        let snapshot = self.snapshot();
        self.store.publish_for_users(|peer| {
            let status = snapshot.status_for(peer);
            let mut published = self.state.published.lock().unwrap();
            let user = (peer.uid, peer.username.clone());
            if published.get(&user) == Some(&status) {
                return None;
            }
            published.insert(user, status.clone());
            Some(DaemonEvent::AutomaticSwitchingChanged(status))
        });
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            recognition: self.running().err(),
            activations: self.state.activations(),
            document: self.store.read(Clone::clone).ok(),
        }
    }
}

/// Each GPU's driver: of the running games linked to it by users who opted in, the most
/// recently started, by its earliest process's start time, with ties to the lower pid.
fn wanted_drivers(document: &TuningDocument, running: &GamesByUser) -> BTreeMap<String, Driver> {
    let mut wanted: BTreeMap<String, ((u64, Reverse<u32>), Driver)> = BTreeMap::new();
    let users = document
        .users
        .iter()
        .filter(|user| user.automatic_switching);
    for user in users {
        for game in running.get(&user.uid).into_iter().flatten() {
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
                let driver = Driver {
                    uid: user.uid,
                    username: user.username.clone(),
                    game: game.game.clone(),
                    profile_id: profile_id.clone(),
                };
                wanted.insert(gpu_id.clone(), (rank, driver));
            }
        }
    }
    wanted
        .into_iter()
        .map(|(gpu_id, (_, driver))| (gpu_id, driver))
        .collect()
}

fn outcomes(application: &ProfileApplicationResponse) -> Vec<(ProfileSettingKey, &TuningOutcome)> {
    [
        (ProfileSettingKey::PowerLimitWatts, &application.power),
        (ProfileSettingKey::GraphicsOffsetMhz, &application.graphics),
        (ProfileSettingKey::MemoryOffsetMhz, &application.memory),
    ]
    .into_iter()
    .filter_map(|(key, outcome)| Some((key, outcome.as_ref()?)))
    .collect()
}

fn succeeded(outcome: &TuningOutcome) -> bool {
    outcome.status == TuningOutcomeStatus::Succeeded
}

fn unattempted(outcome: &TuningOutcome) -> bool {
    outcome.status == TuningOutcomeStatus::Unattempted
}

fn failure_message(outcomes: &[(ProfileSettingKey, &TuningOutcome)]) -> String {
    outcomes
        .iter()
        .filter_map(|(_, outcome)| outcome.error.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join("\n")
}

/// What every user's status is computed from, read once so that no lock is held while pushing.
struct Snapshot {
    /// Why recognition is unavailable, if it is.
    recognition: Option<String>,
    activations: Result<Activations, String>,
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
                return blocked(format!(
                    "Automatic switching is suspended because Prism's boot state can't be used: \
                     {error}"
                ))
            }
        };
        let Some(document) = &self.document else {
            return blocked(
                "Automatic switching is suspended because Prism's tuning store can't be used."
                    .into(),
            );
        };
        let user = document.user(peer.uid, &peer.username);
        let own = |driver: &Driver| driver.uid == peer.uid && driver.username == peer.username;
        let gpus: Vec<_> = activations
            .iter()
            .filter_map(|(gpu_id, activation)| {
                let state = match (&activation.failure, &activation.driver) {
                    (Some(message), _) => GpuSwitchingState::Failed {
                        message: message.clone(),
                    },
                    (None, Some(driver)) if own(driver) => {
                        let profile = document.any_profile(&driver.profile_id)?;
                        GpuSwitchingState::Active {
                            game: driver.game.clone(),
                            game_name: user
                                .and_then(|user| {
                                    user.games.iter().find(|linked| linked.game == driver.game)
                                })
                                .map(|linked| linked.name.clone())
                                .unwrap_or_else(|| {
                                    let GameId::Steam { app_id } = driver.game;
                                    format!("Steam app {app_id}")
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
        let opted_in = user.is_some_and(|user| user.automatic_switching);
        let holding = activations
            .values()
            .any(|activation| activation.driver.as_ref().is_some_and(own));
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
