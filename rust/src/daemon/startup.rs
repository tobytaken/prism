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

use std::collections::BTreeSet;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use super::boot_state::STARTUP_IN_PROGRESS;
use super::daemon_core::DaemonCore;
use super::profiles::verified_gpu;
use super::tuning::apply_startup_profile;
use crate::protocol::{
    ApplyGpuProfileRequest, StartupBehavior, StartupOutcome, StartupOutcomeStatus,
    TuningOutcomeStatus,
};

impl DaemonCore {
    pub(super) fn start_startup(&self) -> Result<(), GpuCommandError> {
        // Reading an unhealthy store fails before the marker or any hardware can be changed.
        let document = self.tuning_store.read(Clone::clone)?;
        let performance = self.dependencies.backend.performance();
        let gpu_ids: BTreeSet<_> = performance
            .gpus
            .iter()
            .filter_map(|gpu| gpu.stable_id.clone())
            .filter(|uuid| !uuid.is_empty())
            .chain(document.startup_profiles.keys().cloned())
            .collect();
        let outcomes: Vec<_> = gpu_ids
            .into_iter()
            .map(|gpu_id| {
                let profile = document
                    .startup_profiles
                    .get(&gpu_id)
                    .and_then(|id| document.any_profile(id));
                StartupOutcome {
                    gpu_id,
                    profile: profile.map(|profile| CurrentProfile {
                        id: profile.id.clone(),
                        name: profile.name.clone(),
                    }),
                    status: StartupOutcomeStatus::NotAttempted,
                    reason: Some(
                        if document.startup_behavior == StartupBehavior::Off {
                            "Startup behavior is Off."
                        } else if profile.is_none() {
                            "No startup profile."
                        } else {
                            STARTUP_IN_PROGRESS
                        }
                        .into(),
                    ),
                    application: None,
                }
            })
            .collect();
        let in_progress = if document.startup_behavior == StartupBehavior::StartupProfiles {
            document
                .startup_profiles
                .iter()
                .filter(|(_, id)| document.any_profile(id).is_some())
                .map(|(gpu_id, _)| gpu_id.clone())
                .collect()
        } else {
            BTreeSet::new()
        };
        if !self.boot_state.begin(outcomes.clone(), in_progress)? {
            return Ok(());
        }
        // A previous boot's current profile never describes what this boot actually applied.
        if !document.current_profiles.is_empty() {
            self.tuning_store
                .update(|document| document.current_profiles.clear())?;
        }
        if document.startup_behavior == StartupBehavior::Off {
            return Ok(());
        }
        let deadline = self
            .dependencies
            .clock
            .now_monotonic_ms()
            .saturating_add(30_000);
        for mut outcome in outcomes {
            let Some(profile) = document
                .startup_profiles
                .get(&outcome.gpu_id)
                .and_then(|id| document.any_profile(id))
                .cloned()
            else {
                continue;
            };
            let backend = Arc::clone(&self.dependencies.backend);
            let store = Arc::clone(&self.tuning_store);
            let boot = Arc::clone(&self.boot_state);
            let clock = Arc::clone(&self.dependencies.clock);
            let stopping = Arc::clone(&self.stopping);
            // Reserve every designated GPU's queue now. Manual writes wait behind startup;
            // reads and unrelated GPUs remain available while a designated GPU is absent.
            let pending = self.queues.submit(&profile.gpu_id.clone(), move || {
                loop {
                    if stopping.load(Ordering::SeqCst) { return; }
                    let readings = backend.performance();
                    if clock.now_monotonic_ms() >= deadline {
                        outcome.reason = Some("The GPU was absent at startup after waiting about 30 seconds. Nothing will be applied to it later this boot.".into());
                        if let Err(error) = boot.record(outcome) { eprintln!("{}", error.message); }
                        return;
                    }
                    if verified_gpu(&readings, &profile.gpu_id).is_ok() { break; }
                    std::thread::sleep(Duration::from_millis(100));
                }
                if store.read(|_| ()).is_err() {
                    outcome.reason = Some("Startup was skipped because the tuning store is unhealthy.".into());
                } else {
                    let application = apply_startup_profile(backend.as_ref(), ApplyGpuProfileRequest {
                        gpu_id: profile.gpu_id.clone(), settings: profile.settings.clone(),
                    });
                    let settings: Vec<_> = [&application.power, &application.graphics, &application.memory].into_iter().flatten().collect();
                    let full = !settings.is_empty() && settings.iter().all(|setting| setting.status == TuningOutcomeStatus::Succeeded);
                    let attempted = settings.iter().any(|setting| setting.status != TuningOutcomeStatus::Unattempted);
                    outcome.status = if full { StartupOutcomeStatus::Applied } else if attempted { StartupOutcomeStatus::Failed } else { StartupOutcomeStatus::NotAttempted };
                    outcome.reason = (!full).then(|| settings.iter().filter_map(|setting| setting.error.clone()).collect::<BTreeSet<_>>().into_iter().collect::<Vec<_>>().join("\n"));
                    outcome.application = Some(application);
                    if full {
                        if let Err(error) = store.update(|document| { document.current_profiles.insert(profile.gpu_id.clone(), profile.id.clone()); }) {
                            outcome.status = StartupOutcomeStatus::Failed;
                            outcome.reason = Some(format!("The GPU accepted the settings, but its current profile could not be recorded: {error}"));
                        }
                    }
                    if let Ok(state) = store.read(|document| {
                        let readings = backend.performance();
                        verified_gpu(&readings, &profile.gpu_id).ok().map(|gpu| document.tuning_state(gpu))
                    }) {
                        if let Some(state) = state { store.publish(crate::protocol::DaemonEvent::TuningStateChanged(state)); }
                    }
                }
                if let Err(error) = boot.record(outcome) { eprintln!("{}", error.message); }
            });
            self.startup_operations.lock().unwrap().push(pending);
        }
        Ok(())
    }
}
