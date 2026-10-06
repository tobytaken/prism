use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::drivers::{request_host_reboot, DriverService};
use crate::gpu::{
    ApplyGpuConfigRequest, GpuApplyRecord, GpuCommandError, GpuCommandErrorCode, GpuDaemonStatus,
    GpuInventoryResponse, GpuMutationResponse, PersistedGpuState, SetGpuClockOffsetsRequest,
};
use crate::protocol::{
    ApplyGpuProfileRequest, ClockOffsetResponse, CorrectAndApplyProfileRequest,
    CorrectedProfileApplication, DaemonCommandResponse, DaemonEvent, DaemonRequest, DaemonResponse,
    GpuTuningProfile, GpuTuningState, LiveClockOffsetsRequest, OffsetInstruction,
    ProfileApplicationResponse, ProfileSetting, ProfileSettingKey, ProfileSettings,
    SaveProfileAsNewRequest, SaveToCurrentProfileRequest, TuningOutcome, TuningOutcomeStatus,
};

use super::backend::DaemonBackend;
use super::host::{Clock, HostShutdownSignal};
use super::possibly_stale::{PossiblyStale, POSSIBLY_STALE_FILE_NAME};
use super::profiles::{capture_settings, new_profile_id, profile_name, uuid_of, verified_gpu};
use super::queue::{GpuOperationQueues, PendingOperation};
use super::tuning::{apply_live_offsets, apply_profile, reset_live_offsets};
use super::tuning_store::{TuningDocument, TuningStore, TUNING_STORE_FILE_NAME};
use super::{DaemonSession, SessionPeer, SessionPolicy};

pub(super) const LEGACY_STATE_FILE_NAME: &str = "state.json";

/// Everything the daemon core needs from the host, so tests can substitute each part.
pub struct CoreDependencies {
    pub backend: Arc<dyn DaemonBackend>,
    pub drivers: DriverService,
    /// Survives reboots.
    pub persistent_dir: PathBuf,
    /// Survives daemon restarts and is cleared at reboot.
    pub boot_dir: PathBuf,
    pub boot_id: String,
    pub clock: Arc<dyn Clock>,
    pub host_shutting_down: Arc<dyn HostShutdownSignal>,
    /// Reported by daemon status requests.
    pub socket_path: PathBuf,
}

/// The daemon below the socket. Requests may arrive from several connections at once: reads
/// never wait for tuning writes, and each GPU's writes run in order on that GPU's queue.
pub struct DaemonCore {
    pub(super) dependencies: CoreDependencies,
    pub(super) queues: GpuOperationQueues,
    pub(super) tuning_store: Arc<TuningStore>,
    legacy_configs: Arc<LegacyConfigStore>,
    pub(super) boot_state: Arc<super::boot_state::BootState>,
    possibly_stale: Arc<PossiblyStale>,
    pub(super) stopping: Arc<AtomicBool>,
    pub(super) startup_operations: Mutex<Vec<PendingOperation<()>>>,
}

impl DaemonCore {
    pub fn open_session(
        &self,
        peer: SessionPeer,
        policy: &SessionPolicy,
    ) -> Result<DaemonSession, GpuCommandError> {
        policy.authorize(&peer)?;
        Ok(self.tuning_store.subscribe(peer))
    }

    pub fn handle_session(
        &self,
        session: &DaemonSession,
        request: DaemonRequest,
    ) -> DaemonResponse {
        self.handle_for_peer(session.peer(), request)
    }

