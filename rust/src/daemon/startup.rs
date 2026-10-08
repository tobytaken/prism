//! Host-wide startup choices. Designations keep profile ids, so profile edits follow the link.

use super::session::SessionPeer;
use super::tuning_store::TuningDocument;
use crate::gpu::{GpuCommandError, GpuCommandErrorCode};
use crate::protocol::{CurrentProfile, SetStartupProfileRequest, StartupProfile, StartupState};

impl TuningDocument {
    pub(super) fn startup_state(&self) -> StartupState {
        StartupState {
            behavior: self.startup_behavior,
            profiles: self
                .startup_profiles
                .iter()
                .filter_map(|(gpu_id, id)| {
                    let profile = self.any_profile(id)?;
                    Some(StartupProfile {
                        gpu_id: gpu_id.clone(),
                        profile: CurrentProfile {
                            id: profile.id.clone(),
                            name: profile.name.clone(),
                        },
                    })
                })
                .collect(),
            outcomes: vec![],
            possibly_stale: vec![],
        }
    }

    pub(super) fn set_startup_profile(
        &mut self,
        peer: &SessionPeer,
        request: SetStartupProfileRequest,
    ) -> Result<(), GpuCommandError> {
        if request.gpu_id.trim().is_empty() {
            return Err(GpuCommandError::new(
                GpuCommandErrorCode::InvalidArgument,
                "Choose a GPU with a stable identity (UUID).",
            ));
        }
        match request.profile_id {
            Some(id) => {
                let profile = self.profile_of(peer, &id)?;
                if profile.gpu_id != request.gpu_id {
                    return Err(GpuCommandError::new(
                        GpuCommandErrorCode::InvalidArgument,
                        "The startup profile belongs to another GPU.",
                    ));
                }
                self.startup_profiles.insert(request.gpu_id, id);
            }
            None => {
                self.startup_profiles.remove(&request.gpu_id);
            }
        }
        Ok(())
    }
}

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::backend::DaemonBackend;
use super::boot_state::STARTUP_IN_PROGRESS;
use super::cooling::CoolingController;
use super::daemon_core::DaemonCore;
use super::host::Clock;
use super::profiles::verified_gpu;
use super::tuning::apply_unreviewed_profile;
use super::tuning_store::{RestoreAttempt, TuningStore};
use crate::protocol::{
    ApplyGpuProfileRequest, DaemonEvent, ProfileSettings, StartupBehavior, StartupOutcome,
    StartupOutcomeStatus, TuningOutcomeStatus,
};

const RESTORE_SKIPPED: &str = "Restoring last applied tuning values was skipped once because the \
    host didn't shut down cleanly after the previous boot restored them. They will be restored \
    again at the next boot.";

/// What startup puts into effect on one GPU.
struct StartupTarget {
    settings: ProfileSettings,
    /// The startup profile, or none when restoring last applied tuning values.
    profile: Option<CurrentProfile>,
}

