use std::env;
use std::ffi::CString;
use std::ffi::OsStr;
use std::fs;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use users::{get_group_by_gid, get_group_by_name, get_user_by_uid, get_user_groups};

use crate::drivers::{request_host_reboot, DriverService};
use crate::gpu::{
    apply_gpu_config, collect_inventory_with_context, collect_performance, reset_gpu_config,
    reset_gpu_power_limit, set_gpu_clock_offsets, set_gpu_fan_mode_auto, set_gpu_fan_speed,
    set_gpu_power_limit, ApplyGpuConfigRequest, GpuApplyRecord, GpuCommandError,
    GpuCommandErrorCode, GpuDaemonStatus, GpuInventoryResponse, GpuMutationResponse,
    GpuPerformanceResponse, GpuTargetRequest, InventoryContext, PersistedGpuState,
    ResetGpuConfigRequest, SetGpuClockOffsetsRequest, SetGpuFanModeAutoRequest,
    SetGpuFanSpeedRequest, SetGpuPowerLimitRequest,
};
use crate::protocol::{
    ApplyGpuProfileRequest, ClockOffsetResponse, DaemonCommandResponse, DaemonRequest,
    DaemonResponse, ProfileApplicationResponse, ProfileSetting, TuningOutcome, TuningOutcomeStatus,
};

pub const DEFAULT_SOCKET_PATH: &str = "/run/prism/prismd.sock";
pub const DEFAULT_STATE_PATH: &str = "/var/lib/prism/state.json";
pub const DEFAULT_DRIVER_JOBS_PATH: &str = "/var/lib/prism/driver-jobs.json";
pub const DEFAULT_CONFIG_PATH: &str = "/etc/prism/prismd.toml";

#[derive(Debug, Clone)]
struct DaemonPaths {
    socket_path: PathBuf,
    state_path: PathBuf,
    driver_jobs_path: PathBuf,
    config_path: PathBuf,
}

