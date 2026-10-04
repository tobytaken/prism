use crate::gpu::{
    apply_gpu_config, collect_inventory_with_context, collect_performance, reset_gpu_config,
    reset_gpu_power_limit, set_gpu_clock_offsets, set_gpu_fan_mode_auto, set_gpu_fan_speed,
    set_gpu_power_limit, ApplyGpuConfigRequest, GpuCommandError, GpuInventoryResponse,
    GpuPerformanceResponse, GpuTargetRequest, InventoryContext, PersistedGpuState,
    SetGpuClockOffsetsRequest, SetGpuFanModeAutoRequest, SetGpuFanSpeedRequest,
    SetGpuPowerLimitRequest,
};

/// The GPUs the daemon reads and tunes. Calls arrive from several threads at once:
/// reads from any connection, and writes from one GPU operation queue per GPU.
pub trait DaemonBackend: Send + Sync {
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

pub struct RealDaemonBackend;

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