    /// Handles a request from an identified caller, whose own profiles it may use.
    pub(super) fn handle_for_peer(
        &self,
        peer: &SessionPeer,
        request: DaemonRequest,
    ) -> DaemonResponse {
        let response = match request {
            DaemonRequest::GetStartupState => self
                .tuning_store
                .read(|document| document.startup_state())
                .map_err(GpuCommandError::from)
                .and_then(|mut state| {
                    state.outcomes = self.boot_state.outcomes(peer)?;
                    state.possibly_stale = self.possibly_stale.gpus();
                    Ok(DaemonResponse::StartupState(state))
                }),
            DaemonRequest::SetStartupBehavior(request) => self
                .tuning_store
                .update(|document| {
                    document.startup_behavior = request.behavior;
                })
                .map_err(GpuCommandError::from)
                .map(|()| startup_command()),
            DaemonRequest::SetStartupProfile(request) => {
                let store = Arc::clone(&self.tuning_store);
                let peer = peer.clone();
                self.tracked_write(&request.gpu_id.clone(), move |_| {
                    store.try_update(|document| document.set_startup_profile(&peer, request))
                })
                .map(|()| startup_command())
            }
            DaemonRequest::DismissStartupOutcome(request) => self
                .boot_state
                .dismiss(peer, &request.gpu_id)
                .map(|()| startup_command()),
            DaemonRequest::ListProfiles => self
                .tuning_store
                .read(|document| document.profiles_of(peer))
                .map_err(GpuCommandError::from)
                .map(DaemonResponse::Profiles),
            DaemonRequest::GetTuningStates => {
                let performance = self.dependencies.backend.performance();
                self.tuning_store
                    .read(|document| {
                        performance
                            .gpus
                            .iter()
                            .filter_map(|gpu| {
                                verified_gpu(&performance, gpu.stable_id.as_deref()?).ok()
                            })
                            .map(|gpu| document.tuning_state(gpu))
                            .collect()
                    })
                    .map_err(GpuCommandError::from)
                    .map(DaemonResponse::TuningStates)
            }
            DaemonRequest::SaveProfileAsNew(request) => self
                .save_profile_as_new(peer, request)
                .map(DaemonResponse::Profile),
            DaemonRequest::SaveToCurrentProfile(request) => self
                .save_to_current_profile(peer, request)
                .map(DaemonResponse::Profile),
            DaemonRequest::RenameProfile(request) => {
                let peer = peer.clone();
                self.update_profile(
                    &peer.clone(),
                    &request.profile_id.clone(),
                    move |document| {
                        document.rename_profile(&peer, &request.profile_id, &request.name)
                    },
                )
                .map(DaemonResponse::Profile)
            }
            DaemonRequest::DeleteProfile(request) => {
                let peer = peer.clone();
                self.update_profile(
                    &peer.clone(),
                    &request.profile_id.clone(),
                    move |document| document.delete_profile(&peer, &request.profile_id),
                )
                .map(|()| {
                    DaemonResponse::Command(DaemonCommandResponse {
                        success: true,
                        error: None,
                    })
                })
            }
            DaemonRequest::ApplyProfileById(request) => self
                .tuning_store
                .read(|document| document.profile_of(peer, &request.profile_id))
                .map_err(GpuCommandError::from)
                .and_then(|profile| profile)
                .and_then(|profile| self.apply_saved_profile(peer, profile))
                .map(DaemonResponse::ProfileApplication),
            DaemonRequest::CorrectAndApplyProfile(request) => self
                .correct_and_apply_profile(peer, request)
                .map(DaemonResponse::CorrectedProfileApplication),
            DaemonRequest::ImportProfiles(request) => self
                .tuning_store
                .try_update(|document| document.import_profiles(peer, request.profiles))
                .map(DaemonResponse::ProfileImport),
            request => Ok(self.handle(request)),
        };
        response.unwrap_or_else(DaemonResponse::Error)
    }

