use prism_gpu::{
    GpuInventoryResponse, GpuPerformanceResponse, GpuProviders, GpuTargetRequest,
    SetGpuClockOffsetsRequest, SetGpuFanModeAutoRequest, SetGpuFanSpeedRequest,
    SetGpuPowerLimitRequest, reset_gpu_power_limit, set_gpu_clock_offsets, set_gpu_fan_mode_auto,
    set_gpu_fan_speed, set_gpu_power_limit,
};

/// The GPUs the daemon reads and tunes. Calls arrive from several threads at once:
/// reads from any connection, and writes from one GPU operation queue per GPU.
pub trait DaemonBackend: Send + Sync {
    fn inventory(&self) -> GpuInventoryResponse;
    fn performance(&self) -> GpuPerformanceResponse;
    fn set_power_limit(&self, request: SetGpuPowerLimitRequest) -> Result<(), String>;
    fn reset_power_limit(&self, request: GpuTargetRequest) -> Result<(), String>;
    fn set_fan_speed(&self, request: SetGpuFanSpeedRequest) -> Result<(), String>;
    fn set_fan_mode_auto(&self, request: SetGpuFanModeAutoRequest) -> Result<(), String>;
    fn set_clock_offsets(&self, request: SetGpuClockOffsetsRequest) -> Result<(), String>;
}

/// The host's NVIDIA GPUs, read and tuned through NVML.
#[derive(Default)]
pub struct RealDaemonBackend {
    providers: GpuProviders,
}

impl DaemonBackend for RealDaemonBackend {
    fn inventory(&self) -> GpuInventoryResponse {
        self.providers.inventory()
    }

    fn performance(&self) -> GpuPerformanceResponse {
        self.providers.performance()
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
