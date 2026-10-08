use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::cooling::{ApplyCoolingRequest, CoolingConfiguration, FanConfiguration, FanSetting};
use crate::drivers::{request_host_reboot, DriverService};
use crate::fan_curve::{self, FanCurvePreview, PreviewFanCurveRequest};
use crate::gpu::{
    GpuCommandError, GpuCommandErrorCode, GpuDaemonStatus, GpuPerformanceResponse,
    SetGpuClockOffsetsRequest,
};
use crate::protocol::{
    ApplyGpuProfileRequest, ClockOffsetResponse, CorrectAndApplyProfileRequest,
    CorrectedProfileApplication, DaemonCommandResponse, DaemonEvent, DaemonRequest, DaemonResponse,
    GpuTuningProfile, GpuTuningState, LiveClockOffsetsRequest, OffsetInstruction,
    ProfileApplicationResponse, ProfileSetting, ProfileSettingKey, ProfileSettings,
    SaveProfileAsNewRequest, SaveToCurrentProfileRequest, TuningOutcome, TuningOutcomeStatus,
};

use super::backend::DaemonBackend;
use super::cooling::{recover_abandoned_curves, CoolingController, EVALUATION_INTERVAL_MS};
use super::host::{Clock, HostShutdownSignal, UserDirectory};
use super::possibly_stale::{PossiblyStale, POSSIBLY_STALE_FILE_NAME};
use super::profiles::{capture_settings, new_profile_id, profile_name, uuid_of, verified_gpu};
use super::queue::{GpuOperationQueues, PendingOperation};
use super::tuning::{apply_live_offsets, apply_profile, reset_live_offsets};
use super::tuning_store::{TuningDocument, TuningStore, TUNING_STORE_FILE_NAME};
use super::{DaemonSession, SessionPeer, SessionPolicy};

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
    /// Where the host's file system is read from, such as callers' Steam libraries.
    pub host_root: PathBuf,
    /// Tells which user a running game's uid belongs to now.
    pub users: Arc<dyn UserDirectory>,
}

/// The daemon below the socket. Requests may arrive from several connections at once: reads
/// never wait for tuning writes, and each GPU's writes run in order on that GPU's queue.
pub struct DaemonCore {
    pub(super) dependencies: CoreDependencies,
    pub(super) queues: GpuOperationQueues,
    pub(super) tuning_store: Arc<TuningStore>,
    pub(super) boot_state: Arc<super::boot_state::BootState>,
    possibly_stale: Arc<PossiblyStale>,
    pub(super) recognition: Arc<Mutex<super::recognition::Recognition>>,
    pub(super) switching: super::switching::Switching,
    pub(super) stopping: Arc<AtomicBool>,
    pub(super) startup_operations: Mutex<Vec<PendingOperation<()>>>,
    pub(super) cooling: Arc<CoolingController>,
}

