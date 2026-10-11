use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use prism_drivers::{DriverService, request_host_reboot};
use prism_gpu::fan_curve::{self, FanCurvePreview, PreviewFanCurveRequest};
use prism_gpu::{
    GpuCommandError, GpuCommandErrorCode, GpuPerformanceResponse, TuningOutcome,
    TuningOutcomeStatus,
};
use prism_protocol::{
    ApplyGpuProfileRequest, CorrectAndApplyProfileRequest, CorrectedProfileApplication,
    DaemonCommandResponse, DaemonEvent, DaemonRequest, DaemonResponse, DaemonSnapshot,
    GpuCoolingControl, GpuDaemonStatus, GpuTuningProfile, GpuTuningState, HelloRequest,
    Incompatible, ProfileApplicationResponse, ProfileSetting, ProfileSettingKey, ProfileSettings,
    ProtocolVersion, SaveProfileAsNewRequest, SaveToCurrentProfileRequest, Welcome,
};

use super::backend::DaemonBackend;
use super::cooling::{
    CoolingApplicationSource, CoolingController, EVALUATION_INTERVAL_MS, recover_abandoned_curves,
};
use super::host::{Clock, HostShutdownSignal, UserDirectory};
use super::possibly_stale::{POSSIBLY_STALE_FILE_NAME, PossiblyStale};
use super::profiles::{capture_settings, new_profile_id, profile_name, verified_gpu};
use super::queue::{GpuOperationQueues, PendingOperation};
use super::tuning::{TuningOrigin, TuningRequest, apply_profile};
use super::tuning_store::{TUNING_STORE_FILE_NAME, TuningDocument, TuningStore};
use super::{DaemonSession, SessionPeer, SessionPolicy};

/// How often subscribed sessions get telemetry.
pub const TELEMETRY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

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
    /// The GPUs, by id, in the latest inventory sessions were given.
    inventory_ids: Mutex<Vec<String>>,
}

/// Why prismd refused a session connection's hello.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelloRefused {
    /// The caller speaks a protocol this daemon can't.
    Incompatible(Incompatible),
    /// The caller couldn't be identified, or didn't start with a hello.
    Unidentified(GpuCommandError),
}

impl From<HelloRefused> for DaemonResponse {
    fn from(refused: HelloRefused) -> Self {
        match refused {
            HelloRefused::Incompatible(incompatible) => Self::Incompatible(incompatible),
            HelloRefused::Unidentified(error) => Self::Error(error),
        }
    }
}

/// A session prismd welcomed, with the state it starts from.
pub struct Welcomed {
    pub welcome: Welcome,
    pub snapshot: DaemonSnapshot,
    pub session: DaemonSession,
}

impl DaemonCore {
    /// Answers a session connection's hello: a caller speaking a compatible protocol gets a
    /// session and its snapshot, and any other is refused with this daemon's versions. A caller
    /// the policy doesn't allow to tune gets a read-only session, and the welcome says why.
    pub fn hello(
        &self,
        peer: SessionPeer,
        policy: &SessionPolicy,
        hello: HelloRequest,
    ) -> Result<Welcomed, HelloRefused> {
        let prismd_version = env!("CARGO_PKG_VERSION").to_owned();
        if !ProtocolVersion::CURRENT.is_compatible_with(hello.protocol) {
            return Err(HelloRefused::Incompatible(Incompatible {
                protocol: ProtocolVersion::CURRENT,
                prismd_version,
            }));
        }
        let tuning = policy.allow_tuning(&peer);
        let authorization = policy.authorization(&peer);
        self.switching.opened(&peer);
        // Subscribed before the snapshot is read, so no change between the two is lost: one
        // already in the snapshot arrives again as an event with the same full value.
        let session = self.tuning_store.subscribe(peer, policy.clone());
        Ok(Welcomed {
            welcome: Welcome {
                protocol: ProtocolVersion::CURRENT,
                prismd_version,
                can_tune: tuning.is_ok(),
                reason: tuning.err().map(|refused| refused.message),
                authorization: Some(authorization),
            },
            snapshot: self.snapshot(session.peer()),
            session,
        })
    }

