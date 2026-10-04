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
use super::queue::GpuOperationQueues;
use super::tuning::{apply_live_offsets, apply_profile, reset_live_offsets};

const SAVED_CONFIG_FILE_NAME: &str = "state.json";

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
    saved_configs: SavedConfigStore,
}

impl DaemonCore {
    /// Loads saved state and reapplies legacy saved configurations, as every daemon start does.
    pub fn start(dependencies: CoreDependencies) -> Self {
        let saved_configs =
            SavedConfigStore::load(dependencies.persistent_dir.join(SAVED_CONFIG_FILE_NAME));
        let core = Self {
            dependencies,
            queues: GpuOperationQueues::default(),
            saved_configs,
        };

        let results = core.reapply_saved_configs();
        let now = core.now();
        let (outcome, _, saved) = core
            .saved_configs
            .update(|state| record_reapplication(state, results, now));
        if let Err(error) = saved {
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
                DaemonResponse::Inventory(self.inventory(&self.saved_configs.snapshot()))
            }
            DaemonRequest::GetPerformance => {
                DaemonResponse::Performance(self.dependencies.backend.performance())
            }
            DaemonRequest::ApplyProfile(request) => {
                let gpu_id = request.gpu_id.clone();
                DaemonResponse::ProfileApplication(
                    self.write(&gpu_id, move |backend| apply_profile(backend, request)),
                )
            }
            DaemonRequest::GetDaemonStatus => DaemonResponse::DaemonStatus(
                GpuDaemonStatus::connected(self.dependencies.socket_path.display().to_string()),
            ),
            DaemonRequest::ApplyConfig(request) => {
                DaemonResponse::Mutation(if request.reapply_saved {
                    self.reapply_saved_configs_and_save()
                } else {
                    self.apply_saved_config(request)
                })
            }
            DaemonRequest::ResetConfig(request) => {
                DaemonResponse::Mutation(self.reset_saved_config(request.gpu_id))
            }
            DaemonRequest::ReapplySavedConfig => {
                DaemonResponse::Mutation(self.reapply_saved_configs_and_save())
            }
            DaemonRequest::SetPowerLimit(request) => {
                let gpu_id = request.gpu_id.clone();
                DaemonResponse::Command(command_result(
                    self.write(&gpu_id, move |backend| backend.set_power_limit(request)),
                ))
            }
            DaemonRequest::ResetPowerLimit(request) => {
                let gpu_id = request.gpu_id.clone();
                DaemonResponse::Command(command_result(
                    self.write(&gpu_id, move |backend| backend.reset_power_limit(request)),
                ))
            }
            DaemonRequest::SetFanSpeed(request) => {
                let gpu_id = request.gpu_id.clone();
                DaemonResponse::Command(command_result(
                    self.write(&gpu_id, move |backend| backend.set_fan_speed(request)),
                ))
            }
            DaemonRequest::SetFanModeAuto(request) => {
                let gpu_id = request.gpu_id.clone();
                DaemonResponse::Command(command_result(
                    self.write(&gpu_id, move |backend| backend.set_fan_mode_auto(request)),
                ))
            }
            DaemonRequest::SetClockOffsets(request) => {
                let gpu_id = request.gpu_id.clone();
                DaemonResponse::ClockOffsets(
                    self.write(&gpu_id, move |backend| apply_live_offsets(backend, request)),
                )
            }
            DaemonRequest::ResetClockOffsets(request) => {
                let gpu_id = request.gpu_id.clone();
                DaemonResponse::ClockOffsets(
                    self.write(&gpu_id, move |backend| reset_live_offsets(backend, request)),
                )
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

    /// Runs a tuning write on the GPU's operation queue and waits for its result.
    fn write<T: Send + 'static>(
        &self,
        gpu_id: &str,
        operation: impl FnOnce(&dyn DaemonBackend) -> T + Send + 'static,
    ) -> T {
        let backend = Arc::clone(&self.dependencies.backend);
        self.queues
            .submit(gpu_id, move || operation(backend.as_ref()))
            .wait()
    }

    fn apply_saved_config(&self, request: ApplyGpuConfigRequest) -> GpuMutationResponse {
        let Some(gpu_id) = request.gpu_id.clone() else {
            let error = GpuCommandError::new(
                GpuCommandErrorCode::InvalidArgument,
                "gpuId is required when applying a GPU configuration",
            );
            return self.save_mutation(|_| (), |_, _| GpuMutationResponse::failure(error));
        };

        let write = request.clone();
        let result = self.write(&gpu_id, move |backend| backend.apply(&write));
        let now = self.now();
        self.save_mutation(
            |state| {
                match &result {
                    Ok(()) => {
                        state.upsert_desired(gpu_id.clone(), request.desired_config());
                        state.set_last_apply(gpu_id, GpuApplyRecord::success_at(now));
                    }
                    Err(error) => state.set_last_apply(
                        gpu_id,
                        GpuApplyRecord::failure_at(now, error.message.clone()),
                    ),
                }
                result
            },
            |result, state| self.mutation_response(result, state),
        )
    }

    fn reset_saved_config(&self, gpu_id: String) -> GpuMutationResponse {
        let target = gpu_id.clone();
        let result = self.write(&gpu_id, move |backend| backend.reset(&target));
        let now = self.now();
        self.save_mutation(
            |state| {
                match &result {
                    Ok(()) => state.clear(&gpu_id),
                    Err(error) => {
                        if state.record_for(&gpu_id).is_some() {
                            state.set_last_apply(
                                gpu_id,
                                GpuApplyRecord::failure_at(now, error.message.clone()),
                            );
                        }
                    }
                }
                result
            },
            |result, state| self.mutation_response(result, state),
        )
    }

    fn reapply_saved_configs_and_save(&self) -> GpuMutationResponse {
        let results = self.reapply_saved_configs();
        let now = self.now();
        self.save_mutation(
            |state| record_reapplication(state, results, now),
            |outcome, state| self.mutation_response(outcome, state),
        )
    }

    /// Reapplies every non-empty saved configuration, each on its GPU's queue.
    fn reapply_saved_configs(&self) -> Vec<(String, Result<(), GpuCommandError>)> {
        let pending: Vec<_> = self
            .saved_configs
            .snapshot()
            .gpus
            .into_iter()
            .filter(|(_, config)| !config.desired.is_empty())
            .map(|(gpu_id, config)| {
                let request = ApplyGpuConfigRequest {
                    gpu_id: Some(gpu_id.clone()),
                    power_limit_watts: config.desired.power_limit_watts,
                    gpu_clock_lock: config.desired.gpu_clock_lock,
                    memory_clock_lock: config.desired.memory_clock_lock,
                    graphics_offset_mhz: config.desired.graphics_offset_mhz,
                    memory_offset_mhz: config.desired.memory_offset_mhz,
                    reapply_saved: false,
                };
                let backend = Arc::clone(&self.dependencies.backend);
                let operation = self.queues.submit(&gpu_id, move || backend.apply(&request));
                (gpu_id, operation)
            })
            .collect();

        pending
            .into_iter()
            .map(|(gpu_id, operation)| (gpu_id, operation.wait()))
            .collect()
    }

    /// Updates and saves the legacy saved configurations, as every legacy mutation always has,
    /// reporting a save failure in place of the mutation's own response.
    fn save_mutation<R>(
        &self,
        update: impl FnOnce(&mut PersistedGpuState) -> R,
        respond: impl FnOnce(R, &PersistedGpuState) -> GpuMutationResponse,
    ) -> GpuMutationResponse {
        let (updated, state, saved) = self.saved_configs.update(update);
        match saved {
            Ok(()) => respond(updated, &state),
            Err(error) => GpuMutationResponse {
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

    fn now(&self) -> u64 {
        self.dependencies.clock.now_unix_ms()
    }
}

fn record_reapplication(
    state: &mut PersistedGpuState,
    results: Vec<(String, Result<(), GpuCommandError>)>,
    now: u64,
) -> Result<(), GpuCommandError> {
    let mut first_error = None;
    for (gpu_id, result) in results {
        match result {
            Ok(()) => state.set_last_apply(gpu_id, GpuApplyRecord::success_at(now)),
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(GpuCommandError::new(
                        error.code,
                        "failed to reapply one or more saved GPU settings",
                    ));
                }
                state.set_last_apply(gpu_id, GpuApplyRecord::failure_at(now, error.message));
            }
        }
    }
    first_error.map_or(Ok(()), Err)
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
struct SavedConfigStore {
    path: PathBuf,
    state: Mutex<PersistedGpuState>,
}

impl SavedConfigStore {
    fn load(path: PathBuf) -> Self {
        let state = Mutex::new(load_state(&path));
        Self { path, state }
    }

    fn snapshot(&self) -> PersistedGpuState {
        self.state.lock().unwrap().clone()
    }

    /// Applies a change and saves the result, returning the change's value and the new state.
    fn update<R>(
        &self,
        change: impl FnOnce(&mut PersistedGpuState) -> R,
    ) -> (R, PersistedGpuState, Result<(), std::io::Error>) {
        let mut state = self.state.lock().unwrap();
        let changed = change(&mut state);
        let saved = save_state(&self.path, &state);
        (changed, state.clone(), saved)
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