    /// Edits a saved profile on its GPU's queue, without changing any GPU controls.
    fn update_profile<T: Send + 'static>(
        &self,
        peer: &SessionPeer,
        profile_id: &str,
        change: impl FnOnce(&mut TuningDocument) -> Result<T, GpuCommandError> + Send + 'static,
    ) -> Result<T, GpuCommandError> {
        let profile = self
            .tuning_store
            .read(|document| document.profile_of(peer, profile_id))??;
        let store = Arc::clone(&self.tuning_store);
        self.tracked_write(&profile.gpu_id, move |_| store.try_update(change))
    }

    // Saving runs on the GPU's queue, so it captures the values after any write queued before it.
    // The new profile becomes the GPU's current profile.
    fn save_profile_as_new(
        &self,
        peer: &SessionPeer,
        request: SaveProfileAsNewRequest,
    ) -> Result<GpuTuningProfile, GpuCommandError> {
        let name = profile_name(&request.name)?;
        let peer = peer.clone();
        let gpu_id = request.gpu_id.clone();
        self.save(
            &request.gpu_id,
            request.revision,
            move |document, settings| {
                let profile = document.add_profile(
                    &peer,
                    GpuTuningProfile {
                        id: new_profile_id()?,
                        name,
                        gpu_id: gpu_id.clone(),
                        settings,
                    },
                )?;
                document.current_profiles.insert(gpu_id, profile.id.clone());
                Ok(profile)
            },
        )
    }

    fn save_to_current_profile(
        &self,
        peer: &SessionPeer,
        request: SaveToCurrentProfileRequest,
    ) -> Result<GpuTuningProfile, GpuCommandError> {
        let peer = peer.clone();
        let gpu_id = request.gpu_id.clone();
        self.save(
            &request.gpu_id,
            request.revision,
            move |document, settings| document.save_to_current(&peer, &gpu_id, settings),
        )
    }

    /// Captures the GPU's applied values from fresh readings on its queue and stores them as
    /// `keep` decides, provided the caller saw the GPU's tuning state at `revision`.
    fn save(
        &self,
        uuid: &str,
        revision: String,
        keep: impl FnOnce(
                &mut TuningDocument,
                ProfileSettings,
            ) -> Result<GpuTuningProfile, GpuCommandError>
            + Send
            + 'static,
    ) -> Result<GpuTuningProfile, GpuCommandError> {
        let store = Arc::clone(&self.tuning_store);
        let uuid = uuid.to_string();
        self.tracked_write(&uuid.clone(), move |backend| {
            let performance = backend.performance();
            let gpu = verified_gpu(&performance, &uuid)?;
            store.try_update(|document| {
                document.check_revision(gpu, &revision)?;
                let settings = capture_settings(gpu, document.last_applied.get(&uuid))?;
                keep(document, settings)
            })
        })
    }

    // Approved corrections are saved before application and stay saved whatever it reports.
    fn correct_and_apply_profile(
        &self,
        peer: &SessionPeer,
        request: CorrectAndApplyProfileRequest,
    ) -> Result<CorrectedProfileApplication, GpuCommandError> {
        let profile = self.tuning_store.try_update(|document| {
            document.correct_profile(peer, &request.profile_id, &request.corrections)
        })?;
        let application = self.apply_saved_profile(peer, profile.clone())?;
        Ok(CorrectedProfileApplication {
            profile,
            application,
        })
    }

    /// Applies the profile. Applied in full, it becomes the GPU's current profile; applied in
    /// part, the GPU is left with no current profile; when nothing was applied, the previous
    /// current profile stays.
    fn apply_saved_profile(
        &self,
        peer: &SessionPeer,
        profile: GpuTuningProfile,
    ) -> Result<ProfileApplicationResponse, GpuCommandError> {
        let store = Arc::clone(&self.tuning_store);
        let stale = Arc::clone(&self.possibly_stale);
        let peer = peer.clone();
        self.tracked_write(&profile.gpu_id, move |backend| {
            // A profile can be deleted or edited while this application waits on the GPU's queue.
            let profile = store.read(|document| document.profile_of(&peer, &profile.id))??;
            Ok(apply_recorded_profile(
                backend,
                &store,
                &stale,
                ApplyGpuProfileRequest {
                    gpu_id: profile.gpu_id,
                    settings: profile.settings,
                },
                Some(profile.id),
            ))
        })
    }

    fn apply_profile_settings(
        &self,
        request: ApplyGpuProfileRequest,
    ) -> ProfileApplicationResponse {
        let store = Arc::clone(&self.tuning_store);
        let stale = Arc::clone(&self.possibly_stale);
        self.tracked_write(&request.gpu_id.clone(), move |backend| {
            apply_recorded_profile(backend, &store, &stale, request, None)
        })
    }

    /// Runs a live tuning change on the GPU's queue, then records the instructions it applied as
    /// that GPU's last applied tuning values.
    fn live_write<T: ManualWriteResponse + Send + 'static>(
        &self,
        gpu_id: String,
        write: impl FnOnce(&dyn DaemonBackend) -> (T, Vec<(ProfileSettingKey, ProfileSetting)>)
            + Send
            + 'static,
    ) -> T {
        let store = Arc::clone(&self.tuning_store);
        let stale = Arc::clone(&self.possibly_stale);
        self.tracked_write(&gpu_id.clone(), move |backend| {
            let (mut response, applied) = write(backend);
            if !applied.is_empty() {
                if let Some(uuid) = uuid_of(&backend.performance(), &gpu_id) {
                    if !record_manual(&store, &stale, &uuid, &applied, |_| {}) {
                        response.not_recorded();
                    }
                }
            }
            response
        })
    }

    /// Runs a tuning write on the GPU's queue, then pushes the GPU's tuning state to every
    /// session if the write changed it.
    fn tracked_write<T: Send + 'static>(
        &self,
        gpu_id: &str,
        operation: impl FnOnce(&dyn DaemonBackend) -> T + Send + 'static,
    ) -> T {
        let store = Arc::clone(&self.tuning_store);
        let uuid = gpu_id.to_string();
        self.write(gpu_id, move |backend| {
            let before = tuning_state(backend, &store, &uuid);
            let result = operation(backend);
            match tuning_state(backend, &store, &uuid) {
                Some(after) if Some(&after) != before.as_ref() => {
                    store.publish(DaemonEvent::TuningStateChanged(after))
                }
                _ => {}
            }
            result
        })
    }

    /// Loads host state and reserves startup operations before serving any manual requests.
    pub fn start(dependencies: CoreDependencies) -> Self {
        let legacy_configs = Arc::new(LegacyConfigStore::load(
            dependencies.persistent_dir.join(LEGACY_STATE_FILE_NAME),
        ));
        let tuning_store = Arc::new(TuningStore::open(
            dependencies.persistent_dir.join(TUNING_STORE_FILE_NAME),
        ));
        let boot_state = Arc::new(super::boot_state::BootState::open(
            &dependencies.boot_dir,
            &dependencies.boot_id,
        ));
        let possibly_stale = Arc::new(PossiblyStale::open(
            dependencies.persistent_dir.join(POSSIBLY_STALE_FILE_NAME),
        ));
        let core = Self {
            dependencies,
            boot_state,
            possibly_stale,
            stopping: Arc::default(),
            startup_operations: Mutex::default(),
            queues: GpuOperationQueues::default(),
            tuning_store,
            legacy_configs,
        };

        if let Err(error) = core.start_startup() {
            eprintln!("Startup is suspended: {}", error.message);
        }
        // Interim rule until #38: Off retains legacy reapplication on every daemon start.
        if matches!(
            core.tuning_store.read(|document| document.startup_behavior),
            Ok(crate::protocol::StartupBehavior::Off)
        ) {
            let outcome = core.reapply_legacy_configs();
            if let (_, Err(error)) = core.legacy_configs.save() {
                eprintln!("failed to save Prism daemon state on startup: {error}");
            }
            if let Err(error) = outcome {
                eprintln!("failed to reapply saved GPU settings: {}", error.message);
            }
        }

        core
    }

    /// Stops the daemon. Only a stop while the host shuts down marks this boot clean for the
    /// unclean-shutdown guard; a daemon restart mid-boot records nothing.
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        if !self.dependencies.host_shutting_down.host_shutting_down() {
            return;
        }
        let boot_id = &self.dependencies.boot_id;
        let restored_this_boot = self.tuning_store.read(|document| {
            document
                .restore_attempt
                .as_ref()
                .is_some_and(|attempt| attempt.boot_id == *boot_id)
        });
        if let Ok(true) = restored_this_boot {
            if let Err(error) = self
                .tuning_store
                .update(|document| document.restore_attempt = None)
            {
                eprintln!("failed to record a clean shutdown after restoring last applied tuning values: {error}");
            }
        }
    }

    /// The service becomes ready only after all boot operations have finished. Request
    /// serving runs independently, so reads and unrelated GPUs stay available meanwhile.
    pub fn wait_for_startup(&self) {
        for pending in std::mem::take(&mut *self.startup_operations.lock().unwrap()) {
            pending.wait();
        }
    }

    pub fn handle(&self, request: DaemonRequest) -> DaemonResponse {
        let drivers = &self.dependencies.drivers;
        match request {
            DaemonRequest::OpenSession => DaemonResponse::Error(GpuCommandError::new(
                GpuCommandErrorCode::InvalidArgument,
                "OpenSession requires a framed connection",
            )),
            DaemonRequest::GetInventory => {
                DaemonResponse::Inventory(self.inventory(&self.legacy_configs.snapshot()))
            }
            DaemonRequest::GetPerformance => {
                DaemonResponse::Performance(self.dependencies.backend.performance())
            }
            DaemonRequest::ApplyProfile(request) => {
                DaemonResponse::ProfileApplication(self.apply_profile_settings(request))
            }
            DaemonRequest::GetDaemonStatus => {
                DaemonResponse::DaemonStatus(GpuDaemonStatus::connected(
                    self.dependencies.socket_path.display().to_string(),
                    self.tuning_store.health(),
                ))
            }
            DaemonRequest::ApplyConfig(request) => {
                DaemonResponse::Mutation(if request.reapply_saved {
                    self.reapply_legacy_configs_and_save()
                } else {
                    self.apply_legacy_config(request)
                })
            }
            DaemonRequest::ResetConfig(request) => {
                DaemonResponse::Mutation(self.reset_legacy_config(request.gpu_id))
            }
            DaemonRequest::ReapplySavedConfig => {
                DaemonResponse::Mutation(self.reapply_legacy_configs_and_save())
            }
            DaemonRequest::SetPowerLimit(request) => {
                let applied = ProfileSetting::Value {
                    value: i64::from(request.limit_watts),
                };
                DaemonResponse::Command(self.live_write(request.gpu_id.clone(), move |backend| {
                    power_applied(backend.set_power_limit(request), applied)
                }))
            }
            DaemonRequest::ResetPowerLimit(request) => {
                DaemonResponse::Command(self.live_write(request.gpu_id.clone(), move |backend| {
                    power_applied(
                        backend.reset_power_limit(request),
                        ProfileSetting::Default {},
                    )
                }))
            }
            DaemonRequest::SetFanSpeed(request) => self
                .live_command(request.gpu_id.clone(), move |backend| {
                    backend.set_fan_speed(request)
                }),
            DaemonRequest::SetFanModeAuto(request) => self
                .live_command(request.gpu_id.clone(), move |backend| {
                    backend.set_fan_mode_auto(request)
                }),
            DaemonRequest::SetClockOffsets(request) => DaemonResponse::ClockOffsets(
                self.live_write(request.gpu_id.clone(), move |backend| {
                    set_live_offsets(backend, request)
                }),
            ),
            DaemonRequest::ResetClockOffsets(request) => DaemonResponse::ClockOffsets(
                self.live_write(request.gpu_id.clone(), move |backend| {
                    let response = reset_live_offsets(backend, request);
                    let applied = offsets_applied(
                        &response,
                        Some(ProfileSetting::Default {}),
                        Some(ProfileSetting::Default {}),
                    );
                    (response, applied)
                }),
            ),
            DaemonRequest::GetStartupState
            | DaemonRequest::SetStartupBehavior(_)
            | DaemonRequest::SetStartupProfile(_)
            | DaemonRequest::DismissStartupOutcome(_)
            | DaemonRequest::ListProfiles
            | DaemonRequest::GetTuningStates
            | DaemonRequest::SaveProfileAsNew(_)
            | DaemonRequest::SaveToCurrentProfile(_)
            | DaemonRequest::RenameProfile(_)
            | DaemonRequest::DeleteProfile(_)
            | DaemonRequest::ApplyProfileById(_)
            | DaemonRequest::CorrectAndApplyProfile(_)
            | DaemonRequest::ImportProfiles(_) => DaemonResponse::Error(GpuCommandError::new(
                GpuCommandErrorCode::NoPermission,
                "GPU tuning profiles need a connection that identifies the caller.",
            )),
            DaemonRequest::GetDriverStatus => DaemonResponse::DriverStatus(drivers.status()),
            DaemonRequest::StartDriverAction(request) => match drivers.start_action(request) {
                Ok(job) => DaemonResponse::DriverJob(job),
                Err(error) => DaemonResponse::Error(error),
            },
            DaemonRequest::GetDriverJob(request) => match drivers.job(request) {
                Ok(job) => DaemonResponse::DriverJob(job),
                Err(error) => DaemonResponse::Error(error),
            },
            DaemonRequest::CancelDriverJob(request) => match drivers.cancel_job(request) {
                Ok(job) => DaemonResponse::DriverJob(job),
                Err(error) => DaemonResponse::Error(error),
            },
            DaemonRequest::RequestHostReboot => DaemonResponse::Command(command_result(
                request_host_reboot().map_err(|error| error.message),
            )),
        }
    }

    /// Queues a tuning write on the GPU's operation queue.
    fn submit_write<T: Send + 'static>(
        &self,
        gpu_id: &str,
        operation: impl FnOnce(&dyn DaemonBackend) -> T + Send + 'static,
    ) -> PendingOperation<T> {
        let backend = Arc::clone(&self.dependencies.backend);
        self.queues
            .submit(gpu_id, move || operation(backend.as_ref()))
    }

    /// Runs a tuning write on the GPU's operation queue and waits for its result.
    fn write<T: Send + 'static>(
        &self,
        gpu_id: &str,
        operation: impl FnOnce(&dyn DaemonBackend) -> T + Send + 'static,
    ) -> T {
        self.submit_write(gpu_id, operation).wait()
    }

    fn live_command(
        &self,
        gpu_id: String,
        command: impl FnOnce(&dyn DaemonBackend) -> Result<(), String> + Send + 'static,
    ) -> DaemonResponse {
        DaemonResponse::Command(command_result(self.write(&gpu_id, command)))
    }

    // Legacy saved configurations change on the GPU's queue, right after the write they record,
    // so they follow the order the GPU was written in.

    fn apply_legacy_config(&self, request: ApplyGpuConfigRequest) -> GpuMutationResponse {
        let Some(gpu_id) = request.gpu_id.clone() else {
            let error = GpuCommandError::new(
                GpuCommandErrorCode::InvalidArgument,
                "gpuId is required when applying a GPU configuration",
            );
            return self.save_and_respond(|_| GpuMutationResponse::failure(error));
        };

        let store = Arc::clone(&self.legacy_configs);
        let clock = Arc::clone(&self.dependencies.clock);
        let result = self.write(&gpu_id.clone(), move |backend| {
            let result = backend.apply(&request);
            let now = clock.now_unix_ms();
            store.change(|state| match &result {
                Ok(()) => {
                    state.upsert_desired(gpu_id.clone(), request.desired_config());
                    state.set_last_apply(gpu_id, GpuApplyRecord::success_at(now));
                }
                Err(error) => state.set_last_apply(
                    gpu_id,
                    GpuApplyRecord::failure_at(now, error.message.clone()),
                ),
            });
            result
        });
        self.save_and_respond(|state| self.mutation_response(result, state))
    }

    fn reset_legacy_config(&self, gpu_id: String) -> GpuMutationResponse {
        let store = Arc::clone(&self.legacy_configs);
        let clock = Arc::clone(&self.dependencies.clock);
        let result = self.write(&gpu_id.clone(), move |backend| {
            let result = backend.reset(&gpu_id);
            let now = clock.now_unix_ms();
            store.change(|state| match &result {
                Ok(()) => state.clear(&gpu_id),
                Err(error) => {
                    if state.record_for(&gpu_id).is_some() {
                        state.set_last_apply(
                            gpu_id,
                            GpuApplyRecord::failure_at(now, error.message.clone()),
                        );
                    }
                }
            });
            result
        });
        self.save_and_respond(|state| self.mutation_response(result, state))
    }

    fn reapply_legacy_configs_and_save(&self) -> GpuMutationResponse {
        let outcome = self.reapply_legacy_configs();
        self.save_and_respond(|state| self.mutation_response(outcome, state))
    }

    /// Reapplies every non-empty legacy saved configuration, each on its GPU's queue. Each GPU's
    /// configuration is read when its write runs, so changes queued ahead of it are respected.
    fn reapply_legacy_configs(&self) -> Result<(), GpuCommandError> {
        let pending: Vec<_> = self
            .legacy_configs
            .snapshot()
            .gpus
            .into_keys()
            .map(|gpu_id| {
                let store = Arc::clone(&self.legacy_configs);
                let clock = Arc::clone(&self.dependencies.clock);
                self.submit_write(&gpu_id.clone(), move |backend| {
                    let Some(request) = store.reapplication_request(&gpu_id) else {
                        return Ok(());
                    };
                    let result = backend.apply(&request);
                    let now = clock.now_unix_ms();
                    let record = match &result {
                        Ok(()) => GpuApplyRecord::success_at(now),
                        Err(error) => GpuApplyRecord::failure_at(now, error.message.clone()),
                    };
                    store.change(|state| state.set_last_apply(gpu_id, record));
                    result
                })
            })
            .collect();

        let mut outcome = Ok(());
        for operation in pending {
            if let (Err(error), Ok(())) = (operation.wait(), &outcome) {
                outcome = Err(GpuCommandError::new(
                    error.code,
                    "failed to reapply one or more saved GPU settings",
                ));
            }
        }
        outcome
    }

    /// Saves the legacy saved configurations, as every legacy mutation always has, reporting a
    /// save failure in place of the mutation's own response.
    fn save_and_respond(
        &self,
        respond: impl FnOnce(&PersistedGpuState) -> GpuMutationResponse,
    ) -> GpuMutationResponse {
        match self.legacy_configs.save() {
            (state, Ok(())) => respond(&state),
            (state, Err(error)) => GpuMutationResponse {
                success: false,
                inventory: Some(self.inventory(&state)),
                error: Some(GpuCommandError::new(
                    GpuCommandErrorCode::Io,
                    error.to_string(),
                )),
            },
        }
    }

    fn mutation_response(
        &self,
        result: Result<(), GpuCommandError>,
        state: &PersistedGpuState,
    ) -> GpuMutationResponse {
        GpuMutationResponse {
            success: result.is_ok(),
            inventory: Some(self.inventory(state)),
            error: result.err(),
        }
    }

    fn inventory(&self, state: &PersistedGpuState) -> GpuInventoryResponse {
        self.dependencies.backend.inventory(state)
    }
}