    /// The daemon state a caller's session starts from.
    fn snapshot(&self, peer: &SessionPeer) -> DaemonSnapshot {
        let inventory = self.dependencies.backend.inventory();
        *self.inventory_ids.lock().unwrap() = gpu_ids(&inventory.gpus, |gpu| &gpu.id);
        // Each read on its own, so no lock is held into the next: switching reads recognition.
        let running_games = self.recognition.lock().unwrap().for_uid(peer.uid);
        let automatic_switching = self.switching.status_for(peer);
        let performance = self.observed_performance(Some(peer));
        let tuning_states = self.tuning_states_of(&performance);
        // The session knows these now; the tick pushes them only once they change.
        self.tuning_store.known_tuning(&tuning_states);
        DaemonSnapshot {
            inventory,
            store_health: self.tuning_store.health(),
            tuning_states,
            cooling_controls: cooling_controls(performance),
            running_games,
            automatic_switching,
        }
    }

    /// Handles one of the session's requests, once its caller is allowed to make it.
    pub fn handle_session(
        &self,
        session: &DaemonSession,
        request: DaemonRequest,
    ) -> DaemonResponse {
        if let Err(refused) = session.authorize(&request) {
            return DaemonResponse::Error(refused);
        }
        match request {
            DaemonRequest::SetEditHolds(request) => {
                session.set_edit_holds(request.gpu_ids);
                self.evaluate_switching();
                startup_command()
            }
            DaemonRequest::SubscribeTelemetry => {
                session.subscribe_telemetry();
                startup_command()
            }
            request => self.handle_for_peer(session.peer(), request),
        }
    }

    /// Runs one tick of the telemetry loop: while any session subscribed, reads every GPU once,
    /// pushes each GPU's tuning state that changed, even outside Prism, and pushes the same sample
    /// to each subscriber, after the new inventory when GPUs arrived or departed. With no
    /// subscriber, the GPUs aren't read.
    pub fn sample_telemetry(&self) {
        if !self.tuning_store.telemetry_subscribed() {
            return;
        }
        let store = &self.tuning_store;
        // Read while holding the pushes, so a write's push never lands between this read and
        // pushing what it found.
        let performance = store.push_tuning(|pushes| {
            let performance = self.observed_performance(None);
            // A GPU's arrival comes before its tuning state and readings.
            self.publish_inventory_change(&performance);
            let states = store.read_for_tuning(|document| {
                performance
                    .gpus
                    .iter()
                    .filter_map(|gpu| verified_gpu(&performance, gpu.stable_id.as_deref()?).ok())
                    .map(|gpu| document.tuning_state(gpu))
                    .collect::<Vec<_>>()
            });
            for state in states {
                pushes.push(state, None);
            }
            performance
        });
        self.tuning_store
            .publish_telemetry(DaemonEvent::TelemetrySampled(performance));
    }

    /// Pushes the inventory when GPUs arrived or departed. The readings only hint at a change; the
    /// inventory itself decides, so a reading that briefly misses a GPU pushes nothing.
    fn publish_inventory_change(&self, performance: &GpuPerformanceResponse) {
        if *self.inventory_ids.lock().unwrap() == gpu_ids(&performance.gpus, |gpu| &gpu.id) {
            return;
        }
        let inventory = self.dependencies.backend.inventory();
        let ids = gpu_ids(&inventory.gpus, |gpu| &gpu.id);
        let mut known = self.inventory_ids.lock().unwrap();
        if *known != ids {
            *known = ids;
            drop(known);
            self.tuning_store
                .publish(DaemonEvent::InventoryChanged(inventory));
        }
    }