impl DaemonPaths {
    fn from_environment() -> Self {
        Self {
            socket_path: env_path("PRISM_SOCKET_PATH", DEFAULT_SOCKET_PATH),
            state_path: env_path("PRISM_STATE_PATH", DEFAULT_STATE_PATH),
            driver_jobs_path: env_path("PRISM_DRIVER_JOBS_PATH", DEFAULT_DRIVER_JOBS_PATH),
            config_path: env_path("PRISM_CONFIG_PATH", DEFAULT_CONFIG_PATH),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrismDaemonConfigFile {
    admin_group: Option<String>,
    admin_user: Option<String>,
}

#[derive(Debug, Clone)]
struct PrismDaemonConfig {
    admin_group: String,
    admin_user: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct PeerCredentials {
    uid: u32,
    gid: u32,
}

trait DaemonBackend {
    fn inventory(&self, persisted_state: &PersistedGpuState) -> GpuInventoryResponse;
    fn performance(&self) -> GpuPerformanceResponse;
    fn apply(&self, request: &ApplyGpuConfigRequest) -> Result<(), GpuCommandError>;
    fn reset(&self, gpu_id: &str) -> Result<(), GpuCommandError>;
    fn set_power_limit(&self, request: SetGpuPowerLimitRequest) -> Result<(), String>;
    fn reset_power_limit(&self, request: GpuTargetRequest) -> Result<(), String>;
    fn set_fan_speed(&self, request: SetGpuFanSpeedRequest) -> Result<(), String>;
    fn set_fan_mode_auto(&self, request: SetGpuFanModeAutoRequest) -> Result<(), String>;
    fn set_clock_offsets(&self, request: SetGpuClockOffsetsRequest) -> Result<(), String>;
}

struct RealDaemonBackend;

impl DaemonBackend for RealDaemonBackend {
    fn inventory(&self, persisted_state: &PersistedGpuState) -> GpuInventoryResponse {
        collect_inventory_with_context(InventoryContext {
            managed_by_daemon: true,
            daemon_reachable: true,
            persisted_state: Some(persisted_state),
            daemon_warning: None,
        })
    }

    fn performance(&self) -> GpuPerformanceResponse {
        collect_performance()
    }

    fn apply(&self, request: &ApplyGpuConfigRequest) -> Result<(), GpuCommandError> {
        apply_gpu_config(request)
    }

    fn reset(&self, gpu_id: &str) -> Result<(), GpuCommandError> {
        reset_gpu_config(gpu_id)
    }

    fn set_power_limit(&self, request: SetGpuPowerLimitRequest) -> Result<(), String> {
        set_gpu_power_limit(request)
    }

    fn reset_power_limit(&self, request: GpuTargetRequest) -> Result<(), String> {
        reset_gpu_power_limit(request)
    }

    fn set_fan_speed(&self, request: SetGpuFanSpeedRequest) -> Result<(), String> {
        set_gpu_fan_speed(request)
    }

    fn set_fan_mode_auto(&self, request: SetGpuFanModeAutoRequest) -> Result<(), String> {
        set_gpu_fan_mode_auto(request)
    }

    fn set_clock_offsets(&self, request: SetGpuClockOffsetsRequest) -> Result<(), String> {
        set_gpu_clock_offsets(request)
    }
}

pub fn run() -> Result<(), String> {
    let paths = DaemonPaths::from_environment();
    let config = load_config(&paths).map_err(|error| error.to_string())?;
    let listener = bind_listener(&paths, &config).map_err(|error| error.to_string())?;
    let backend = RealDaemonBackend;
    let driver_service = DriverService::new(paths.driver_jobs_path.clone());
    let mut persisted_state = load_state(&paths.state_path);

    let startup_result = reapply_saved_configs(&backend, &mut persisted_state);
    if let Err(error) = save_state(&paths.state_path, &persisted_state) {
        eprintln!("failed to save Prism daemon state on startup: {error}");
    }
    if let Err(error) = startup_result {
        eprintln!(
            "failed to reapply one or more saved GPU settings on startup: {}",
            error.message
        );
    }

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                if let Err(error) = handle_connection(
                    &backend,
                    &driver_service,
                    &paths,
                    &config,
                    &mut persisted_state,
                    &mut stream,
                ) {
                    eprintln!("failed to serve Prism daemon request: {error}");
                }
            }
            Err(error) => eprintln!("failed to accept Prism daemon connection: {error}"),
        }
    }

    Ok(())
}

fn handle_connection<B: DaemonBackend>(
    backend: &B,
    driver_service: &DriverService,
    paths: &DaemonPaths,
    config: &PrismDaemonConfig,
    persisted_state: &mut PersistedGpuState,
    stream: &mut UnixStream,
) -> Result<(), String> {
    let mut request = Vec::new();
    stream
        .read_to_end(&mut request)
        .map_err(|error| error.to_string())?;

    let request: DaemonRequest =
        serde_json::from_slice(&request).map_err(|error| error.to_string())?;
    let response = match authorize_request(config, stream, &request) {
        Ok(()) => handle_request_with_driver_service(
            backend,
            driver_service,
            paths,
            persisted_state,
            request,
        ),
        Err(error) => DaemonResponse::Error(error),
    };
    let payload = serde_json::to_vec(&response).map_err(|error| error.to_string())?;

    stream
        .write_all(&payload)
        .and_then(|_| stream.flush())
        .map_err(|error| error.to_string())?;

    Ok(())
}

#[cfg(test)]
fn handle_request<B: DaemonBackend>(
    backend: &B,
    paths: &DaemonPaths,
    persisted_state: &mut PersistedGpuState,
    request: DaemonRequest,
) -> DaemonResponse {
    let driver_service = DriverService::new(paths.driver_jobs_path.clone());
    handle_request_with_driver_service(backend, &driver_service, paths, persisted_state, request)
}

fn handle_request_with_driver_service<B: DaemonBackend>(
    backend: &B,
    driver_service: &DriverService,
    paths: &DaemonPaths,
    persisted_state: &mut PersistedGpuState,
    request: DaemonRequest,
) -> DaemonResponse {
    match request {
        DaemonRequest::GetInventory => {
            DaemonResponse::Inventory(backend.inventory(persisted_state))
        }
        DaemonRequest::GetPerformance => DaemonResponse::Performance(backend.performance()),
        DaemonRequest::ApplyProfile(request) => {
            DaemonResponse::ProfileApplication(apply_profile(backend, request))
        }
        DaemonRequest::GetDaemonStatus => DaemonResponse::DaemonStatus(GpuDaemonStatus::connected(
            paths.socket_path.display().to_string(),
        )),
        DaemonRequest::ApplyConfig(request) => {
            let response = if request.reapply_saved {
                match reapply_saved_configs(backend, persisted_state) {
                    Ok(()) => mutation_success(backend, persisted_state),
                    Err(error) => mutation_failure(backend, persisted_state, error),
                }
            } else {
                apply_single_config(backend, persisted_state, request)
            };

            if let Err(error) = save_state(&paths.state_path, persisted_state) {
                return DaemonResponse::Mutation(GpuMutationResponse {
                    success: false,
                    inventory: Some(backend.inventory(persisted_state)),
                    error: Some(GpuCommandError::new(
                        GpuCommandErrorCode::Io,
                        error.to_string(),
                    )),
                });
            }

            DaemonResponse::Mutation(response)
        }
        DaemonRequest::ResetConfig(request) => {
            let response = reset_single_config(backend, persisted_state, request);

            if let Err(error) = save_state(&paths.state_path, persisted_state) {
                return DaemonResponse::Mutation(GpuMutationResponse {
                    success: false,
                    inventory: Some(backend.inventory(persisted_state)),
                    error: Some(GpuCommandError::new(
                        GpuCommandErrorCode::Io,
                        error.to_string(),
                    )),
                });
            }

            DaemonResponse::Mutation(response)
        }
        DaemonRequest::ReapplySavedConfig => {
            let response = match reapply_saved_configs(backend, persisted_state) {
                Ok(()) => mutation_success(backend, persisted_state),
                Err(error) => mutation_failure(backend, persisted_state, error),
            };

            if let Err(error) = save_state(&paths.state_path, persisted_state) {
                return DaemonResponse::Mutation(GpuMutationResponse {
                    success: false,
                    inventory: Some(backend.inventory(persisted_state)),
                    error: Some(GpuCommandError::new(
                        GpuCommandErrorCode::Io,
                        error.to_string(),
                    )),
                });
            }

            DaemonResponse::Mutation(response)
        }
        DaemonRequest::SetPowerLimit(request) => {
            DaemonResponse::Command(command_result(backend.set_power_limit(request)))
        }
        DaemonRequest::ResetPowerLimit(request) => {
            DaemonResponse::Command(command_result(backend.reset_power_limit(request)))
        }
        DaemonRequest::SetFanSpeed(request) => {
            DaemonResponse::Command(command_result(backend.set_fan_speed(request)))
        }
        DaemonRequest::SetFanModeAuto(request) => {
            DaemonResponse::Command(command_result(backend.set_fan_mode_auto(request)))
        }
        DaemonRequest::SetClockOffsets(request) => {
            DaemonResponse::ClockOffsets(apply_live_offsets(backend, request))
        }
        DaemonRequest::ResetClockOffsets(request) => {
            DaemonResponse::ClockOffsets(reset_live_offsets(backend, request))
        }
        DaemonRequest::GetDriverStatus => DaemonResponse::DriverStatus(driver_service.status()),
        DaemonRequest::StartDriverAction(request) => match driver_service.start_action(request) {
            Ok(job) => DaemonResponse::DriverJob(job),
            Err(error) => DaemonResponse::Error(error),
        },
        DaemonRequest::GetDriverJob(request) => match driver_service.job(request) {
            Ok(job) => DaemonResponse::DriverJob(job),
            Err(error) => DaemonResponse::Error(error),
        },
        DaemonRequest::CancelDriverJob(request) => match driver_service.cancel_job(request) {
            Ok(job) => DaemonResponse::DriverJob(job),
            Err(error) => DaemonResponse::Error(error),
        },
        DaemonRequest::RequestHostReboot => DaemonResponse::Command(command_result(
            request_host_reboot().map_err(|error| error.message),
        )),
    }
}

// Validate the entire profile before any live write. Named profiles never change saved configuration.
fn apply_profile<B: DaemonBackend>(
    backend: &B,
    request: ApplyGpuProfileRequest,
) -> ProfileApplicationResponse {
    use crate::gpu::ControlSupport;
    let settings = &request.settings;
    let blocked = |message: String| {
        let outcome = |_| TuningOutcome {
            status: TuningOutcomeStatus::Unattempted,
            error: Some(message.clone()),
        };
        ProfileApplicationResponse {
            power: settings.power_limit_watts.map(outcome),
            graphics: settings.graphics_offset_mhz.map(outcome),
            memory: settings.memory_offset_mhz.map(outcome),
        }
    };
    let performance = backend.performance();
    let mut owners = performance.gpus.iter().filter(|gpu| {
        !request.gpu_id.trim().is_empty()
            && gpu.stable_id.as_deref() == Some(request.gpu_id.as_str())
    });
    let Some(gpu) = owners.next() else {
        return blocked("The owning GPU is absent or its stable identity could not be verified. Refresh the GPU readings and try again.".into());
    };
    if owners.next().is_some() {
        return blocked("The owning GPU's stable identity is ambiguous. Refresh the GPU readings and try again.".into());
    }
    // Resolve explicit defaults from the same fresh readings used for validation.
    let mut errors = Vec::new();
    let mut resolve = |label: &str,
                       setting: Option<ProfileSetting>,
                       support: ControlSupport,
                       reason: &Option<String>,
                       default: Option<i64>,
                       min: Option<i64>,
                       max: Option<i64>|
     -> Option<i64> {
        let setting = setting?;
        match support {
            ControlSupport::Unsupported => errors.push(format!(
                "{label} is unsupported. {}",
                reason.as_deref().unwrap_or("Choose a compatible profile.")
            )),
            ControlSupport::Unknown => errors.push(format!(
                "{label} support is unknown. {}",
                reason
                    .as_deref()
                    .unwrap_or("Refresh the GPU readings and try again.")
            )),
            ControlSupport::Supported => {
                let (value, requested) = match setting {
                    ProfileSetting::Value { value } => (value, "requested"),
                    ProfileSetting::Default {} => match default {
                        Some(value) => (value, "the default"),
                        None => {
                            errors.push(format!("{label}: the default is unavailable. Refresh the GPU readings and try again."));
                            return None;
                        }
                    },
                };
                match (min, max) {
                    (Some(min), Some(max)) if min <= max => {
                        if value < min || value > max {
                            errors.push(format!("{label}: {requested} {value} is outside the current allowed range {min} to {max}. Update the profile or cancel loading."));
                        }
                    },
                    _ => errors.push(format!("{label}: the current allowed range is unavailable. Refresh the GPU readings and try again.")),
                }
                return Some(value);
            }
        }
        None
    };
    let power = resolve(
        "Power limit",
        settings.power_limit_watts,
        gpu.power.support,
        &gpu.power.reason,
        gpu.power.default_limit_watts.map(i64::from),
        gpu.power.min_limit_watts.map(i64::from),
        gpu.power.max_limit_watts.map(i64::from),
    );
    let graphics = resolve(
        "Graphics clock offset",
        settings.graphics_offset_mhz,
        gpu.clocks.graphics.support,
        &gpu.clocks.graphics.reason,
        Some(0),
        gpu.clocks.graphics.min_offset_mhz.map(i64::from),
        gpu.clocks.graphics.max_offset_mhz.map(i64::from),
    );
    let memory = resolve(
        "Memory clock offset",
        settings.memory_offset_mhz,
        gpu.clocks.memory.support,
        &gpu.clocks.memory.reason,
        Some(0),
        gpu.clocks.memory.min_offset_mhz.map(i64::from),
        gpu.clocks.memory.max_offset_mhz.map(i64::from),
    );
    if !errors.is_empty() {
        return blocked(errors.join("\n"));
    }
    let mut failed = false;
    let mut execute = |operation: &mut dyn FnMut() -> Result<(), String>| {
        if failed {
            return TuningOutcome {
                status: TuningOutcomeStatus::Unattempted,
                error: Some(
                    "An earlier profile setting failed. Retry or discard this instruction.".into(),
                ),
            };
        }
        match operation() {
            Ok(()) => TuningOutcome {
                status: TuningOutcomeStatus::Succeeded,
                error: None,
            },
            Err(error) => {
                failed = true;
                TuningOutcome {
                    status: TuningOutcomeStatus::Failed,
                    error: Some(error),
                }
            }
        }
    };
    ProfileApplicationResponse {
        power: power.map(|value| {
            execute(&mut || {
                backend.set_power_limit(SetGpuPowerLimitRequest {
                    gpu_id: request.gpu_id.clone(),
                    limit_watts: value as u32,
                })
            })
        }),
        graphics: graphics.map(|value| {
            execute(&mut || {
                backend.set_clock_offsets(SetGpuClockOffsetsRequest {
                    gpu_id: request.gpu_id.clone(),
                    graphics_offset_mhz: Some(value as i32),
                    memory_offset_mhz: None,
                })
            })
        }),
        memory: memory.map(|value| {
            execute(&mut || {
                backend.set_clock_offsets(SetGpuClockOffsetsRequest {
                    gpu_id: request.gpu_id.clone(),
                    graphics_offset_mhz: None,
                    memory_offset_mhz: Some(value as i32),
                })
            })
        }),
    }
}

// Split the existing live command so a later failure cannot hide an earlier write.
// These commands deliberately never touch PersistedGpuState.
fn apply_live_offsets<B: DaemonBackend>(
    backend: &B,
    request: SetGpuClockOffsetsRequest,
) -> ClockOffsetResponse {
    let mut failed = false;
    let mut apply = |graphics: bool, value: Option<i32>| {
        value.map(|value| {
            if failed {
                return TuningOutcome {
                    status: TuningOutcomeStatus::Unattempted,
                    error: Some("An earlier clock offset failed".into()),
                };
            }
            let result = backend.set_clock_offsets(SetGpuClockOffsetsRequest {
                gpu_id: request.gpu_id.clone(),
                graphics_offset_mhz: if graphics { Some(value) } else { None },
                memory_offset_mhz: if graphics { None } else { Some(value) },
            });
            match result {
                Ok(()) => TuningOutcome {
                    status: TuningOutcomeStatus::Succeeded,
                    error: None,
                },
                Err(error) => {
                    failed = true;
                    TuningOutcome {
                        status: TuningOutcomeStatus::Failed,
                        error: Some(error),
                    }
                }
            }
        })
    };
    ClockOffsetResponse {
        graphics: apply(true, request.graphics_offset_mhz),
        memory: apply(false, request.memory_offset_mhz),
    }
}

fn reset_live_offsets<B: DaemonBackend>(
    backend: &B,
    request: GpuTargetRequest,
) -> ClockOffsetResponse {
    use crate::gpu::ControlSupport;
    let performance = backend.performance();
    let gpu = performance.gpus.iter().find(|gpu| gpu.id == request.gpu_id);
    let unavailable = |message: &str| {
        Some(TuningOutcome {
            status: TuningOutcomeStatus::Unattempted,
            error: Some(message.into()),
        })
    };
    let Some(gpu) = gpu else {
        return ClockOffsetResponse {
            graphics: unavailable("GPU unavailable"),
            memory: unavailable("GPU unavailable"),
        };
    };
    if gpu.clocks.graphics.support == ControlSupport::Unknown
        || gpu.clocks.memory.support == ControlSupport::Unknown
    {
        return ClockOffsetResponse {
            graphics: unavailable("Clock offset support is unknown"),
            memory: unavailable("Clock offset support is unknown"),
        };
    }
    apply_live_offsets(
        backend,
        SetGpuClockOffsetsRequest {
            gpu_id: request.gpu_id,
            graphics_offset_mhz: (gpu.clocks.graphics.support == ControlSupport::Supported)
                .then_some(0),
            memory_offset_mhz: (gpu.clocks.memory.support == ControlSupport::Supported)
                .then_some(0),
        },
    )
}

fn apply_single_config<B: DaemonBackend>(
    backend: &B,
    persisted_state: &mut PersistedGpuState,
    request: ApplyGpuConfigRequest,
) -> GpuMutationResponse {
    let Some(gpu_id) = request.gpu_id.clone() else {
        return GpuMutationResponse::failure(GpuCommandError::new(
            GpuCommandErrorCode::InvalidArgument,
            "gpuId is required when applying a GPU configuration",
        ));
    };

    match backend.apply(&request) {
        Ok(()) => {
            persisted_state.upsert_desired(gpu_id.clone(), request.desired_config());
            persisted_state.set_last_apply(gpu_id, GpuApplyRecord::success());
            mutation_success(backend, persisted_state)
        }
        Err(error) => {
            persisted_state.set_last_apply(gpu_id, GpuApplyRecord::failure(error.message.clone()));
            mutation_failure(backend, persisted_state, error)
        }
    }
}

fn reset_single_config<B: DaemonBackend>(
    backend: &B,
    persisted_state: &mut PersistedGpuState,
    request: ResetGpuConfigRequest,
) -> GpuMutationResponse {
    match backend.reset(&request.gpu_id) {
        Ok(()) => {
            persisted_state.clear(&request.gpu_id);
            mutation_success(backend, persisted_state)
        }
        Err(error) => {
            if persisted_state.record_for(&request.gpu_id).is_some() {
                persisted_state.set_last_apply(
                    request.gpu_id.clone(),
                    GpuApplyRecord::failure(error.message.clone()),
                );
            }
            mutation_failure(backend, persisted_state, error)
        }
    }
}

fn reapply_saved_configs<B: DaemonBackend>(
    backend: &B,
    persisted_state: &mut PersistedGpuState,
) -> Result<(), GpuCommandError> {
    let saved_configs: Vec<(String, ApplyGpuConfigRequest)> = persisted_state
        .gpus
        .iter()
        .filter(|(_, config)| !config.desired.is_empty())
        .map(|(gpu_id, config)| {
            (
                gpu_id.clone(),
                ApplyGpuConfigRequest {
                    gpu_id: Some(gpu_id.clone()),
                    power_limit_watts: config.desired.power_limit_watts,
                    gpu_clock_lock: config.desired.gpu_clock_lock,
                    memory_clock_lock: config.desired.memory_clock_lock,
                    graphics_offset_mhz: config.desired.graphics_offset_mhz,
                    memory_offset_mhz: config.desired.memory_offset_mhz,
                    reapply_saved: false,
                },
            )
        })
        .collect();

    let mut first_error = None;

    for (gpu_id, request) in saved_configs {
        match backend.apply(&request) {
            Ok(()) => persisted_state.set_last_apply(gpu_id, GpuApplyRecord::success()),
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(GpuCommandError::new(
                        error.code,
                        "failed to reapply one or more saved GPU settings",
                    ));
                }

                persisted_state
                    .set_last_apply(gpu_id, GpuApplyRecord::failure(error.message.clone()));
            }
        }
    }

    if let Some(error) = first_error {
        return Err(error);
    }

    Ok(())
}