/// Applies settings and records successful manual instructions and the resulting current profile.
fn apply_recorded_profile(
    backend: &dyn DaemonBackend,
    store: &TuningStore,
    stale: &PossiblyStale,
    request: ApplyGpuProfileRequest,
    profile_id: Option<String>,
) -> ProfileApplicationResponse {
    let uuid = request.gpu_id.clone();
    let settings = request.settings.clone();
    let mut response = apply_profile(backend, request);
    let outcomes = [
        (ProfileSettingKey::PowerLimitWatts, &response.power),
        (ProfileSettingKey::GraphicsOffsetMhz, &response.graphics),
        (ProfileSettingKey::MemoryOffsetMhz, &response.memory),
    ];
    let included = outcomes.iter().filter(|(_, outcome)| outcome.is_some());
    let applied: Vec<_> = outcomes
        .iter()
        .filter(|(_, outcome)| succeeded(outcome))
        .filter_map(|(key, _)| settings.get(*key).map(|setting| (*key, setting)))
        .collect();
    let current = match profile_id {
        Some(id) if applied.len() == included.count() => CurrentProfileChange::Become(id),
        Some(_) if !applied.is_empty() => CurrentProfileChange::Clear,
        _ => CurrentProfileChange::Keep,
    };
    let recorded = record_manual(store, stale, &uuid, &applied, |document| match current {
        CurrentProfileChange::Become(id) => {
            document.current_profiles.insert(uuid.clone(), id);
        }
        CurrentProfileChange::Clear => {
            document.current_profiles.remove(&uuid);
        }
        CurrentProfileChange::Keep => {}
    });
    if !recorded {
        response.not_recorded();
    }
    response
}