impl DaemonCore {
    pub fn open_session(
        &self,
        peer: SessionPeer,
        policy: &SessionPolicy,
    ) -> Result<DaemonSession, GpuCommandError> {
        policy.authorize(&peer)?;
        if let Err(error) = self.tuning_store.reserve_migration_adoption(&peer) {
            eprintln!("Could not reserve migrated profiles: {error}");
        }
        self.switching.opened(&peer);
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
        // These change which game, if any, should drive a GPU, or how its status reads.
        let reevaluate = matches!(
            request,
            DaemonRequest::SetGameProfile(_)
                | DaemonRequest::RemoveGame(_)
                | DaemonRequest::SetAutomaticSwitching(_)
                | DaemonRequest::DeleteProfile(_)
                | DaemonRequest::RenameProfile(_)
        );
        let response = match request {
            DaemonRequest::AdoptMigratedProfile(request) => self
                .tuning_store
                .try_update(|document| document.adopt_migrated_profile(peer, &request.profile_id))
                .map(DaemonResponse::Profile),
            DaemonRequest::DismissMigrationNotice => self
                .tuning_store
                .update(|document| {
                    document
                        .user_mut(peer.uid, &peer.username)
                        .migration_notice_dismissed = true
                })
                .map_err(GpuCommandError::from)
                .map(|()| startup_command()),
            DaemonRequest::GetMigrationState => self
                .tuning_store
                .read(|document| document.migration_state(peer))
                .map_err(GpuCommandError::from)
                .map(DaemonResponse::MigrationState),
            DaemonRequest::GetStartupState => self
                .tuning_store
                .read(|document| {
                    let mut state = document.startup_state();
                    state.possibly_stale = self.possibly_stale.gpus(document.last_applied.keys());
                    state
                })
                .map_err(GpuCommandError::from)
                .and_then(|mut state| {
                    state.outcomes = self.boot_state.outcomes(peer)?;
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
            DaemonRequest::ListSteamApps => self.steam_apps(peer).map(DaemonResponse::SteamApps),
            DaemonRequest::GetGames => self.games(peer),
            DaemonRequest::GetRunningGames => Ok(DaemonResponse::RunningGames(
                self.recognition.lock().unwrap().for_uid(peer.uid),
            )),
            DaemonRequest::AddSteamGame(request) => self.add_steam_game(peer, request.app_id),
            DaemonRequest::SetGameProfile(request) => self.set_game_profile(peer, request),
            DaemonRequest::RemoveGame(request) => self.remove_game(peer, request.game),
            DaemonRequest::SetAutomaticSwitching(request) => {
                self.set_automatic_switching(peer, request.enabled)
            }
            DaemonRequest::GetAutomaticSwitchingStatus => Ok(
                DaemonResponse::AutomaticSwitchingStatus(self.switching.status_for(peer)),
            ),
            request => Ok(self.handle(request)),
        };
        if reevaluate {
            self.evaluate_switching();
        }
        response.unwrap_or_else(DaemonResponse::Error)
    }

    /// Queues a reconciliation on each GPU whose automatic switching should change, then pushes
    /// every changed status. Saving a linked profile changes nothing until its next application.
    pub(super) fn evaluate_switching(&self) {
        for gpu_id in self.switching.claim_unsettled() {
            let switching = self.switching.clone();
            let id = gpu_id.clone();
            drop(self.submit_tracked(&gpu_id, move |backend| {
                switching.reconcile(backend, &id);
                switching.publish();
            }));
        }
        self.switching.publish();
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
        self.submit_tracked(gpu_id, operation).wait()
    }

    /// Queues a tuning write like `tracked_write`, without waiting for it.
    fn submit_tracked<T: Send + 'static>(
        &self,
        gpu_id: &str,
        operation: impl FnOnce(&dyn DaemonBackend) -> T + Send + 'static,
    ) -> PendingOperation<T> {
        let store = Arc::clone(&self.tuning_store);
        let uuid = gpu_id.to_string();
        self.submit_write(gpu_id, move |backend| {
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
        let tuning_store = Arc::new(TuningStore::open_with_inventory(
            dependencies.persistent_dir.join(TUNING_STORE_FILE_NAME),
            || dependencies.backend.performance(),
        ));
        let boot_state = Arc::new(super::boot_state::BootState::open(
            &dependencies.boot_dir,
            &dependencies.boot_id,
        ));
        let possibly_stale = Arc::new(PossiblyStale::open(
            dependencies.persistent_dir.join(POSSIBLY_STALE_FILE_NAME),
        ));
        // Fans an earlier daemon left following a curve return to automatic cooling first.
        recover_abandoned_curves(
            dependencies.backend.as_ref(),
            &dependencies.boot_dir,
            &dependencies.boot_id,
        );
        let cooling = Arc::new(CoolingController::open(
            &dependencies.boot_dir,
            &dependencies.boot_id,
            Arc::clone(&dependencies.clock),
        ));
        let recognition = Arc::default();
        let switching = super::switching::Switching::open(
            &dependencies.boot_dir,
            &dependencies.boot_id,
            Arc::clone(&tuning_store),
            Arc::clone(&recognition),
            Arc::clone(&dependencies.users),
        );
        let core = Self {
            dependencies,
            cooling,
            boot_state,
            possibly_stale,
            stopping: Arc::default(),
            startup_operations: Mutex::default(),
            recognition,
            switching,
            queues: GpuOperationQueues::default(),
            tuning_store,
        };

        if let Err(error) = core.start_startup() {
            eprintln!("Startup is suspended: {}", error.message);
        }

        core.scan_running_games();
        core
    }

    /// Stops the daemon. Only a stop while the host shuts down marks this boot clean for the
    /// unclean-shutdown guard; a daemon restart mid-boot records nothing.
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.stop_cooling();
        if !self.dependencies.host_shutting_down.host_shutting_down() {
            return;
        }
        let boot_id = &self.dependencies.boot_id;
        if let Ok(true) = self
            .tuning_store
            .read(|document| document.restored_in(boot_id))
        {
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
                DaemonResponse::Inventory(self.dependencies.backend.inventory())
            }
            DaemonRequest::GetPerformance => DaemonResponse::Performance(self.performance()),
            DaemonRequest::ApplyProfile(request) => {
                DaemonResponse::ProfileApplication(self.apply_profile_settings(request))
            }
            DaemonRequest::GetDaemonStatus => {
                DaemonResponse::DaemonStatus(GpuDaemonStatus::connected(
                    self.dependencies.socket_path.display().to_string(),
                    self.tuning_store.health(),
                ))
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
            DaemonRequest::SetFanSpeed(request) => self.set_one_fan(
                request.gpu_id,
                request.fan_index,
                FanSetting::Fixed {
                    speed_percent: i64::from(request.speed_percent),
                },
            ),
            DaemonRequest::SetFanModeAuto(request) => {
                self.set_one_fan(request.gpu_id, request.fan_index, FanSetting::Automatic {})
            }
            DaemonRequest::PreviewFanCurve(request) => self.preview_fan_curve(request),
            DaemonRequest::ApplyCooling(request) => {
                DaemonResponse::CoolingApplication(self.apply_cooling(request))
            }
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
            DaemonRequest::GetMigrationState
            | DaemonRequest::AdoptMigratedProfile(_)
            | DaemonRequest::DismissMigrationNotice
            | DaemonRequest::GetStartupState
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
            DaemonRequest::ListSteamApps
            | DaemonRequest::GetGames
            | DaemonRequest::GetRunningGames
            | DaemonRequest::AddSteamGame(_)
            | DaemonRequest::SetGameProfile(_)
            | DaemonRequest::RemoveGame(_)
            | DaemonRequest::SetAutomaticSwitching(_)
            | DaemonRequest::GetAutomaticSwitchingStatus => {
                DaemonResponse::Error(GpuCommandError::new(
                    GpuCommandErrorCode::NoPermission,
                    "Games need a connection that identifies the caller.",
                ))
            }
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

    /// The GPUs' readings, with the daemon's judgement of the fan curves each can use and Prism's
    /// control of each identified GPU's fans.
    fn performance(&self) -> GpuPerformanceResponse {
        let mut performance = self.dependencies.backend.performance();
        let identified: Vec<_> = performance
            .gpus
            .iter()
            .map(|gpu| {
                gpu.stable_id
                    .as_deref()
                    .filter(|uuid| verified_gpu(&performance, uuid).is_ok())
                    .map(str::to_string)
            })
            .collect();
        for (gpu, uuid) in performance.gpus.iter_mut().zip(identified) {
            gpu.cooling.curve_support = Some(fan_curve::curve_support(&gpu.cooling));
            gpu.cooling.control = uuid.map(|uuid| self.cooling.control(&uuid));
        }
        performance
    }

    /// Applies cooling on the GPU's queue, so it never races a curve being followed.
    fn apply_cooling(&self, request: ApplyCoolingRequest) -> crate::cooling::CoolingApplication {
        let cooling = Arc::clone(&self.cooling);
        self.write(&request.gpu_id.clone(), move |backend| {
            cooling.apply(backend, request)
        })
    }

    /// A single fan's live change, which replaces only that fan's applied setting. A GPU without a
    /// verified identity can't follow curves, so its fan is written directly.
    fn set_one_fan(&self, gpu_id: String, fan_index: u32, setting: FanSetting) -> DaemonResponse {
        let Some(uuid) = uuid_of(&self.dependencies.backend.performance(), &gpu_id) else {
            return self.live_command(gpu_id.clone(), move |backend| match setting {
                FanSetting::Fixed { speed_percent } => {
                    backend.set_fan_speed(crate::gpu::SetGpuFanSpeedRequest {
                        gpu_id,
                        fan_index,
                        speed_percent: speed_percent as u32,
                    })
                }
                _ => backend
                    .set_fan_mode_auto(crate::gpu::SetGpuFanModeAutoRequest { gpu_id, fan_index }),
            });
        };
        let application = self.apply_cooling(ApplyCoolingRequest {
            expected_revision: None,
            gpu_id: uuid,
            configuration: CoolingConfiguration::PerFan {
                fans: vec![FanConfiguration { fan_index, setting }],
            },
        });
        let outcome = application
            .fans
            .into_iter()
            .next()
            .map(|fan| fan.outcome)
            .unwrap_or(application.outcome);
        DaemonResponse::Command(DaemonCommandResponse {
            success: outcome.status == TuningOutcomeStatus::Succeeded,
            error: outcome.error,
        })
    }

    /// Follows every running curve once, on each GPU's queue, without waiting. A GPU whose last
    /// evaluation hasn't finished is skipped rather than queued twice.
    pub(super) fn follow_curves(&self) {
        for uuid in self.cooling.claim_due() {
            let cooling = Arc::clone(&self.cooling);
            let id = uuid.clone();
            drop(self.submit_write(&uuid, move |backend| cooling.follow(backend, &id)));
        }
    }

    /// Starts the loop that follows applied curves every 500 ms.
    pub fn start_cooling(self: &Arc<Self>) -> std::io::Result<()> {
        let core = Arc::downgrade(self);
        std::thread::Builder::new()
            .name("prismd-cooling".into())
            .spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_millis(EVALUATION_INTERVAL_MS));
                let Some(core) = core.upgrade() else {
                    break;
                };
                if core.stopping.load(Ordering::SeqCst) {
                    break;
                }
                core.follow_curves();
            })?;
        Ok(())
    }

    /// Whether the curve loop, and every curve it follows, has made progress within the window.
    pub fn cooling_is_live(&self, window_ms: u64) -> bool {
        self.cooling.is_live(window_ms)
    }

    /// Stops following curves and returns their fans to automatic cooling. A GPU whose queue
    /// doesn't free up in time is restored directly, since its curve can no longer make progress.
    fn stop_cooling(&self) {
        self.cooling.begin_stopping();
        let pending: Vec<_> = self
            .cooling
            .running_gpus()
            .into_iter()
            .map(|uuid| {
                let cooling = Arc::clone(&self.cooling);
                let id = uuid.clone();
                let operation =
                    self.submit_write(&uuid, move |backend| cooling.stop_following(backend, &id));
                (uuid, operation)
            })
            .collect();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        for (uuid, operation) in pending {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if operation.wait_timeout(remaining).is_none() {
                self.cooling
                    .stop_following_stuck(self.dependencies.backend.as_ref(), &uuid);
            }
        }
    }

    /// Checks a draft curve against the GPU's current limits. A read: it never waits on, or
    /// adds to, the GPU's operation queue.
    fn preview_fan_curve(&self, request: PreviewFanCurveRequest) -> DaemonResponse {
        let performance = self.dependencies.backend.performance();
        match performance.gpus.iter().find(|gpu| gpu.id == request.gpu_id) {
            Some(gpu) => DaemonResponse::FanCurvePreview(FanCurvePreview {
                problems: fan_curve::validate(&gpu.cooling, &request.points),
            }),
            None => DaemonResponse::Error(GpuCommandError::new(
                GpuCommandErrorCode::NotFound,
                "The GPU is unavailable. Refresh the GPU readings and try again.",
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
        mark_not_recorded([&mut self.graphics, &mut self.memory]);
    }
}

impl ManualWriteResponse for ProfileApplicationResponse {
    fn not_recorded(&mut self) {
        mark_not_recorded([&mut self.power, &mut self.graphics, &mut self.memory]);
    }
}

fn mark_not_recorded<const N: usize>(outcomes: [&mut Option<TuningOutcome>; N]) {
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