impl DaemonCore {
    pub(super) fn start_startup(&self) -> Result<(), GpuCommandError> {
        // Reading an unhealthy store fails before the marker or any hardware can be changed.
        let document = self.tuning_store.read(Clone::clone)?;
        let behavior = document.startup_behavior;
        let boot_id = self.dependencies.boot_id.clone();
        // Startup profiles were chosen deliberately for boot, so only a restore is guarded.
        let restore_skipped = behavior == StartupBehavior::RestoreLastApplied
            && document.restore_attempt.is_some()
            && !document.restored_in(&boot_id);
        let restorable = || {
            document
                .last_applied
                .iter()
                .filter(|(_, settings)| !settings.is_empty())
        };
        let designated = |gpu_id: &str| {
            let profile = document.any_profile(document.startup_profiles.get(gpu_id)?)?;
            Some((
                profile.settings.clone(),
                CurrentProfile {
                    id: profile.id.clone(),
                    name: profile.name.clone(),
                },
            ))
        };
        let mut targets: BTreeMap<String, StartupTarget> = match behavior {
            StartupBehavior::Off => BTreeMap::new(),
            StartupBehavior::StartupProfiles => document
                .startup_profiles
                .keys()
                .filter_map(|gpu_id| {
                    let (settings, profile) = designated(gpu_id)?;
                    Some((
                        gpu_id.clone(),
                        StartupTarget {
                            settings,
                            profile: Some(profile),
                        },
                    ))
                })
                .collect(),
            StartupBehavior::RestoreLastApplied if restore_skipped => BTreeMap::new(),
            StartupBehavior::RestoreLastApplied => restorable()
                .map(|(gpu_id, settings)| {
                    (
                        gpu_id.clone(),
                        StartupTarget {
                            settings: settings.clone(),
                            profile: None,
                        },
                    )
                })
                .collect(),
        };
        let performance = self.dependencies.backend.performance();
        let gpu_ids: BTreeSet<_> = performance
            .gpus
            .iter()
            .filter_map(|gpu| gpu.stable_id.clone())
            .filter(|uuid| !uuid.is_empty())
            .chain(document.startup_profiles.keys().cloned())
            .chain(targets.keys().cloned())
            .chain(
                (behavior == StartupBehavior::RestoreLastApplied)
                    .then(|| restorable().map(|(gpu_id, _)| gpu_id.clone()))
                    .into_iter()
                    .flatten(),
            )
            .collect();
        let outcomes: Vec<_> = gpu_ids
            .into_iter()
            .map(|gpu_id| {
                let reason = match behavior {
                    StartupBehavior::Off => "Startup behavior is Off.",
                    _ if targets.contains_key(&gpu_id) => STARTUP_IN_PROGRESS,
                    StartupBehavior::StartupProfiles => "No startup profile.",
                    StartupBehavior::RestoreLastApplied
                        if restore_skipped && restorable().any(|(id, _)| *id == gpu_id) =>
                    {
                        RESTORE_SKIPPED
                    }
                    StartupBehavior::RestoreLastApplied => "No last applied tuning values.",
                };
                StartupOutcome {
                    behavior,
                    profile: (behavior != StartupBehavior::RestoreLastApplied)
                        .then(|| designated(&gpu_id).map(|(_, profile)| profile))
                        .flatten(),
                    gpu_id,
                    status: StartupOutcomeStatus::NotAttempted,
                    reason: Some(reason.into()),
                    application: None,
                }
            })
            .collect();
        if !self
            .boot_state
            .begin(outcomes.clone(), targets.keys().cloned().collect())?
        {
            return Ok(());
        }
        let restoring = behavior == StartupBehavior::RestoreLastApplied && !targets.is_empty();
        let prepare = |document: &mut TuningDocument| {
            // A previous boot's current profile describes this boot only where its last applied
            // values are restored.
            document
                .current_profiles
                .retain(|gpu_id, _| restoring && targets.contains_key(gpu_id));
            // The guard's record is on disk before any restored value is written, and every
            // other boot consumes it.
            document.restore_attempt = restoring.then(|| RestoreAttempt {
                boot_id: boot_id.clone(),
            });
        };
        let mut prepared = document.clone();
        prepare(&mut prepared);
        if prepared != document {
            if let Err(error) = self.tuning_store.update(prepare) {
                for mut outcome in outcomes {
                    if targets.contains_key(&outcome.gpu_id) {
                        outcome.reason = Some(format!("Startup was skipped because Prism's tuning store couldn't be written: {error}"));
                        self.boot_state.record(outcome)?;
                    }
                }
                return Err(error.into());
            }
        }
        let deadline = self
            .dependencies
            .clock
            .now_monotonic_ms()
            .saturating_add(30_000);
        for mut outcome in outcomes {
            let Some(target) = targets.remove(&outcome.gpu_id) else {
                continue;
            };
            let gpu_id = outcome.gpu_id.clone();
            let backend = Arc::clone(&self.dependencies.backend);
            let cooling = Arc::clone(&self.cooling);
            let store = Arc::clone(&self.tuning_store);
            let boot = Arc::clone(&self.boot_state);
            let clock = Arc::clone(&self.dependencies.clock);
            let stopping = Arc::clone(&self.stopping);
            // Reserve every target GPU's queue now. Manual writes wait behind startup; reads and
            // unrelated GPUs remain available while a target GPU is absent.
            let pending = self.queues.submit(&gpu_id.clone(), move || {
                let Some(full) = apply_target(
                    backend.as_ref(),
                    &cooling,
                    &store,
                    clock.as_ref(),
                    &stopping,
                    deadline,
                    &gpu_id,
                    target.settings,
                    &mut outcome,
                ) else {
                    return;
                };
                match (target.profile, full) {
                    (Some(profile), true) => {
                        if let Err(error) = store.update(|document| {
                            document.current_profiles.insert(gpu_id.clone(), profile.id);
                        }) {
                            outcome.status = StartupOutcomeStatus::Failed;
                            outcome.reason = Some(format!("The GPU accepted the settings, but its current profile could not be recorded: {error}"));
                        }
                    }
                    // The previous current profile no longer describes a GPU whose last applied
                    // values weren't restored in full.
                    (None, false) => {
                        if let Err(error) = store.update(|document| {
                            document.current_profiles.remove(&gpu_id);
                        }) {
                            eprintln!("failed to clear the current profile of {gpu_id}: {error}");
                        }
                    }
                    _ => {}
                }
                if let Ok(Some(state)) = store.read(|document| {
                    let readings = backend.performance();
                    verified_gpu(&readings, &gpu_id).ok().map(|gpu| {
                        let mut gpu = gpu.clone();
                        gpu.cooling.control = Some(cooling.control(&gpu_id));
                        document.tuning_state(&gpu)
                    })
                }) {
                    store.publish(DaemonEvent::TuningStateChanged(state));
                }
                if let Err(error) = boot.record(outcome) {
                    eprintln!("{}", error.message);
                }
            });
            self.startup_operations.lock().unwrap().push(pending);
        }
        Ok(())
    }
}