/// What a profile application does to the GPU's current profile.
enum CurrentProfileChange {
    /// Applied in full, the profile becomes current.
    Become(String),
    /// Applied in part, no profile describes the GPU.
    Clear,
    /// Nothing was applied, so the previous current profile stays.
    Keep,
}

fn succeeded(outcome: &Option<TuningOutcome>) -> bool {
    matches!(
        outcome,
        Some(TuningOutcome {
            status: TuningOutcomeStatus::Succeeded,
            ..
        })
    )
}

fn power_applied(
    result: Result<(), String>,
    setting: ProfileSetting,
) -> (
    DaemonCommandResponse,
    Vec<(ProfileSettingKey, ProfileSetting)>,
) {
    let applied = match result {
        Ok(()) => vec![(ProfileSettingKey::PowerLimitWatts, setting)],
        Err(_) => vec![],
    };
    (command_result(result), applied)
}

fn offsets_applied(
    response: &ClockOffsetResponse,
    graphics: Option<ProfileSetting>,
    memory: Option<ProfileSetting>,
) -> Vec<(ProfileSettingKey, ProfileSetting)> {
    [
        (
            ProfileSettingKey::GraphicsOffsetMhz,
            &response.graphics,
            graphics,
        ),
        (ProfileSettingKey::MemoryOffsetMhz, &response.memory, memory),
    ]
    .into_iter()
    .filter(|(_, outcome, _)| succeeded(outcome))
    .filter_map(|(key, _, setting)| setting.map(|setting| (key, setting)))
    .collect()
}

