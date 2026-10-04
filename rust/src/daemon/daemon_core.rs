use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::drivers::{request_host_reboot, DriverService};
use crate::gpu::{
    ApplyGpuConfigRequest, GpuApplyRecord, GpuCommandError, GpuCommandErrorCode, GpuDaemonStatus,
    GpuInventoryResponse, GpuMutationResponse, PersistedGpuState,
};
use crate::protocol::{DaemonCommandResponse, DaemonRequest, DaemonResponse};

use super::backend::DaemonBackend;
use super::host::{Clock, HostShutdownSignal};
use super::queue::{GpuOperationQueues, PendingOperation};
use super::tuning::{apply_live_offsets, apply_profile, reset_live_offsets};
use super::tuning_store::{TuningStore, TUNING_STORE_FILE_NAME};

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
    dependencies: CoreDependencies,
    queues: GpuOperationQueues,
    tuning_store: TuningStore,
    legacy_configs: Arc<LegacyConfigStore>,
}

impl DaemonCore {
    /// Loads saved state and reapplies legacy saved configurations, as every daemon start does.
    pub fn start(dependencies: CoreDependencies) -> Self {
        let legacy_configs = Arc::new(LegacyConfigStore::load(
            dependencies.persistent_dir.join(LEGACY_STATE_FILE_NAME),
        ));
        let tuning_store =
            TuningStore::open(dependencies.persistent_dir.join(TUNING_STORE_FILE_NAME));
        let core = Self {
            dependencies,
            queues: GpuOperationQueues::default(),
            tuning_store,
            legacy_configs,
        };

        let outcome = core.reapply_legacy_configs();
        if let (_, Err(error)) = core.legacy_configs.save() {
            eprintln!("failed to save Prism daemon state on startup: {error}");
        }
        if let Err(error) = outcome {
            eprintln!(
                "failed to reapply one or more saved GPU settings on startup: {}",
                error.message
            );
        }

        core
    }

    pub fn handle(&self, request: DaemonRequest) -> DaemonResponse {
        let drivers = &self.dependencies.drivers;
        match request {
            DaemonRequest::GetInventory => {
                DaemonResponse::Inventory(self.inventory(&self.legacy_configs.snapshot()))
            }
            DaemonRequest::GetPerformance => {
                DaemonResponse::Performance(self.dependencies.backend.performance())
            }
            DaemonRequest::ApplyProfile(request) => DaemonResponse::ProfileApplication(
                self.write(&request.gpu_id.clone(), move |backend| {
                    apply_profile(backend, request)
                }),
            ),
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
            DaemonRequest::SetPowerLimit(request) => self
                .live_command(request.gpu_id.clone(), move |backend| {
                    backend.set_power_limit(request)
                }),
            DaemonRequest::ResetPowerLimit(request) => self
                .live_command(request.gpu_id.clone(), move |backend| {
                    backend.reset_power_limit(request)
                }),
            DaemonRequest::SetFanSpeed(request) => self
                .live_command(request.gpu_id.clone(), move |backend| {
                    backend.set_fan_speed(request)
                }),
            DaemonRequest::SetFanModeAuto(request) => self
                .live_command(request.gpu_id.clone(), move |backend| {
                    backend.set_fan_mode_auto(request)
                }),
            DaemonRequest::SetClockOffsets(request) => {
                DaemonResponse::ClockOffsets(self.write(&request.gpu_id.clone(), move |backend| {
                    apply_live_offsets(backend, request)
                }))
            }
            DaemonRequest::ResetClockOffsets(request) => {
                DaemonResponse::ClockOffsets(self.write(&request.gpu_id.clone(), move |backend| {
                    reset_live_offsets(backend, request)
                }))
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