fn mutation_success<B: DaemonBackend>(
    backend: &B,
    persisted_state: &PersistedGpuState,
) -> GpuMutationResponse {
    GpuMutationResponse {
        success: true,
        inventory: Some(backend.inventory(persisted_state)),
        error: None,
    }
}

fn mutation_failure<B: DaemonBackend>(
    backend: &B,
    persisted_state: &PersistedGpuState,
    error: GpuCommandError,
) -> GpuMutationResponse {
    GpuMutationResponse {
        success: false,
        inventory: Some(backend.inventory(persisted_state)),
        error: Some(error),
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

fn authorize_request(
    config: &PrismDaemonConfig,
    stream: &UnixStream,
    request: &DaemonRequest,
) -> Result<(), GpuCommandError> {
    if matches!(request, DaemonRequest::GetDaemonStatus) {
        return Ok(());
    }

    let credentials = peer_credentials(stream)
        .map_err(|error| GpuCommandError::new(GpuCommandErrorCode::Io, error.to_string()))?;

    if credentials.uid == 0 {
        return Ok(());
    }

    let user = get_user_by_uid(credentials.uid).ok_or_else(|| {
        GpuCommandError::new(
            GpuCommandErrorCode::NoPermission,
            format!("Unable to resolve user for uid {}", credentials.uid),
        )
    })?;

    if let Some(admin_user) = config.admin_user.as_deref() {
        if user.name() == OsStr::new(admin_user) {
            return Ok(());
        }
    }

    let groups = get_user_groups(user.name(), credentials.gid).ok_or_else(|| {
        GpuCommandError::new(
            GpuCommandErrorCode::NoPermission,
            format!(
                "Unable to resolve group membership for user {}",
                user.name().to_string_lossy()
            ),
        )
    })?;

    if groups
        .iter()
        .any(|group| group.name() == OsStr::new(&config.admin_group))
    {
        return Ok(());
    }

    let primary_group = get_group_by_gid(credentials.gid)
        .map(|group| group.name().to_string_lossy().into_owned())
        .unwrap_or_else(|| credentials.gid.to_string());

    Err(GpuCommandError::new(
        GpuCommandErrorCode::NoPermission,
        format!(
            "User {} is not allowed to access prismd (primary group: {primary_group})",
            user.name().to_string_lossy()
        ),
    ))
}

fn load_config(paths: &DaemonPaths) -> Result<PrismDaemonConfig, std::io::Error> {
    let raw = match fs::read_to_string(&paths.config_path) {
        Ok(contents) => toml::from_str::<PrismDaemonConfigFile>(&contents).map_err(|error| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string())
        })?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => PrismDaemonConfigFile {
            admin_group: None,
            admin_user: None,
        },
        Err(error) => return Err(error),
    };

    Ok(PrismDaemonConfig {
        admin_group: raw.admin_group.unwrap_or_else(default_admin_group),
        admin_user: raw.admin_user,
    })
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

fn bind_listener(
    paths: &DaemonPaths,
    config: &PrismDaemonConfig,
) -> Result<UnixListener, std::io::Error> {
    if let Some(parent) = paths.socket_path.parent() {
        fs::create_dir_all(parent)?;
    }

    if paths.socket_path.exists() {
        fs::remove_file(&paths.socket_path)?;
    }

    let listener = UnixListener::bind(&paths.socket_path)?;
    let socket_mode = if config.admin_user.is_some() {
        0o666
    } else {
        0o660
    };
    fs::set_permissions(&paths.socket_path, fs::Permissions::from_mode(socket_mode))?;
    set_socket_group(&paths.socket_path, &config.admin_group)?;

    Ok(listener)
}

fn set_socket_group(socket_path: &Path, admin_group: &str) -> Result<(), std::io::Error> {
    let Some(group) = get_group_by_name(admin_group) else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("group '{admin_group}' was not found"),
        ));
    };

    let path = CString::new(socket_path.as_os_str().as_bytes()).map_err(|error| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, error.to_string())
    })?;

    let result = unsafe { libc::chown(path.as_ptr(), u32::MAX, group.gid()) };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }

    Ok(())
}