/// Writes live offsets, where a default request writes the default offset of zero.
fn set_live_offsets(
    backend: &dyn DaemonBackend,
    request: LiveClockOffsetsRequest,
) -> (
    ClockOffsetResponse,
    Vec<(ProfileSettingKey, ProfileSetting)>,
) {
    let instruction = |offset: Option<OffsetInstruction>| {
        offset.map(|offset| match offset {
            OffsetInstruction::Value(value) => ProfileSetting::Value {
                value: i64::from(value),
            },
            OffsetInstruction::Default(_) => ProfileSetting::Default {},
        })
    };
    let megahertz = |offset: Option<OffsetInstruction>| {
        offset.map(|offset| match offset {
            OffsetInstruction::Value(value) => value,
            OffsetInstruction::Default(_) => 0,
        })
    };
    let response = apply_live_offsets(
        backend,
        SetGpuClockOffsetsRequest {
            gpu_id: request.gpu_id,
            graphics_offset_mhz: megahertz(request.graphics_offset_mhz),
            memory_offset_mhz: megahertz(request.memory_offset_mhz),
        },
    );
    let applied = offsets_applied(
        &response,
        instruction(request.graphics_offset_mhz),
        instruction(request.memory_offset_mhz),
    );
    (response, applied)
}

const NOT_RECORDED: &str = "This change was not recorded for startup.";