/// Waits about 30 seconds for the GPU, then validates every setting and applies them all or
/// none, never clamping. Returns whether every setting applied, or none when the daemon stopped.
#[allow(clippy::too_many_arguments)]
fn apply_target(
    backend: &dyn DaemonBackend,
    cooling: &CoolingController,
    store: &TuningStore,
    clock: &dyn Clock,
    stopping: &AtomicBool,
    deadline: u64,
    gpu_id: &str,
    settings: ProfileSettings,
    outcome: &mut StartupOutcome,
) -> Option<bool> {
    loop {
        if stopping.load(Ordering::SeqCst) {
            return None;
        }
        let readings = backend.performance();
        if clock.now_monotonic_ms() >= deadline {
            outcome.reason = Some("The GPU was absent at startup after waiting about 30 seconds. Nothing will be applied to it later this boot.".into());
            return Some(false);
        }
        if verified_gpu(&readings, gpu_id).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if store.read(|_| ()).is_err() {
        outcome.reason = Some("Startup was skipped because the tuning store is unhealthy.".into());
        return Some(false);
    }
    let application = apply_unreviewed_profile(
        backend,
        Some(cooling),
        ApplyGpuProfileRequest {
            gpu_id: gpu_id.into(),
            settings,
        },
    );
    let results: Vec<_> = [
        &application.power,
        &application.graphics,
        &application.memory,
    ]
    .into_iter()
    .flatten()
    .chain(application.cooling.as_ref().map(|c| &c.outcome))
    .collect();
    let full = !results.is_empty()
        && results
            .iter()
            .all(|result| result.status == TuningOutcomeStatus::Succeeded);
    let attempted = results
        .iter()
        .any(|result| result.status != TuningOutcomeStatus::Unattempted);
    outcome.status = if full {
        StartupOutcomeStatus::Applied
    } else if attempted {
        StartupOutcomeStatus::Failed
    } else {
        StartupOutcomeStatus::NotAttempted
    };
    // A fan's failure is reported with that fan, not in the cooling outcome.
    let fan_errors = application.cooling.iter().flat_map(|cooling| {
        cooling.fans.iter().filter_map(|fan| {
            let error = fan.outcome.error.as_ref()?;
            Some(format!("Fan {}: {error}", fan.fan_index + 1))
        })
    });
    outcome.reason = (!full).then(|| {
        results
            .iter()
            .filter_map(|result| result.error.clone())
            .chain(fan_errors)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join("\n")
    });
    outcome.application = Some(application);
    Some(full)
}