fn peer_credentials(stream: &UnixStream) -> Result<PeerCredentials, std::io::Error> {
    let fd = stream.as_raw_fd();
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;

    let result = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut credentials as *mut _ as *mut libc::c_void,
            &mut size,
        )
    };

    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }

    Ok(PeerCredentials {
        uid: credentials.uid,
        gid: credentials.gid,
    })
}

fn env_path(var_name: &str, default: &str) -> PathBuf {
    env::var_os(var_name)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default))
}

fn default_admin_group() -> String {
    if get_group_by_name("wheel").is_some() {
        "wheel".to_string()
    } else if get_group_by_name("sudo").is_some() {
        "sudo".to_string()
    } else {
        "root".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{handle_request, DaemonBackend, DaemonPaths};
    use crate::gpu::{
        ApplyGpuConfigRequest, ClockRangeInput, DesiredGpuConfig, GpuCommandError,
        GpuCommandErrorCode, GpuInventoryResponse, GpuPerformanceResponse, GpuTargetRequest,
        PersistedGpuConfig, PersistedGpuState, SetGpuClockOffsetsRequest, SetGpuFanModeAutoRequest,
        SetGpuFanSpeedRequest, SetGpuPowerLimitRequest,
    };
    use crate::protocol::{ApplyGpuProfileRequest, DaemonRequest, DaemonResponse, ProfileSetting};
    use std::cell::RefCell;
    use std::path::PathBuf;

    struct MockBackend {
        should_fail_apply: bool,
        should_fail_reset: bool,
        command_error: Option<String>,
        graphics_offset_error: Option<String>,
        memory_offset_error: Option<String>,
        applied_requests: RefCell<Vec<ApplyGpuConfigRequest>>,
        performance: RefCell<GpuPerformanceResponse>,
    }

    impl DaemonBackend for MockBackend {
        fn inventory(&self, persisted_state: &PersistedGpuState) -> GpuInventoryResponse {
            GpuInventoryResponse {
                gpus: vec![],
                warnings: persisted_state
                    .gpus
                    .values()
                    .filter_map(|record| {
                        record
                            .last_apply
                            .as_ref()
                            .and_then(|last_apply| last_apply.message.clone())
                    })
                    .collect(),
            }
        }

        fn performance(&self) -> GpuPerformanceResponse {
            self.performance.borrow().clone()
        }

        fn apply(&self, request: &ApplyGpuConfigRequest) -> Result<(), GpuCommandError> {
            self.applied_requests.borrow_mut().push(request.clone());

            if self.should_fail_apply {
                Err(GpuCommandError::new(
                    GpuCommandErrorCode::Internal,
                    "mock apply failed",
                ))
            } else {
                Ok(())
            }
        }

        fn reset(&self, _gpu_id: &str) -> Result<(), GpuCommandError> {
            if self.should_fail_reset {
                Err(GpuCommandError::new(
                    GpuCommandErrorCode::Internal,
                    "mock reset failed",
                ))
            } else {
                Ok(())
            }
        }

        fn set_power_limit(&self, request: SetGpuPowerLimitRequest) -> Result<(), String> {
            self.command_result()?;
            if let Some(gpu) = self
                .performance
                .borrow_mut()
                .gpus
                .iter_mut()
                .find(|gpu| gpu.id == request.gpu_id)
            {
                gpu.power.current_limit_watts = Some(request.limit_watts);
            }
            Ok(())
        }

        fn reset_power_limit(&self, _request: GpuTargetRequest) -> Result<(), String> {
            self.command_result()
        }

        fn set_fan_speed(&self, _request: SetGpuFanSpeedRequest) -> Result<(), String> {
            self.command_result()
        }

        fn set_fan_mode_auto(&self, _request: SetGpuFanModeAutoRequest) -> Result<(), String> {
            self.command_result()
        }

        fn set_clock_offsets(&self, request: SetGpuClockOffsetsRequest) -> Result<(), String> {
            self.command_result()?;
            let mut performance = self.performance.borrow_mut();
            let gpu = performance
                .gpus
                .iter_mut()
                .find(|gpu| gpu.id == request.gpu_id)
                .unwrap();
            if let Some(value) = request.graphics_offset_mhz {
                if let Some(error) = &self.graphics_offset_error {
                    return Err(error.clone());
                }
                gpu.clocks.graphics.current_offset_mhz = Some(value);
            }
            if let Some(value) = request.memory_offset_mhz {
                if let Some(error) = &self.memory_offset_error {
                    return Err(error.clone());
                }
                gpu.clocks.memory.current_offset_mhz = Some(value);
            }
            Ok(())
        }
    }

    impl MockBackend {
        fn new() -> Self {
            Self {
                should_fail_apply: false,
                should_fail_reset: false,
                command_error: None,
                graphics_offset_error: None,
                memory_offset_error: None,
                applied_requests: RefCell::new(vec![]),
                performance: RefCell::new(GpuPerformanceResponse {
                    gpus: vec![],
                    warnings: vec![],
                }),
            }
        }

        fn command_result(&self) -> Result<(), String> {
            self.command_error.clone().map_or(Ok(()), Err)
        }
    }

    fn test_paths() -> DaemonPaths {
        DaemonPaths {
            socket_path: PathBuf::from("/tmp/prismd-test.sock"),
            state_path: PathBuf::from("/tmp/prismd-test-state.json"),
            driver_jobs_path: PathBuf::from("/tmp/prismd-test-driver-jobs.json"),
            config_path: PathBuf::from("/tmp/prismd-test.toml"),
        }
    }

    #[test]
    fn profile_validation_blocks_all_writes_when_an_included_setting_is_unsupported() {
        let backend = MockBackend::new();
        *backend.performance.borrow_mut() = serde_json::from_str(include_str!(
            "../../src/test/contracts/tuning-observations.json"
        ))
        .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let paths = DaemonPaths {
            state_path: directory.path().join("state.json"),
            ..test_paths()
        };
        let mut state = PersistedGpuState::default();
        let request = serde_json::from_value(serde_json::json!({
            "type": "apply_profile", "payload": {
                "gpuId": "GPU-123", "settings": { "powerLimitWatts": { "kind": "value", "value": 450 }, "memoryOffsetMHz": { "kind": "value", "value": 80 } }
            }
        }))
        .unwrap();
        let result =
            serde_json::to_value(handle_request(&backend, &paths, &mut state, request)).unwrap();
        assert_eq!(result["payload"]["power"]["status"], "unattempted");
        assert_eq!(result["payload"]["memory"]["status"], "unattempted");
        assert!(result["payload"]["memory"]["error"]
            .as_str()
            .unwrap()
            .contains("Memory clock offset is unsupported"));
        let DaemonResponse::Performance(actual) =
            handle_request(&backend, &paths, &mut state, DaemonRequest::GetPerformance)
        else {
            panic!("expected performance")
        };
        assert_eq!(actual.gpus[0].power.current_limit_watts, None);
        assert!(!paths.state_path.exists());
    }

    fn profile_backend() -> MockBackend {
        let backend = MockBackend::new();
        let mut performance: GpuPerformanceResponse = serde_json::from_str(include_str!(
            "../../src/test/contracts/tuning-observations.json"
        ))
        .unwrap();
        let gpu = &mut performance.gpus[0];
        gpu.power.current_limit_watts = Some(475);
        gpu.power.current_value_error = None;
        gpu.clocks.graphics.support = crate::gpu::ControlSupport::Supported;
        gpu.clocks.graphics.current_offset_mhz = Some(120);
        gpu.clocks.graphics.min_offset_mhz = Some(-200);
        gpu.clocks.graphics.max_offset_mhz = Some(200);
        gpu.clocks.graphics.reason = None;
        gpu.clocks.memory = gpu.clocks.graphics.clone();
        gpu.clocks.memory.current_offset_mhz = Some(0);
        *backend.performance.borrow_mut() = performance;
        backend
    }

    #[test]
    fn profiles_validate_fresh_support_ranges_and_stable_ownership_before_any_write() {
        use crate::gpu::ControlSupport;
        for invalid in [
            "power low",
            "power high",
            "graphics low",
            "memory high",
            "unknown support",
            "unsupported",
            "missing range",
            "reversed range",
            "absent owner",
            "unverified identity",
            "ambiguous identity",
        ] {
            let backend = profile_backend();
            let mut performance = backend.performance.borrow_mut();
            match invalid {
                "power low" => performance.gpus[0].power.min_limit_watts = Some(500),
                "power high" => performance.gpus[0].power.max_limit_watts = Some(400),
                "graphics low" => performance.gpus[0].clocks.graphics.min_offset_mhz = Some(180),
                "memory high" => performance.gpus[0].clocks.memory.max_offset_mhz = Some(50),
                "unknown support" => {
                    performance.gpus[0].clocks.memory.support = ControlSupport::Unknown
                }
                "unsupported" => {
                    performance.gpus[0].clocks.memory.support = ControlSupport::Unsupported
                }
                "missing range" => performance.gpus[0].clocks.memory.min_offset_mhz = None,
                "reversed range" => performance.gpus[0].clocks.memory.min_offset_mhz = Some(300),
                "absent owner" => {
                    performance.gpus[0].stable_id = Some("GPU-replacement".into());
                    performance.gpus[0].id = "GPU-replacement".into();
                }
                "unverified identity" => performance.gpus[0].stable_id = None,
                "ambiguous identity" => {
                    let duplicate = performance.gpus[0].clone();
                    performance.gpus.push(duplicate);
                }
                _ => unreachable!(),
            }
            let before = performance.clone();
            drop(performance);
            let directory = tempfile::tempdir().unwrap();
            let paths = DaemonPaths {
                state_path: directory.path().join("state.json"),
                ..test_paths()
            };
            let mut state = PersistedGpuState::default();
            let request = serde_json::from_value(serde_json::json!({ "type": "apply_profile", "payload": {
                "gpuId": "GPU-123", "settings": { "powerLimitWatts": { "kind": "value", "value": 450 }, "graphicsOffsetMHz": { "kind": "value", "value": 150 }, "memoryOffsetMHz": { "kind": "value", "value": 80 } }
            }})).unwrap();
            let result =
                serde_json::to_value(handle_request(&backend, &paths, &mut state, request))
                    .unwrap();
            for setting in ["power", "graphics", "memory"] {
                assert_eq!(
                    result["payload"][setting]["status"], "unattempted",
                    "{invalid}"
                );
                assert!(
                    result["payload"][setting]["error"].as_str().unwrap().len() > 10,
                    "{invalid}"
                );
            }
            let DaemonResponse::Performance(actual) =
                handle_request(&backend, &paths, &mut state, DaemonRequest::GetPerformance)
            else {
                panic!("expected performance")
            };
            assert_eq!(actual, before, "{invalid}");
            assert!(!paths.state_path.exists());
        }
    }

    #[test]
    fn profile_application_changes_only_included_values_on_its_owner_and_preserves_saved_configuration(
    ) {
        let backend = profile_backend();
        let mut other = backend.performance.borrow().gpus[0].clone();
        other.id = "GPU-other".into();
        other.stable_id = Some(other.id.clone());
        backend
            .performance
            .borrow_mut()
            .gpus
            .insert(0, other.clone());
        // Omitted controls need no applicability information.
        backend.performance.borrow_mut().gpus[1]
            .clocks
            .memory
            .support = crate::gpu::ControlSupport::Unknown;
        let before = backend.performance.borrow().gpus[1].clone();
        let mut state = PersistedGpuState::default();
        state.gpus.insert(
            "GPU-123".into(),
            PersistedGpuConfig {
                desired: DesiredGpuConfig {
                    power_limit_watts: Some(575),
                    gpu_clock_lock: Some(ClockRangeInput {
                        min_mhz: 1800,
                        max_mhz: 2400,
                    }),
                    memory_clock_lock: Some(ClockRangeInput {
                        min_mhz: 6000,
                        max_mhz: 8000,
                    }),
                    ..DesiredGpuConfig::default()
                },
                last_apply: None,
            },
        );
        let saved = state.clone();
        let directory = tempfile::tempdir().unwrap();
        let paths = DaemonPaths {
            state_path: directory.path().join("state.json"),
            ..test_paths()
        };
        let bytes = serde_json::to_vec(&saved).unwrap();
        std::fs::write(&paths.state_path, &bytes).unwrap();
        let request = serde_json::from_value(serde_json::json!({ "type": "apply_profile", "payload": {
            "gpuId": "GPU-123", "settings": { "powerLimitWatts": { "kind": "value", "value": 450 }, "graphicsOffsetMHz": { "kind": "value", "value": -150 } }
        }})).unwrap();
        let result =
            serde_json::to_value(handle_request(&backend, &paths, &mut state, request)).unwrap();
        assert_eq!(result["payload"]["power"]["status"], "succeeded");
        assert_eq!(result["payload"]["graphics"]["status"], "succeeded");
        assert!(result["payload"]["memory"].is_null());
        let DaemonResponse::Performance(actual) =
            handle_request(&backend, &paths, &mut state, DaemonRequest::GetPerformance)
        else {
            panic!("expected performance")
        };
        assert_eq!(actual.gpus[0], other);
        assert_eq!(actual.gpus[1].power.current_limit_watts, Some(450));
        assert_eq!(
            actual.gpus[1].clocks.graphics.current_offset_mhz,
            Some(-150)
        );
        assert_eq!(actual.gpus[1].clocks.memory, before.clocks.memory);
        assert_eq!(actual.gpus[1].cooling, before.cooling);
        assert_eq!(state, saved);
        assert_eq!(std::fs::read(&paths.state_path).unwrap(), bytes);
    }

    #[test]
    fn profile_application_reports_partial_failure_and_validates_retries() {
        let mut backend = profile_backend();
        backend.graphics_offset_error = Some("Graphics write denied".into());
        let directory = tempfile::tempdir().unwrap();
        let paths = DaemonPaths {
            state_path: directory.path().join("state.json"),
            ..test_paths()
        };
        let mut state = PersistedGpuState::default();
        let request = serde_json::from_value(serde_json::json!({ "type": "apply_profile", "payload": {
            "gpuId": "GPU-123", "settings": { "powerLimitWatts": { "kind": "value", "value": 450 }, "graphicsOffsetMHz": { "kind": "value", "value": 150 }, "memoryOffsetMHz": { "kind": "value", "value": 80 } }
        }})).unwrap();
        let result =
            serde_json::to_value(handle_request(&backend, &paths, &mut state, request)).unwrap();
        assert_eq!(result["payload"]["power"]["status"], "succeeded");
        assert_eq!(result["payload"]["graphics"]["status"], "failed");
        assert_eq!(result["payload"]["memory"]["status"], "unattempted");
        let contract: serde_json::Value = serde_json::from_str(include_str!(
            "../../src/test/contracts/profile-outcomes.json"
        ))
        .unwrap();
        assert_eq!(result["payload"], contract);
        let roundtrip: DaemonResponse = serde_json::from_value(result).unwrap();
        assert!(matches!(roundtrip, DaemonResponse::ProfileApplication(_)));
        backend.graphics_offset_error = None;
        backend.performance.borrow_mut().gpus[0]
            .clocks
            .memory
            .max_offset_mhz = Some(50);
        let retry: DaemonRequest =
            serde_json::from_value(serde_json::json!({ "type": "apply_profile", "payload": {
                "gpuId": "GPU-123", "settings": { "graphicsOffsetMHz": { "kind": "value", "value": 150 }, "memoryOffsetMHz": { "kind": "value", "value": 80 } }
            }}))
            .unwrap();
        let rejected =
            serde_json::to_value(handle_request(&backend, &paths, &mut state, retry.clone()))
                .unwrap();
        assert_eq!(rejected["payload"]["graphics"]["status"], "unattempted");
        assert!(rejected["payload"]["memory"]["error"]
            .as_str()
            .unwrap()
            .contains("-200 to 50"));
        backend.performance.borrow_mut().gpus[0]
            .clocks
            .memory
            .max_offset_mhz = Some(200);
        let success =
            serde_json::to_value(handle_request(&backend, &paths, &mut state, retry)).unwrap();
        assert_eq!(success["payload"]["graphics"]["status"], "succeeded");
        assert_eq!(success["payload"]["memory"]["status"], "succeeded");
        assert!(success["payload"]["power"].is_null());
        let DaemonResponse::Performance(actual) =
            handle_request(&backend, &paths, &mut state, DaemonRequest::GetPerformance)
        else {
            panic!("expected performance")
        };
        assert_eq!(actual.gpus[0].power.current_limit_watts, Some(450));
        assert_eq!(actual.gpus[0].clocks.graphics.current_offset_mhz, Some(150));
        assert_eq!(actual.gpus[0].clocks.memory.current_offset_mhz, Some(80));
        assert!(!paths.state_path.exists());
    }

    fn apply_profile_settings(
        backend: &MockBackend,
        settings: serde_json::Value,
    ) -> serde_json::Value {
        let directory = tempfile::tempdir().unwrap();
        let paths = DaemonPaths {
            state_path: directory.path().join("state.json"),
            ..test_paths()
        };
        let mut state = PersistedGpuState::default();
        let request =
            serde_json::from_value(serde_json::json!({ "type": "apply_profile", "payload": {
                "gpuId": "GPU-123", "settings": settings
            }}))
            .unwrap();
        let result =
            serde_json::to_value(handle_request(backend, &paths, &mut state, request)).unwrap();
        assert!(!paths.state_path.exists());
        result["payload"].clone()
    }

    #[test]
    fn explicit_defaults_follow_the_currently_reported_power_default_while_numbers_stay_fixed() {
        let backend = profile_backend();
        let defaults = serde_json::json!({
            "powerLimitWatts": { "kind": "default" },
            "graphicsOffsetMHz": { "kind": "default" },
            "memoryOffsetMHz": { "kind": "default" }
        });
        let fixed = serde_json::json!({ "powerLimitWatts": { "kind": "value", "value": 575 } });
        let result = apply_profile_settings(&backend, defaults.clone());
        for setting in ["power", "graphics", "memory"] {
            assert_eq!(result[setting]["status"], "succeeded", "{setting}");
        }
        let gpu = backend.performance.borrow().gpus[0].clone();
        assert_eq!(gpu.power.current_limit_watts, Some(575));
        assert_eq!(gpu.clocks.graphics.current_offset_mhz, Some(0));
        assert_eq!(gpu.clocks.memory.current_offset_mhz, Some(0));

        // A driver update changes the reported default.
        backend.performance.borrow_mut().gpus[0]
            .power
            .default_limit_watts = Some(520);
        apply_profile_settings(&backend, fixed);
        assert_eq!(
            backend.performance.borrow().gpus[0]
                .power
                .current_limit_watts,
            Some(575)
        );
        apply_profile_settings(&backend, defaults);
        assert_eq!(
            backend.performance.borrow().gpus[0]
                .power
                .current_limit_watts,
            Some(520)
        );
    }

    #[test]
    fn unavailable_or_out_of_range_defaults_block_every_included_instruction_before_writes() {
        for (invalid, expected) in [
            (
                "power default unavailable",
                "Power limit: the default is unavailable",
            ),
            (
                "power default above maximum",
                "Power limit: the default 650 is outside the current allowed range 300 to 600",
            ),
            (
                "offset range excludes zero",
                "Memory clock offset: the default 0 is outside the current allowed range 10 to 200",
            ),
            ("unsupported default", "Memory clock offset is unsupported"),
        ] {
            let backend = profile_backend();
            {
                let mut performance = backend.performance.borrow_mut();
                let gpu = &mut performance.gpus[0];
                match invalid {
                    "power default unavailable" => gpu.power.default_limit_watts = None,
                    "power default above maximum" => gpu.power.default_limit_watts = Some(650),
                    "offset range excludes zero" => gpu.clocks.memory.min_offset_mhz = Some(10),
                    "unsupported default" => {
                        gpu.clocks.memory.support = crate::gpu::ControlSupport::Unsupported
                    }
                    _ => unreachable!(),
                }
            }
            let before = backend.performance.borrow().clone();
            let result = apply_profile_settings(
                &backend,
                serde_json::json!({
                    "powerLimitWatts": { "kind": "default" },
                    "graphicsOffsetMHz": { "kind": "value", "value": 150 },
                    "memoryOffsetMHz": { "kind": "default" }
                }),
            );
            for setting in ["power", "graphics", "memory"] {
                assert_eq!(result[setting]["status"], "unattempted", "{invalid}");
                assert!(
                    result[setting]["error"]
                        .as_str()
                        .unwrap()
                        .contains(expected),
                    "{invalid}: {}",
                    result[setting]["error"]
                );
            }
            assert_eq!(*backend.performance.borrow(), before, "{invalid}");
        }
    }

    #[test]
    fn omitted_settings_need_no_default_resolution() {
        let backend = profile_backend();
        {
            let mut performance = backend.performance.borrow_mut();
            performance.gpus[0].power.default_limit_watts = None;
            performance.gpus[0].power.support = crate::gpu::ControlSupport::Unknown;
        }
        let result = apply_profile_settings(
            &backend,
            serde_json::json!({ "graphicsOffsetMHz": { "kind": "default" } }),
        );
        assert_eq!(result["graphics"]["status"], "succeeded");
        assert!(result["power"].is_null());
        assert_eq!(
            backend.performance.borrow().gpus[0]
                .power
                .current_limit_watts,
            Some(475)
        );
    }

    #[test]
    fn profile_settings_round_trip_omitted_numeric_and_default_instructions() {
        let request: ApplyGpuProfileRequest = serde_json::from_value(serde_json::json!({
            "gpuId": "GPU-123",
            "settings": {
                "powerLimitWatts": { "kind": "value", "value": 575 },
                "graphicsOffsetMHz": { "kind": "default" }
            }
        }))
        .unwrap();
        assert_eq!(
            request.settings.power_limit_watts,
            Some(ProfileSetting::Value { value: 575 })
        );
        assert_eq!(
            request.settings.graphics_offset_mhz,
            Some(ProfileSetting::Default {})
        );
        assert_eq!(request.settings.memory_offset_mhz, None);
        assert_eq!(
            serde_json::to_value(&request).unwrap()["settings"],
            serde_json::json!({
                "powerLimitWatts": { "kind": "value", "value": 575 },
                "graphicsOffsetMHz": { "kind": "default" }
            })
        );
    }

    #[test]
    fn live_offsets_report_partial_success_without_replacing_saved_configuration() {
        let mut backend = MockBackend::new();
        *backend.performance.borrow_mut() = serde_json::from_str(include_str!(
            "../../src/test/contracts/tuning-observations.json"
        ))
        .unwrap();
        backend.memory_offset_error = Some("Memory write failed".into());
        let mut state = PersistedGpuState::default();
        state.gpus.insert(
            "GPU-123".into(),
            PersistedGpuConfig {
                desired: DesiredGpuConfig {
                    power_limit_watts: Some(450),
                    gpu_clock_lock: Some(ClockRangeInput {
                        min_mhz: 1800,
                        max_mhz: 2400,
                    }),
                    ..DesiredGpuConfig::default()
                },
                last_apply: None,
            },
        );
        let saved = state.clone();
        let response = handle_request(
            &backend,
            &test_paths(),
            &mut state,
            DaemonRequest::SetClockOffsets(SetGpuClockOffsetsRequest {
                gpu_id: "GPU-123".into(),
                graphics_offset_mhz: Some(150),
                memory_offset_mhz: Some(80),
            }),
        );
        let json = serde_json::to_value(response).unwrap();
        assert_eq!(json["payload"]["graphics"]["status"], "succeeded");
        assert_eq!(json["payload"]["memory"]["status"], "failed");
        assert_eq!(json["payload"]["memory"]["error"], "Memory write failed");
        let contract: serde_json::Value = serde_json::from_str(include_str!(
            "../../src/test/contracts/tuning-outcomes.json"
        ))
        .unwrap();
        assert_eq!(json["payload"], contract);
        let roundtrip: DaemonResponse = serde_json::from_value(json).unwrap();
        assert!(matches!(roundtrip, DaemonResponse::ClockOffsets(_)));
        let DaemonResponse::Performance(actual) = handle_request(
            &backend,
            &test_paths(),
            &mut state,
            DaemonRequest::GetPerformance,
        ) else {
            panic!("expected performance")
        };
        assert_eq!(actual.gpus[0].clocks.graphics.current_offset_mhz, Some(150));
        assert_eq!(state, saved);
    }

    #[test]
    fn live_offsets_stop_after_failure_and_leave_later_setting_unattempted() {
        let mut backend = MockBackend::new();
        *backend.performance.borrow_mut() = serde_json::from_str(include_str!(
            "../../src/test/contracts/tuning-observations.json"
        ))
        .unwrap();
        backend.command_error = Some("GPU unavailable".into());
        let mut state = PersistedGpuState::default();
        let response = handle_request(
            &backend,
            &test_paths(),
            &mut state,
            DaemonRequest::SetClockOffsets(SetGpuClockOffsetsRequest {
                gpu_id: "GPU-123".into(),
                graphics_offset_mhz: Some(150),
                memory_offset_mhz: Some(80),
            }),
        );
        let json = serde_json::to_value(response).unwrap();
        assert_eq!(json["payload"]["graphics"]["status"], "failed");
        assert_eq!(json["payload"]["memory"]["status"], "unattempted");
        let DaemonResponse::Performance(actual) = handle_request(
            &backend,
            &test_paths(),
            &mut state,
            DaemonRequest::GetPerformance,
        ) else {
            panic!("expected performance")
        };
        assert_eq!(actual.gpus[0].clocks.graphics.current_offset_mhz, None);
        assert_eq!(actual.gpus[0].clocks.memory.current_offset_mhz, None);
    }

    #[test]
    fn live_reset_reports_partial_outcomes_and_memory_only_retry_leaves_graphics_alone() {
        let mut backend = MockBackend::new();
        *backend.performance.borrow_mut() = serde_json::from_str(include_str!(
            "../../src/test/contracts/tuning-observations.json"
        ))
        .unwrap();
        {
            let mut performance = backend.performance.borrow_mut();
            let clocks = &mut performance.gpus[0].clocks;
            clocks.graphics.support = crate::gpu::ControlSupport::Supported;
            clocks.memory.support = crate::gpu::ControlSupport::Supported;
            clocks.graphics.current_offset_mhz = Some(150);
            clocks.memory.current_offset_mhz = Some(80);
        }
        backend.memory_offset_error = Some("Memory write failed".into());
        let mut state = PersistedGpuState::default();
        let response = handle_request(
            &backend,
            &test_paths(),
            &mut state,
            DaemonRequest::ResetClockOffsets(GpuTargetRequest {
                gpu_id: "GPU-123".into(),
            }),
        );
        let json = serde_json::to_value(response).unwrap();
        assert_eq!(json["payload"]["graphics"]["status"], "succeeded");
        assert_eq!(json["payload"]["memory"]["status"], "failed");
        backend.memory_offset_error = None;
        let response = handle_request(
            &backend,
            &test_paths(),
            &mut state,
            DaemonRequest::SetClockOffsets(SetGpuClockOffsetsRequest {
                gpu_id: "GPU-123".into(),
                graphics_offset_mhz: None,
                memory_offset_mhz: Some(0),
            }),
        );
        let json = serde_json::to_value(response).unwrap();
        assert!(json["payload"]["graphics"].is_null());
        assert_eq!(json["payload"]["memory"]["status"], "succeeded");
        let DaemonResponse::Performance(actual) = handle_request(
            &backend,
            &test_paths(),
            &mut state,
            DaemonRequest::GetPerformance,
        ) else {
            panic!("expected performance")
        };
        assert_eq!(actual.gpus[0].clocks.graphics.current_offset_mhz, Some(0));
        assert_eq!(actual.gpus[0].clocks.memory.current_offset_mhz, Some(0));
    }

    #[test]
    fn performance_reads_preserve_tuning_states_and_recover_without_changing_saved_configuration() {
        let backend = MockBackend::new();
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../src/test/contracts/tuning-observations.json"
        ))
        .unwrap();
        *backend.performance.borrow_mut() = serde_json::from_value(fixture.clone()).unwrap();
        let mut state = PersistedGpuState::default();
        state.gpus.insert(
            "GPU-123".into(),
            PersistedGpuConfig {
                desired: DesiredGpuConfig {
                    power_limit_watts: Some(400),
                    ..Default::default()
                },
                last_apply: None,
            },
        );
        let original_state = state.clone();
        let temp = tempfile::tempdir().unwrap();
        let paths = DaemonPaths {
            state_path: temp.path().join("state.json"),
            ..test_paths()
        };
        let response = handle_request(&backend, &paths, &mut state, DaemonRequest::GetPerformance);
        let wire = serde_json::to_value(&response).unwrap();
        assert_eq!(wire["type"], "performance");
        assert_eq!(wire["payload"], fixture);
        assert_eq!(
            serde_json::from_value::<DaemonResponse>(wire).unwrap(),
            response
        );

        {
            let mut performance = backend.performance.borrow_mut();
            let gpu = &mut performance.gpus[0];
            gpu.power.current_limit_watts = Some(450);
            gpu.power.current_value_error = None;
            gpu.clocks.graphics.support = crate::gpu::ControlSupport::Supported;
            gpu.clocks.graphics.reason = None;
            gpu.clocks.graphics.current_offset_mhz = Some(80);
            gpu.clocks.graphics.min_offset_mhz = Some(-200);
            gpu.clocks.graphics.max_offset_mhz = Some(200);
        }
        let response = handle_request(&backend, &paths, &mut state, DaemonRequest::GetPerformance);
        let DaemonResponse::Performance(performance) = response else {
            panic!("expected performance")
        };
        assert_eq!(performance.gpus[0].power.current_limit_watts, Some(450));
        assert_eq!(performance.gpus[0].power.current_value_error, None);
        assert_eq!(
            performance.gpus[0].clocks.graphics.support,
            crate::gpu::ControlSupport::Supported
        );
        assert_eq!(
            performance.gpus[0].clocks.memory.support,
            crate::gpu::ControlSupport::Unsupported
        );
        assert_eq!(state, original_state);
        assert!(backend.applied_requests.borrow().is_empty());
        assert!(!paths.state_path.exists());
    }

    #[test]
    fn apply_request_updates_saved_state() {
        let backend = MockBackend::new();
        let mut state = PersistedGpuState::default();

        let response = handle_request(
            &backend,
            &test_paths(),
            &mut state,
            DaemonRequest::ApplyConfig(ApplyGpuConfigRequest {
                gpu_id: Some("GPU-123".into()),
                power_limit_watts: Some(250),
                gpu_clock_lock: Some(ClockRangeInput {
                    min_mhz: 1800,
                    max_mhz: 2200,
                }),
                ..Default::default()
            }),
        );

        let DaemonResponse::Mutation(response) = response else {
            panic!("expected mutation response");
        };

        assert!(response.success);
        assert_eq!(
            state.gpus["GPU-123"].desired,
            DesiredGpuConfig {
                power_limit_watts: Some(250),
                gpu_clock_lock: Some(ClockRangeInput {
                    min_mhz: 1800,
                    max_mhz: 2200,
                }),
                ..Default::default()
            }
        );
    }

    #[test]
    fn failed_apply_keeps_existing_desired_state_and_records_error() {
        let backend = MockBackend {
            should_fail_apply: true,
            ..MockBackend::new()
        };
        let mut state = PersistedGpuState::default();
        state.gpus.insert(
            "GPU-123".into(),
            PersistedGpuConfig {
                desired: DesiredGpuConfig {
                    power_limit_watts: Some(225),
                    ..Default::default()
                },
                last_apply: None,
            },
        );

        let response = handle_request(
            &backend,
            &test_paths(),
            &mut state,
            DaemonRequest::ApplyConfig(ApplyGpuConfigRequest {
                gpu_id: Some("GPU-123".into()),
                power_limit_watts: Some(250),
                ..Default::default()
            }),
        );

        let DaemonResponse::Mutation(response) = response else {
            panic!("expected mutation response");
        };

        assert!(!response.success);
        assert_eq!(state.gpus["GPU-123"].desired.power_limit_watts, Some(225));
        assert_eq!(
            state.gpus["GPU-123"]
                .last_apply
                .as_ref()
                .and_then(|record| record.message.as_deref()),
            Some("mock apply failed")
        );
    }

    #[test]
    fn get_daemon_status_returns_connected_socket_info() {
        let backend = MockBackend::new();
        let mut state = PersistedGpuState::default();

        let response = handle_request(
            &backend,
            &test_paths(),
            &mut state,
            DaemonRequest::GetDaemonStatus,
        );

        let DaemonResponse::DaemonStatus(status) = response else {
            panic!("expected daemon status response");
        };

        assert!(status.reachable);
        assert!(status.managed_by_daemon);
        assert!(status.socket_path.contains("prismd-test.sock"));
    }

    #[test]
    fn power_limit_commands_return_command_responses_without_mutating_state() {
        let backend = MockBackend {
            command_error: Some("mock permission denied".into()),
            ..MockBackend::new()
        };
        let mut state = PersistedGpuState::default();

        let response = handle_request(
            &backend,
            &test_paths(),
            &mut state,
            DaemonRequest::SetPowerLimit(SetGpuPowerLimitRequest {
                gpu_id: "GPU-123".into(),
                limit_watts: 250,
            }),
        );

        let DaemonResponse::Command(response) = response else {
            panic!("expected command response");
        };

        assert!(!response.success);
        assert_eq!(response.error.as_deref(), Some("mock permission denied"));
        assert!(state.gpus.is_empty());
    }

    #[test]
    fn reset_request_clears_saved_state() {
        let backend = MockBackend::new();
        let mut state = PersistedGpuState::default();
        state.upsert_desired(
            "GPU-123",
            DesiredGpuConfig {
                power_limit_watts: Some(225),
                ..Default::default()
            },
        );

        let response = handle_request(
            &backend,
            &test_paths(),
            &mut state,
            DaemonRequest::ResetConfig(crate::gpu::ResetGpuConfigRequest {
                gpu_id: "GPU-123".into(),
            }),
        );

        let DaemonResponse::Mutation(response) = response else {
            panic!("expected mutation response");
        };

        assert!(response.success);
        assert!(state.gpus.is_empty());
    }

    #[test]
    fn failed_reset_preserves_saved_state_and_records_error() {
        let backend = MockBackend {
            should_fail_reset: true,
            ..MockBackend::new()
        };
        let mut state = PersistedGpuState::default();
        state.upsert_desired(
            "GPU-123",
            DesiredGpuConfig {
                power_limit_watts: Some(225),
                ..Default::default()
            },
        );

        let response = handle_request(
            &backend,
            &test_paths(),
            &mut state,
            DaemonRequest::ResetConfig(crate::gpu::ResetGpuConfigRequest {
                gpu_id: "GPU-123".into(),
            }),
        );

        let DaemonResponse::Mutation(response) = response else {
            panic!("expected mutation response");
        };

        assert!(!response.success);
        assert_eq!(state.gpus["GPU-123"].desired.power_limit_watts, Some(225));
        assert_eq!(
            state.gpus["GPU-123"]
                .last_apply
                .as_ref()
                .and_then(|record| record.message.as_deref()),
            Some("mock reset failed")
        );
    }

    #[test]
    fn apply_request_without_gpu_id_returns_invalid_argument_without_saving_state() {
        let backend = MockBackend::new();
        let mut state = PersistedGpuState::default();

        let response = handle_request(
            &backend,
            &test_paths(),
            &mut state,
            DaemonRequest::ApplyConfig(ApplyGpuConfigRequest {
                power_limit_watts: Some(250),
                ..Default::default()
            }),
        );

        let DaemonResponse::Mutation(response) = response else {
            panic!("expected mutation response");
        };

        assert!(!response.success);
        assert_eq!(
            response.error.expect("error").code,
            GpuCommandErrorCode::InvalidArgument
        );
        assert!(state.gpus.is_empty());
        assert!(backend.applied_requests.borrow().is_empty());
    }

    #[test]
    fn reapply_saved_config_applies_non_empty_saved_configs() {
        let backend = MockBackend::new();
        let mut state = PersistedGpuState::default();
        state.upsert_desired(
            "GPU-123",
            DesiredGpuConfig {
                power_limit_watts: Some(225),
                ..Default::default()
            },
        );
        state.gpus.insert(
            "GPU-empty".into(),
            PersistedGpuConfig {
                desired: DesiredGpuConfig::default(),
                last_apply: Some(crate::gpu::GpuApplyRecord::failure("old failure")),
            },
        );

        let response = handle_request(
            &backend,
            &test_paths(),
            &mut state,
            DaemonRequest::ReapplySavedConfig,
        );

        let DaemonResponse::Mutation(response) = response else {
            panic!("expected mutation response");
        };

        assert!(response.success);
        let applied = backend.applied_requests.borrow();
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].gpu_id.as_deref(), Some("GPU-123"));
        assert_eq!(applied[0].power_limit_watts, Some(225));
        assert_eq!(
            state.gpus["GPU-123"]
                .last_apply
                .as_ref()
                .map(|record| record.success),
            Some(true)
        );
    }

    #[test]
    fn command_success_returns_success_response() {
        let backend = MockBackend::new();
        let mut state = PersistedGpuState::default();

        let response = handle_request(
            &backend,
            &test_paths(),
            &mut state,
            DaemonRequest::SetPowerLimit(SetGpuPowerLimitRequest {
                gpu_id: "GPU-123".into(),
                limit_watts: 250,
            }),
        );

        let DaemonResponse::Command(response) = response else {
            panic!("expected command response");
        };

        assert!(response.success);
        assert!(response.error.is_none());
    }
}