/// Records a manual write's successful instructions as the GPU's last applied tuning values,
/// along with the rest of what it changed. The write already happened, so failing to record it
/// never fails or undoes the write; its settings are marked possibly stale instead. Returns
/// whether the record was written.
fn record_manual(
    store: &TuningStore,
    stale: &PossiblyStale,
    uuid: &str,
    applied: &[(ProfileSettingKey, ProfileSetting)],
    change: impl FnOnce(&mut TuningDocument),
) -> bool {
    let keys = applied.iter().map(|(key, _)| *key);
    match store.update(|document| {
        document.record_applied(uuid, applied);
        change(document);
    }) {
        Ok(()) => {
            stale.clear(uuid, keys);
            true
        }
        Err(error) => {
            eprintln!("failed to record the tuning state of {uuid}: {error}");
            stale.mark(uuid, keys);
            false
        }
    }
}

/// A manual write's response, which reports each setting that reached the GPU as succeeded even
/// when its last applied tuning value couldn't be recorded.
trait ManualWriteResponse {
    fn not_recorded(&mut self);
}

impl ManualWriteResponse for DaemonCommandResponse {
    fn not_recorded(&mut self) {
        if self.success {
            self.error = Some(NOT_RECORDED.into());
        }
    }
}

impl ManualWriteResponse for ClockOffsetResponse {
    fn not_recorded(&mut self) {
        not_recorded([&mut self.graphics, &mut self.memory]);
    }
}