    /// Starts the loop that pushes telemetry to subscribed sessions once a second. The curve
    /// loop reads the GPUs on its own schedule.
    pub fn start_telemetry(self: &Arc<Self>) -> std::io::Result<()> {
        let core = Arc::downgrade(self);
        std::thread::Builder::new()
            .name("prismd-telemetry".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(TELEMETRY_INTERVAL);
                    let Some(core) = core.upgrade() else {
                        break;
                    };
                    if core.stopping.load(Ordering::SeqCst) {
                        break;
                    }
                    core.sample_telemetry();
                }
            })?;
        Ok(())
    }

    /// The caller's view of each identified GPU's tuning state.
    fn tuning_states(&self, peer: &SessionPeer) -> Vec<GpuTuningState> {
        self.tuning_states_of(&self.observed_performance(Some(peer)))
    }

    /// The tuning state of each GPU the readings identify.
    fn tuning_states_of(&self, performance: &GpuPerformanceResponse) -> Vec<GpuTuningState> {
        self.tuning_store.read_for_tuning(|document| {
            performance
                .gpus
                .iter()
                .filter_map(|gpu| verified_gpu(performance, gpu.stable_id.as_deref()?).ok())
                .map(|gpu| document.tuning_state(gpu))
                .collect()
        })
    }

    /// Handles a request from an identified caller once the policy allows it.
    pub(super) fn handle_authorized(
        &self,
        peer: &SessionPeer,
        policy: &SessionPolicy,
        request: DaemonRequest,
    ) -> DaemonResponse {
        match policy.authorize(peer, &request) {
            Ok(()) => self.handle_for_peer(peer, request),
            Err(refused) => DaemonResponse::Error(refused),
        }
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
                | DaemonRequest::SetGpuSwitchingPaused(_)
                | DaemonRequest::DeleteProfile(_)
                | DaemonRequest::RenameProfile(_)
        );
        let response = match request {
            DaemonRequest::ApplyProfile(request) => self
                .apply_staged(request, Some(peer.clone()))
                .map(DaemonResponse::ProfileApplication),
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
                Ok(DaemonResponse::TuningStates(self.tuning_states(peer)))
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
            DaemonRequest::ListSteamApps => self.steam_apps(peer).map(DaemonResponse::SteamApps),
            DaemonRequest::ListRunningPrograms => {
                super::programs::list(&self.dependencies.host_root, peer)
                    .map(DaemonResponse::RunningPrograms)
                    .map_err(|error| {
                        GpuCommandError::new(
                            GpuCommandErrorCode::Io,
                            format!("Could not list running programs: {error}"),
                        )
                    })
            }
            DaemonRequest::AddProgram(request) => self.add_program(peer, request.path),
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
            DaemonRequest::RetryAutomaticSwitching(request) => {
                let switching = self.switching.clone();
                let gpu_id = request.gpu_id;
                self.tracked_write(&gpu_id.clone(), move |backend| {
                    switching.retry(backend, &gpu_id)
                });
                Ok(DaemonResponse::AutomaticSwitchingStatus(
                    self.switching.status_for(peer),
                ))
            }
            DaemonRequest::GetAutomaticSwitchingStatus => Ok(
                DaemonResponse::AutomaticSwitchingStatus(self.switching.status_for(peer)),
            ),
            DaemonRequest::SetGpuSwitchingPaused(request) => {
                let switching = self.switching.clone();
                let gpu_id = request.gpu_id.clone();
                self.tracked_write(&gpu_id, move |backend| {
                    switching.set_paused(backend, &request.gpu_id, request.paused)
                })
                .map(|()| startup_command())
            }
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
        for gpu_id in self
            .switching
            .claim_unsettled(self.dependencies.backend.as_ref())
        {
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
        let cooling = Arc::clone(&self.cooling);
        self.tracked_write(&uuid.clone(), move |backend| {
            let performance = backend.performance();
            let mut gpu = verified_gpu(&performance, &uuid)?.clone();
            gpu.cooling.control = Some(cooling.control(&uuid));
            store.try_update(|document| {
                document.check_revision(&gpu, &revision)?;
                let settings =
                    capture_settings(&gpu, document.last_applied.get(&uuid), TuningOrigin::Manual)?;
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
        let target = self
            .tuning_store
            .read(|document| document.profile_of(peer, &request.profile_id))??;
        // Corrections wait on the same queue before applying. Reserve precedence now, while
        // recording the override only when apply_saved_profile starts the manual write.
        let _pending = self.switching.manual_requested(&target.gpu_id);
        let peer_owned = peer.clone();
        let corrections = request.corrections;
        let backend = Arc::clone(&self.dependencies.backend);
        let profile_id = request.profile_id.clone();
        let profile = self.update_profile(peer, &request.profile_id, move |document| {
            let profile = document.profile_of(&peer_owned, &profile_id)?;
            for correction in &corrections {
                if let prism_protocol::ProfileRangeCorrection::Cooling { saved_value, .. } = correction {
                    let performance = backend.performance();
                    let gpu = verified_gpu(&performance, &profile.gpu_id)?;
                    if super::cooling_profiles::correction(gpu, saved_value).as_ref() != Some(correction) {
                        return Err(GpuCommandError::new(prism_gpu::GpuCommandErrorCode::InvalidArgument,
                            "Cooling limits changed or the correction is invalid. Review the profile again. No GPU settings were changed."));
                    }
                }
            }
            document.correct_profile(&peer_owned, &profile_id, &corrections)
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
        let cooling = Arc::clone(&self.cooling);
        let peer = peer.clone();
        self.manual_write(&profile.gpu_id, move |backend| {
            // A profile can be deleted or edited while this application waits on the GPU's queue.
            let profile = store.read(|document| document.profile_of(&peer, &profile.id))??;
            Ok(apply_recorded_profile(
                backend,
                &cooling,
                &store,
                &stale,
                TuningRequest::new(profile.gpu_id, profile.settings),
                Some(profile.id),
                TuningOrigin::Manual,
                Some(peer),
            ))
        })
    }

    /// Applies a GPU's staged changes as one manual write, unless the GPU's tuning state changed
    /// since they were made: then nothing is written, so the caller can review it first.
    fn apply_staged(
        &self,
        request: ApplyGpuProfileRequest,
        peer: Option<SessionPeer>,
    ) -> Result<ProfileApplicationResponse, GpuCommandError> {
        let store = Arc::clone(&self.tuning_store);
        let stale = Arc::clone(&self.possibly_stale);
        let cooling = Arc::clone(&self.cooling);
        let pending = self.switching.manual_requested(&request.gpu_id);
        self.tracked_write(&request.gpu_id.clone(), move |backend| {
            // A GPU whose state can't be read is refused by the application itself.
            if let Some(state) = tuning_state(backend, &store, &cooling, &request.gpu_id)
                && state.revision != request.expected_revision
            {
                return Err(GpuCommandError::new(
                    GpuCommandErrorCode::StaleRevision,
                    "The GPU's tuning values changed since these changes were made. Review them, then apply again. No GPU settings were changed.",
                ));
            }
            pending.started();
            Ok(apply_recorded_profile(
                backend,
                &cooling,
                &store,
                &stale,
                TuningRequest {
                    gpu_id: request.gpu_id,
                    settings: request.settings,
                    fan_indexes: request.fan_indexes,
                },
                None,
                TuningOrigin::Manual,
                peer,
            ))
        })
    }

    fn manual_write<T: Send + 'static>(
        &self,
        gpu_id: &str,
        operation: impl FnOnce(&dyn DaemonBackend) -> T + Send + 'static,
    ) -> T {
        let pending = self.switching.manual_requested(gpu_id);
        self.tracked_write(gpu_id, move |backend| {
            pending.started();
            operation(backend)
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
        let cooling = Arc::clone(&self.cooling);
        self.submit_write(gpu_id, move |backend| {
            let before = tuning_state(backend, &store, &cooling, &uuid);
            let result = operation(backend);
            store.push_tuning(|pushes| {
                if let Some(after) = tuning_state(backend, &store, &cooling, &uuid) {
                    pushes.push(after, before.as_ref());
                }
            });
            result
        })
    }

    /// Loads host state and reserves startup operations before serving any manual requests.
    pub fn start(dependencies: CoreDependencies) -> Self {
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
            Arc::clone(&tuning_store),
        ));
        let recognition = Arc::default();
        let switching = super::switching::Switching::open(
            &dependencies.boot_dir,
            &dependencies.boot_id,
            Arc::clone(&tuning_store),
            Arc::clone(&cooling),
            Arc::clone(&recognition),
            Arc::clone(&dependencies.users),
            Arc::clone(&dependencies.clock),
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
            inventory_ids: Mutex::default(),
        };

        core.resume_interrupted_curves();
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
            && let Err(error) = self
                .tuning_store
                .update(|document| document.restore_attempt = None)
        {
            eprintln!(
                "failed to record a clean shutdown after restoring last applied tuning values: {error}"
            );
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
            DaemonRequest::Hello(_)
            | DaemonRequest::SetEditHolds(_)
            | DaemonRequest::SubscribeTelemetry => DaemonResponse::Error(GpuCommandError::new(
                GpuCommandErrorCode::InvalidArgument,
                "This request requires a framed session connection",
            )),
            DaemonRequest::GetInventory => {
                DaemonResponse::Inventory(self.dependencies.backend.inventory())
            }
            DaemonRequest::ApplyProfile(request) => self
                .apply_staged(request, None)
                .map_or_else(DaemonResponse::Error, DaemonResponse::ProfileApplication),
            DaemonRequest::GetDaemonStatus => {
                DaemonResponse::DaemonStatus(GpuDaemonStatus::connected(
                    self.dependencies.socket_path.display().to_string(),
                    self.tuning_store.health(),
                ))
            }
            DaemonRequest::PreviewFanCurve(request) => self.preview_fan_curve(request),
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
            | DaemonRequest::CorrectAndApplyProfile(_) => {
                DaemonResponse::Error(GpuCommandError::new(
                    GpuCommandErrorCode::NoPermission,
                    "GPU tuning profiles need a connection that identifies the caller.",
                ))
            }
            DaemonRequest::ListSteamApps
            | DaemonRequest::ListRunningPrograms
            | DaemonRequest::AddProgram(_)
            | DaemonRequest::GetGames
            | DaemonRequest::GetRunningGames
            | DaemonRequest::AddSteamGame(_)
            | DaemonRequest::SetGameProfile(_)
            | DaemonRequest::RemoveGame(_)
            | DaemonRequest::SetAutomaticSwitching(_)
            | DaemonRequest::RetryAutomaticSwitching(_)
            | DaemonRequest::SetGpuSwitchingPaused(_)
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

    /// The GPUs' readings, with the daemon's judgment of the fan curves each can use and Prism's
    /// control of each identified GPU's fans, including the caller's remembered curves when
    /// there is a caller.
    pub(super) fn observed_performance(
        &self,
        peer: Option<&SessionPeer>,
    ) -> GpuPerformanceResponse {
        let mut performance = self.dependencies.backend.performance();
        let remembered = peer
            .and_then(|peer| {
                self.tuning_store
                    .read(|document| {
                        document
                            .user(peer.uid, &peer.username)
                            .map(|user| user.remembered_curves.clone())
                    })
                    .ok()
                    .flatten()
            })
            .unwrap_or_default();
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
            gpu.cooling.control = uuid.map(|uuid| {
                let mut control = self.cooling.control(&uuid);
                control.remembered_curve = remembered.get(&uuid).cloned();
                control
            });
        }
        performance
    }

    /// Applies cooling on the GPU's queue, as a game's temporary application or its restoration
    /// does, never as a manual change.
    #[cfg(test)]
    pub(super) fn apply_cooling(
        &self,
        request: prism_gpu::cooling::ApplyCoolingRequest,
    ) -> prism_gpu::cooling::CoolingApplication {
        let cooling = Arc::clone(&self.cooling);
        self.tracked_write(&request.gpu_id.clone(), move |backend| {
            cooling.apply(backend, request, CoolingApplicationSource::NonManual)
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

    /// Queues the resumption of curves an earlier daemon of this boot was following when it
    /// stopped, ahead of any other write to their GPUs. Readiness waits for them.
    fn resume_interrupted_curves(&self) {
        for uuid in self.cooling.interrupted_gpus() {
            let cooling = Arc::clone(&self.cooling);
            let id = uuid.clone();
            let pending = self.submit_write(&uuid, move |backend| {
                cooling.resume_interrupted(backend, &id)
            });
            self.startup_operations.lock().unwrap().push(pending);
        }
    }

    /// Records whether curve control has stalled, for whoever recovers after the daemon stops.
    pub fn record_cooling_stall(&self, stalled: bool) {
        self.cooling.record_stall(stalled);
    }

    /// Starts the loop that follows applied curves every 500 ms.
    pub fn start_cooling(self: &Arc<Self>) -> std::io::Result<()> {
        let core = Arc::downgrade(self);
        std::thread::Builder::new()
            .name("prismd-cooling".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(EVALUATION_INTERVAL_MS));
                    let Some(core) = core.upgrade() else {
                        break;
                    };
                    if core.stopping.load(Ordering::SeqCst) {
                        break;
                    }
                    core.follow_curves();
                }
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
            .gpus_to_stop()
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
                problems: match request.fan_index {
                    Some(index) => fan_curve::validate_for(&gpu.cooling, &[index], &request.points),
                    None => fan_curve::validate(&gpu.cooling, &request.points),
                },
                support: request
                    .fan_index
                    .map(|index| fan_curve::curve_support_for(&gpu.cooling, &[index])),
            }),
            None => DaemonResponse::Error(GpuCommandError::new(
                GpuCommandErrorCode::NotFound,
                "The GPU is unavailable. Refresh the GPU readings and try again.",
            )),
        }
    }

    /// Queues a tuning write on the GPU's operation queue. Every operation on a GPU's fans runs
    /// there, so when one changed Prism's control of them, it pushes the new control to every
    /// session.
    pub(super) fn submit_write<T: Send + 'static>(
        &self,
        gpu_id: &str,
        operation: impl FnOnce(&dyn DaemonBackend) -> T + Send + 'static,
    ) -> PendingOperation<T> {
        let backend = Arc::clone(&self.dependencies.backend);
        let cooling = Arc::clone(&self.cooling);
        let uuid = gpu_id.to_string();
        self.queues.submit(gpu_id, move || {
            let before = cooling.revision(&uuid);
            let result = operation(backend.as_ref());
            if cooling.revision(&uuid) != before {
                cooling.publish(&uuid);
            }
            result
        })
    }
}

/// Applies settings and records successful manual instructions and the resulting current profile.
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_recorded_profile(
    backend: &dyn DaemonBackend,
    cooling: &CoolingController,
    store: &TuningStore,
    stale: &PossiblyStale,
    request: TuningRequest,
    profile_id: Option<String>,
    origin: TuningOrigin,
    peer: Option<SessionPeer>,
) -> ProfileApplicationResponse {
    let uuid = request.gpu_id.clone();
    let settings = request.settings.clone();
    let source = match (origin, peer) {
        (TuningOrigin::Manual, Some(peer)) => CoolingApplicationSource::Manual(peer),
        (TuningOrigin::Manual, None) => CoolingApplicationSource::UnidentifiedManual,
        (TuningOrigin::TemporaryAutomatic, _) => CoolingApplicationSource::NonManual,
    };
    let mut response = apply_profile(backend, cooling, request, origin, source);
    if origin == TuningOrigin::TemporaryAutomatic {
        return response;
    }
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
    let cooling_succeeded = response
        .cooling
        .as_ref()
        .is_some_and(|c| c.outcome.status == TuningOutcomeStatus::Succeeded);
    let cooling_changed = response.cooling.as_ref().is_some_and(|c| {
        c.fans
            .iter()
            .any(|f| f.outcome.status == TuningOutcomeStatus::Succeeded)
    });
    let complete =
        applied.len() == included.count() && (settings.cooling.is_none() || cooling_succeeded);
    let current = match profile_id {
        Some(id) if complete => CurrentProfileChange::Become(id),
        Some(_) if !applied.is_empty() || cooling_changed => CurrentProfileChange::Clear,
        _ => CurrentProfileChange::Keep,
    };
    let recorded = record_manual(store, stale, &uuid, &applied, |document| {
        if cooling_succeeded {
            document
                .last_applied
                .entry(uuid.clone())
                .or_default()
                .cooling = settings.cooling.clone();
        }
        match current {
            CurrentProfileChange::Become(id) => {
                document.current_profiles.insert(uuid.clone(), id);
            }
            CurrentProfileChange::Clear => {
                document.current_profiles.remove(&uuid);
            }
            CurrentProfileChange::Keep => {}
        }
    });
    if cooling_succeeded {
        stale.cooling_recorded(&uuid, recorded);
    }
    if !recorded {
        not_recorded(&mut response);
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

/// Notes on each setting of a manual application that reached the GPU that its last applied
/// tuning value couldn't be recorded; it still reports as succeeded.
fn not_recorded(response: &mut ProfileApplicationResponse) {
    let numeric = [
        &mut response.power,
        &mut response.graphics,
        &mut response.memory,
    ];
    let cooling = response
        .cooling
        .as_mut()
        .map(|cooling| &mut cooling.outcome);
    for outcome in numeric.into_iter().flatten().chain(cooling) {
        if outcome.status == TuningOutcomeStatus::Succeeded {
            outcome.error = Some(NOT_RECORDED.into());
        }
    }
}

/// The GPU's tuning state from fresh readings, unless the GPU can't be identified.
pub(super) fn tuning_state(
    backend: &dyn DaemonBackend,
    store: &TuningStore,
    cooling: &CoolingController,
    uuid: &str,
) -> Option<GpuTuningState> {
    let performance = backend.performance();
    let mut gpu = verified_gpu(&performance, uuid).ok()?.clone();
    gpu.cooling.control = Some(cooling.control(uuid));
    Some(store.read_for_tuning(|document| document.tuning_state(&gpu)))
}

/// Prism's control of the fans of each GPU the readings identify, as the readings' caller sees it.
fn cooling_controls(performance: GpuPerformanceResponse) -> Vec<GpuCoolingControl> {
    performance
        .gpus
        .into_iter()
        .filter_map(|gpu| Some(GpuCoolingControl::of(gpu.stable_id?, gpu.cooling.control?)))
        .collect()
}

/// Each GPU's id, in order.
fn gpu_ids<T>(gpus: &[T], id: impl Fn(&T) -> &String) -> Vec<String> {
    gpus.iter().map(|gpu| id(gpu).clone()).collect()
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
