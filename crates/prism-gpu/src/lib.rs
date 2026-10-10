pub mod cooling;
pub mod fan_curve;
mod nvidia;

use serde::{Deserialize, Serialize};

use crate::cooling::CoolingControl;
use crate::fan_curve::FanCurveSupport;

use nvidia::{
    NvmlNvidiaProvider, collect_nvidia_performance, reset_nvidia_clock_offsets,
    reset_nvidia_power_limit, set_nvidia_clock_offsets, set_nvidia_fan_auto, set_nvidia_fan_speed,
    set_nvidia_power_limit,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct GpuInventoryResponse {
    pub gpus: Vec<GpuDevice>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GpuDevice {
    pub id: String,
    pub vendor: GpuVendor,
    pub name: String,
    pub architecture: Option<String>,
    pub driver_version: Option<String>,
    pub firmware_version: Option<String>,
    pub bus_id: Option<String>,
    pub memory: MemoryInfo,
    pub compute: ComputeInfo,
    pub pcie: PcieInfo,
    pub power: PowerInfo,
    pub clocks: ClockInfo,
    pub control: GpuControlInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryInfo {
    pub total_mib: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComputeInfo {
    pub api: Option<ComputeApi>,
    pub api_version: Option<String>,
    pub capability: Option<String>,
    pub core_count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PcieInfo {
    pub gen_max: Option<u32>,
    pub width_max: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PowerInfo {
    pub enforced_limit_watts: Option<u32>,
    pub default_limit_watts: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClockInfo {
    pub graphics_current_mhz: Option<u32>,
    pub graphics_max_mhz: Option<u32>,
    pub sm_max_mhz: Option<u32>,
    pub memory_max_mhz: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GpuPerformanceResponse {
    pub gpus: Vec<GpuPerformanceDevice>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GpuPerformanceDevice {
    pub id: String,
    #[serde(default)]
    pub stable_id: Option<String>,
    pub vendor: GpuVendor,
    pub name: String,
    pub architecture: Option<String>,
    pub bus_id: Option<String>,
    pub power: PowerControlInfo,
    pub cooling: CoolingInfo,
    pub clocks: ClockOffsetsInfo,
    pub telemetry: GpuTelemetryInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GpuTelemetryInfo {
    pub power_draw_watts: Option<u32>,
    pub core_clock_mhz: Option<u32>,
    pub memory_clock_mhz: Option<u32>,
    pub fan_speed_percent: Option<u32>,
    pub voltage_millivolts: Option<u32>,
    pub temperature_c: Option<u32>,
    pub gpu_utilization_percent: Option<u32>,
    pub memory_used_mib: Option<u64>,
    pub memory_total_mib: Option<u64>,
    pub performance_state: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ControlSupport {
    Supported,
    Unsupported,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PowerControlInfo {
    #[serde(default)]
    pub support: ControlSupport,
    pub reason: Option<String>,
    pub current_value_error: Option<String>,
    pub current_draw_watts: Option<u32>,
    pub current_limit_watts: Option<u32>,
    pub default_limit_watts: Option<u32>,
    pub enforced_limit_watts: Option<u32>,
    pub min_limit_watts: Option<u32>,
    pub max_limit_watts: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CoolingInfo {
    pub is_supported: bool,
    pub unsupported_reason: Option<String>,
    pub fan_count: u32,
    pub fans: Vec<FanControlInfo>,
    /// The GPU's maximum operating temperature in °C, known only when the GPU reports a
    /// trustworthy absolute temperature.
    #[serde(default)]
    pub max_operating_temperature_c: Option<u32>,
    /// Why the maximum operating temperature is unknown.
    #[serde(default)]
    pub max_operating_temperature_reason: Option<String>,
    /// Whether one shared fan curve can control every fan, as judged by whoever follows the
    /// curves; unknown when the readings came straight from the GPU.
    #[serde(default)]
    pub curve_support: Option<FanCurveSupport>,
    /// Prism's control of the fans, as reported by whoever follows the curves; unknown when the
    /// readings came straight from the GPU.
    #[serde(default)]
    pub control: Option<CoolingControl>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FanControlInfo {
    pub index: u32,
    pub current_speed_percent: Option<u32>,
    pub min_speed_percent: Option<u32>,
    pub max_speed_percent: Option<u32>,
    pub mode: Option<FanControlMode>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FanControlMode {
    Auto,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClockOffsetsInfo {
    pub graphics: ClockOffsetInfo,
    pub memory: ClockOffsetInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClockOffsetInfo {
    #[serde(default)]
    pub support: ControlSupport,
    pub reason: Option<String>,
    pub current_value_error: Option<String>,
    pub current_offset_mhz: Option<i32>,
    pub min_offset_mhz: Option<i32>,
    pub max_offset_mhz: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetGpuPowerLimitRequest {
    pub gpu_id: String,
    pub limit_watts: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GpuTargetRequest {
    pub gpu_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetGpuFanSpeedRequest {
    pub gpu_id: String,
    pub fan_index: u32,
    pub speed_percent: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetGpuFanModeAutoRequest {
    pub gpu_id: String,
    pub fan_index: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetGpuClockOffsetsRequest {
    pub gpu_id: String,
    pub graphics_offset_mhz: Option<i32>,
    pub memory_offset_mhz: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct GpuControlInfo {
    pub supported_features: GpuSupportedFeatures,
    pub power_limit: PowerLimitControl,
    pub gpu_clock_lock: ClockLockControl,
    pub memory_clock_lock: ClockLockControl,
    pub offsets: OffsetControl,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct GpuSupportedFeatures {
    pub power_limit: bool,
    pub gpu_clock_lock: bool,
    pub memory_clock_lock: bool,
    pub graphics_offset: bool,
    pub memory_offset: bool,
    pub reset: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PowerLimitControl {
    pub current_watts: Option<u32>,
    pub default_watts: Option<u32>,
    pub min_watts: Option<u32>,
    pub max_watts: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ClockLockControl {
    pub current_min_mhz: Option<u32>,
    pub current_max_mhz: Option<u32>,
    pub min_allowed_mhz: Option<u32>,
    pub max_allowed_mhz: Option<u32>,
    pub supported: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct OffsetControl {
    pub graphics_mhz: Option<i32>,
    pub memory_mhz: Option<i32>,
    pub min_graphics_mhz: Option<i32>,
    pub max_graphics_mhz: Option<i32>,
    pub min_memory_mhz: Option<i32>,
    pub max_memory_mhz: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TuningOutcomeStatus {
    Succeeded,
    Failed,
    Unattempted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TuningOutcome {
    pub status: TuningOutcomeStatus,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GpuCommandError {
    pub code: GpuCommandErrorCode,
    pub message: String,
}

impl GpuCommandError {
    pub fn new(code: GpuCommandErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GpuCommandErrorCode {
    NoPermission,
    InvalidArgument,
    NotSupported,
    GpuLost,
    NvmlUnavailable,
    Io,
    Internal,
    NotFound,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GpuVendor {
    Nvidia,
    Amd,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ComputeApi {
    Cuda,
    Rocm,
}

#[derive(Debug, Default)]
pub(crate) struct ProviderOutput {
    gpus: Vec<GpuDevice>,
    warnings: Vec<String>,
}

impl ProviderOutput {
    fn warning(message: impl Into<String>) -> Self {
        Self {
            gpus: Vec::new(),
            warnings: vec![message.into()],
        }
    }

    fn append(&mut self, other: Self) {
        self.gpus.extend(other.gpus);
        self.warnings.extend(other.warnings);
    }
}

pub(crate) trait GpuInventoryProvider {
    fn collect(&self) -> ProviderOutput;
}

#[derive(Debug, Default)]
pub(crate) struct PerformanceProviderOutput {
    gpus: Vec<GpuPerformanceDevice>,
    warnings: Vec<String>,
}

impl PerformanceProviderOutput {
    fn warning(message: impl Into<String>) -> Self {
        Self {
            gpus: Vec::new(),
            warnings: vec![message.into()],
        }
    }
}

pub fn collect_inventory() -> GpuInventoryResponse {
    let providers: [Box<dyn GpuInventoryProvider>; 1] = [Box::new(NvmlNvidiaProvider)];
    let mut output = ProviderOutput::default();

    for provider in providers {
        output.append(provider.collect());
    }

    GpuInventoryResponse {
        gpus: output.gpus,
        warnings: output.warnings,
    }
}

pub fn collect_performance() -> GpuPerformanceResponse {
    let output = collect_nvidia_performance();

    GpuPerformanceResponse {
        gpus: output.gpus,
        warnings: output.warnings,
    }
}

pub fn set_gpu_power_limit(request: SetGpuPowerLimitRequest) -> Result<(), String> {
    set_nvidia_power_limit(request)
}

pub fn reset_gpu_power_limit(request: GpuTargetRequest) -> Result<(), String> {
    reset_nvidia_power_limit(request)
}

pub fn set_gpu_fan_speed(request: SetGpuFanSpeedRequest) -> Result<(), String> {
    set_nvidia_fan_speed(request)
}

pub fn set_gpu_fan_mode_auto(request: SetGpuFanModeAutoRequest) -> Result<(), String> {
    set_nvidia_fan_auto(request)
}

pub fn set_gpu_clock_offsets(request: SetGpuClockOffsetsRequest) -> Result<(), String> {
    set_nvidia_clock_offsets(request)
}

pub fn reset_gpu_clock_offsets(request: GpuTargetRequest) -> Result<(), String> {
    reset_nvidia_clock_offsets(request)
}

#[cfg(test)]
mod tests {
    use super::{
        ClockInfo, ClockLockControl, ClockOffsetInfo, ClockOffsetsInfo, ComputeApi, ComputeInfo,
        ControlSupport, CoolingInfo, FanControlInfo, FanControlMode, GpuControlInfo, GpuDevice,
        GpuInventoryResponse, GpuPerformanceDevice, GpuPerformanceResponse, GpuSupportedFeatures,
        GpuTelemetryInfo, GpuVendor, MemoryInfo, OffsetControl, PcieInfo, PowerControlInfo,
        PowerInfo, PowerLimitControl,
    };

    #[test]
    fn inventory_response_serializes_rust_field_names() {
        let response = GpuInventoryResponse {
            gpus: vec![GpuDevice {
                id: "gpu-0".into(),
                vendor: GpuVendor::Nvidia,
                name: "RTX".into(),
                architecture: Some("Blackwell".into()),
                driver_version: Some("595.58.03".into()),
                firmware_version: Some("98.02".into()),
                bus_id: Some("0000:01:00.0".into()),
                memory: MemoryInfo {
                    total_mib: Some(32_768),
                },
                compute: ComputeInfo {
                    api: Some(ComputeApi::Cuda),
                    api_version: Some("13.2".into()),
                    capability: Some("12.0".into()),
                    core_count: Some(21_760),
                },
                pcie: PcieInfo {
                    gen_max: Some(5),
                    width_max: Some(16),
                },
                power: PowerInfo {
                    enforced_limit_watts: Some(475),
                    default_limit_watts: Some(575),
                },
                clocks: ClockInfo {
                    graphics_current_mhz: Some(1_365),
                    graphics_max_mhz: Some(3_300),
                    sm_max_mhz: Some(3_300),
                    memory_max_mhz: Some(14_001),
                },
                control: GpuControlInfo {
                    supported_features: GpuSupportedFeatures {
                        power_limit: true,
                        gpu_clock_lock: true,
                        memory_clock_lock: true,
                        graphics_offset: true,
                        memory_offset: true,
                        reset: true,
                    },
                    power_limit: PowerLimitControl {
                        current_watts: Some(475),
                        default_watts: Some(575),
                        min_watts: Some(300),
                        max_watts: Some(575),
                    },
                    gpu_clock_lock: ClockLockControl {
                        current_min_mhz: Some(1800),
                        current_max_mhz: Some(2500),
                        min_allowed_mhz: Some(1500),
                        max_allowed_mhz: Some(3000),
                        supported: true,
                    },
                    memory_clock_lock: ClockLockControl {
                        current_min_mhz: Some(1000),
                        current_max_mhz: Some(1100),
                        min_allowed_mhz: Some(800),
                        max_allowed_mhz: Some(1200),
                        supported: true,
                    },
                    offsets: OffsetControl {
                        graphics_mhz: Some(100),
                        memory_mhz: Some(250),
                        min_graphics_mhz: Some(-200),
                        max_graphics_mhz: Some(200),
                        min_memory_mhz: Some(-500),
                        max_memory_mhz: Some(500),
                    },
                    warnings: vec!["saved values were reapplied".into()],
                },
            }],
            warnings: vec![],
        };

        let value = serde_json::to_value(response).expect("serialize response");

        assert_eq!(value["gpus"][0]["driver_version"], "595.58.03");
        assert_eq!(value["gpus"][0]["memory"]["total_mib"], 32_768);
        assert_eq!(value["gpus"][0]["compute"]["api"], "cuda");
        assert_eq!(value["gpus"][0]["clocks"]["graphics_current_mhz"], 1_365);
        assert_eq!(
            value["gpus"][0]["control"]["power_limit"].get("desired_watts"),
            None
        );
    }

    #[test]
    fn performance_response_serializes_rust_field_names() {
        let response = GpuPerformanceResponse {
            gpus: vec![GpuPerformanceDevice {
                id: "GPU-123".into(),
                stable_id: Some("GPU-123".into()),
                vendor: GpuVendor::Nvidia,
                name: "RTX".into(),
                architecture: Some("Blackwell".into()),
                bus_id: Some("0000:01:00.0".into()),
                power: PowerControlInfo {
                    support: ControlSupport::Supported,
                    reason: None,
                    current_value_error: None,
                    current_draw_watts: Some(312),
                    current_limit_watts: Some(475),
                    default_limit_watts: Some(575),
                    enforced_limit_watts: Some(475),
                    min_limit_watts: Some(400),
                    max_limit_watts: Some(600),
                },
                cooling: CoolingInfo {
                    is_supported: true,
                    unsupported_reason: None,
                    fan_count: 1,
                    fans: vec![FanControlInfo {
                        index: 0,
                        current_speed_percent: Some(42),
                        min_speed_percent: Some(30),
                        max_speed_percent: Some(100),
                        mode: Some(FanControlMode::Auto),
                    }],
                    max_operating_temperature_c: Some(90),
                    max_operating_temperature_reason: None,
                    curve_support: None,
                    control: None,
                },
                clocks: ClockOffsetsInfo {
                    graphics: ClockOffsetInfo {
                        support: ControlSupport::Supported,
                        reason: None,
                        current_value_error: None,
                        current_offset_mhz: Some(120),
                        min_offset_mhz: Some(-200),
                        max_offset_mhz: Some(200),
                    },
                    memory: ClockOffsetInfo {
                        support: ControlSupport::Supported,
                        reason: None,
                        current_value_error: None,
                        current_offset_mhz: Some(300),
                        min_offset_mhz: Some(-500),
                        max_offset_mhz: Some(500),
                    },
                },
                telemetry: GpuTelemetryInfo {
                    power_draw_watts: Some(312),
                    core_clock_mhz: Some(2_550),
                    memory_clock_mhz: Some(14_001),
                    fan_speed_percent: Some(42),
                    voltage_millivolts: None,
                    temperature_c: Some(71),
                    gpu_utilization_percent: Some(96),
                    memory_used_mib: Some(10_240),
                    memory_total_mib: Some(32_768),
                    performance_state: Some("P0".into()),
                },
            }],
            warnings: vec![],
        };

        let value = serde_json::to_value(response).expect("serialize response");

        assert_eq!(value["gpus"][0]["power"]["support"], "supported");
        assert_eq!(value["gpus"][0]["stable_id"], "GPU-123");
        assert_eq!(
            value["gpus"][0]["clocks"]["graphics"]["support"],
            "supported"
        );
        assert_eq!(value["gpus"][0]["power"]["current_draw_watts"], 312);
        assert_eq!(value["gpus"][0]["power"]["current_limit_watts"], 475);
        assert_eq!(value["gpus"][0]["cooling"]["fans"][0]["mode"], "auto");
        assert_eq!(
            value["gpus"][0]["cooling"]["max_operating_temperature_c"],
            90
        );
        assert_eq!(
            value["gpus"][0]["clocks"]["graphics"]["max_offset_mhz"],
            200
        );
        assert_eq!(value["gpus"][0]["telemetry"]["core_clock_mhz"], 2_550);
        assert_eq!(value["gpus"][0]["telemetry"]["performance_state"], "P0");
    }
}