impl ManualWriteResponse for ProfileApplicationResponse {
    fn not_recorded(&mut self) {
        not_recorded([&mut self.power, &mut self.graphics, &mut self.memory]);
    }
}

fn not_recorded<const N: usize>(outcomes: [&mut Option<TuningOutcome>; N]) {
    for outcome in outcomes.into_iter().flatten() {
        if outcome.status == TuningOutcomeStatus::Succeeded {
            outcome.error = Some(NOT_RECORDED.into());
        }
    }
}

/// The GPU's tuning state from fresh readings, unless the GPU can't be identified or the
/// tuning store can't be used.
fn tuning_state(
    backend: &dyn DaemonBackend,
    store: &TuningStore,
    uuid: &str,
) -> Option<GpuTuningState> {
    let performance = backend.performance();
    let gpu = verified_gpu(&performance, uuid).ok()?;
    store.read(|document| document.tuning_state(gpu)).ok()
}

fn command_result(result: Result<(), String>) -> DaemonCommandResponse {
    match result {
        Ok(()) => DaemonCommandResponse {
            success: true,
            error: None,
        },
        Err(error) => DaemonCommandResponse {
            success: false,
            error: Some(error),
        },
    }
}

/// The legacy saved GPU configurations. Its lock serializes every change and save, and is
/// never held across a GPU write.
struct LegacyConfigStore {
    path: PathBuf,
    state: Mutex<PersistedGpuState>,
}

impl LegacyConfigStore {
    fn load(path: PathBuf) -> Self {
        let state = Mutex::new(load_state(&path));
        Self { path, state }
    }

    fn snapshot(&self) -> PersistedGpuState {
        self.state.lock().unwrap().clone()
    }

    fn change(&self, change: impl FnOnce(&mut PersistedGpuState)) {
        change(&mut self.state.lock().unwrap());
    }

    /// Writes the current state to disk, returning what was saved and whether saving worked.
    fn save(&self) -> (PersistedGpuState, Result<(), std::io::Error>) {
        let state = self.state.lock().unwrap();
        let saved = save_state(&self.path, &state);
        (state.clone(), saved)
    }

    /// The GPU's saved configuration as an apply request, unless it has none.
    fn reapplication_request(&self, gpu_id: &str) -> Option<ApplyGpuConfigRequest> {
        let state = self.state.lock().unwrap();
        let desired = state
            .desired_for(gpu_id)
            .filter(|desired| !desired.is_empty())?;
        Some(ApplyGpuConfigRequest {
            gpu_id: Some(gpu_id.to_string()),
            power_limit_watts: desired.power_limit_watts,
            gpu_clock_lock: desired.gpu_clock_lock,
            memory_clock_lock: desired.memory_clock_lock,
            graphics_offset_mhz: desired.graphics_offset_mhz,
            memory_offset_mhz: desired.memory_offset_mhz,
            reapply_saved: false,
        })
    }
}

fn load_state(path: &Path) -> PersistedGpuState {
    match fs::read_to_string(path) {
        Ok(contents) => match serde_json::from_str(&contents) {
            Ok(state) => state,
            Err(error) => {
                eprintln!(
                    "failed to parse Prism daemon state file {}: {error}",
                    path.display()
                );
                PersistedGpuState::default()
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => PersistedGpuState::default(),
        Err(error) => {
            eprintln!(
                "failed to read Prism daemon state file {}: {error}",
                path.display()
            );
            PersistedGpuState::default()
        }
    }
}

fn save_state(path: &Path, state: &PersistedGpuState) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let payload = serde_json::to_vec_pretty(state)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string()))?;
    fs::write(path, payload)
}

fn startup_command() -> DaemonResponse {
    DaemonResponse::Command(DaemonCommandResponse {
        success: true,
        error: None,
    })
}

impl Drop for DaemonCore {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
    }
}
