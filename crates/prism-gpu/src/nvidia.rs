use std::collections::BTreeSet;
use std::ops::RangeInclusive;

use nvml_wrapper::enum_wrappers::device::{
    Clock, PerformanceState, TemperatureSensor, TemperatureThreshold,
};
use nvml_wrapper::enums::device::{DeviceArchitecture, FanControlPolicy, SampleValue};
use nvml_wrapper::error::NvmlError;
use nvml_wrapper::struct_wrappers::device::MemoryInfo as NvmlMemoryInfo;
use nvml_wrapper::structs::device::FieldId;
use nvml_wrapper::{Device, Nvml, cuda_driver_version_major, cuda_driver_version_minor};
use once_cell::sync::OnceCell;
use thiserror::Error;

use super::{
    ClockInfo, ClockLockControl, ClockOffsetInfo, ClockOffsetsInfo, ComputeApi, ComputeInfo,
    ControlSupport, CoolingInfo, FanControlInfo, FanControlMode, GpuControlInfo, GpuDevice,
    GpuInventoryProvider, GpuPerformanceDevice, GpuSupportedFeatures, GpuTargetRequest,
    GpuTelemetryInfo, GpuVendor, MemoryInfo, OffsetControl, PcieInfo, PerformanceProviderOutput,
    PowerControlInfo, PowerInfo, PowerLimitControl, ProviderOutput, SetGpuClockOffsetsRequest,
    SetGpuFanModeAutoRequest, SetGpuFanSpeedRequest, SetGpuPowerLimitRequest,
};

const OFFSET_POWER_STATE: PerformanceState = PerformanceState::Zero;
/// NVML_FI_DEV_TEMPERATURE_GPU_MAX_TLIMIT: the maximum operating temperature as an offset from
/// T.Limit, which is a margin rather than a temperature.
const GPU_MAX_TLIMIT_FIELD: u32 = 196;
/// Absolute maximum operating temperatures outside this range are not credible GPU readings,
/// such as a T.Limit offset or an unset value read as degrees Celsius.
const PLAUSIBLE_MAX_OPERATING_TEMPERATURE_C: RangeInclusive<u32> = 50..=125;
static NVML: OnceCell<Nvml> = OnceCell::new();

pub struct NvmlNvidiaProvider;

#[derive(Debug, Error)]
enum NvidiaProviderError {
    #[error("unable to initialize NVML: {0}")]
    NvmlInit(#[source] NvmlError),
    #[error("unable to query NVIDIA device count: {0}")]
    DeviceCount(#[source] NvmlError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NvidiaSnapshot {
    id: String,
    name: String,
    architecture: Option<String>,
    driver_version: Option<String>,
    firmware_version: Option<String>,
    bus_id: Option<String>,
    memory_total_bytes: Option<u64>,
    compute_api_version: Option<String>,
    compute_capability: Option<String>,
    core_count: Option<u32>,
    pcie_gen_max: Option<u32>,
    pcie_width_max: Option<u32>,
    power_management_limit_mw: Option<u32>,
    enforced_power_limit_mw: Option<u32>,
    default_power_limit_mw: Option<u32>,
    min_power_limit_mw: Option<u32>,
    max_power_limit_mw: Option<u32>,
    graphics_current_mhz: Option<u32>,
    graphics_max_mhz: Option<u32>,
    sm_max_mhz: Option<u32>,
    memory_max_mhz: Option<u32>,
    supported_gpu_locked_clocks_mhz: Vec<u32>,
    supported_memory_locked_clocks_mhz: Vec<u32>,
    graphics_offset: Option<ClockOffsetSnapshot>,
    memory_offset: Option<ClockOffsetSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClockOffsetSnapshot {
    current_mhz: i32,
    min_mhz: i32,
    max_mhz: i32,
}

/// What NVML reports about a GPU's maximum operating temperature.
struct ThermalLimitReadings {
    /// NVML_TEMPERATURE_THRESHOLD_GPU_MAX, an absolute temperature in °C.
    gpu_max_threshold: Result<u32, NvmlError>,
    /// An offset from T.Limit, present on GPUs that report thermal limits as margins.
    gpu_max_tlimit_offset: Option<i64>,
}

struct TelemetryQueryResults {
    core_clock_mhz: Result<u32, NvmlError>,
    memory_clock_mhz: Result<u32, NvmlError>,
    temperature_c: Result<u32, NvmlError>,
    gpu_utilization_percent: Result<u32, NvmlError>,
    memory_info: Result<NvmlMemoryInfo, NvmlError>,
    performance_state: Result<PerformanceState, NvmlError>,
}

impl GpuInventoryProvider for NvmlNvidiaProvider {
    fn collect(&self) -> ProviderOutput {
        match collect_nvml_inventory() {
            Ok(output) => output,
            Err(error) => ProviderOutput::warning(format!(
                "NVIDIA inventory is unavailable on this system: {error}"
            )),
        }
    }
}

pub fn collect_nvidia_performance() -> PerformanceProviderOutput {
    match collect_nvml_performance() {
        Ok(output) => output,
        Err(error) => PerformanceProviderOutput::warning(format!(
            "NVIDIA performance controls are unavailable on this system: {error}"
        )),
    }
}

pub fn set_nvidia_power_limit(request: SetGpuPowerLimitRequest) -> Result<(), String> {
    with_device_mut(&request.gpu_id, |device| {
        device
            .set_power_management_limit(request.limit_watts.saturating_mul(1_000))
            .map_err(|error| map_mutation_error("Changing the power limit", error))
    })
}

pub fn reset_nvidia_power_limit(request: GpuTargetRequest) -> Result<(), String> {
    with_device_mut(&request.gpu_id, |device| {
        let default_limit = device
            .power_management_limit_default()
            .map_err(|error| map_mutation_error("Reading the default power limit", error))?;

        device
            .set_power_management_limit(default_limit)
            .map_err(|error| map_mutation_error("Restoring the default power limit", error))
    })
}

pub fn set_nvidia_fan_speed(request: SetGpuFanSpeedRequest) -> Result<(), String> {
    with_device_mut(&request.gpu_id, |device| {
        device
            .set_fan_speed(request.fan_index, request.speed_percent)
            .map_err(|error| map_mutation_error("Setting the fan speed", error))
    })
}

pub fn set_nvidia_fan_auto(request: SetGpuFanModeAutoRequest) -> Result<(), String> {
    with_device_mut(&request.gpu_id, |device| {
        device
            .set_default_fan_speed(request.fan_index)
            .map_err(|error| map_mutation_error("Restoring automatic fan control", error))
    })
}

pub fn set_nvidia_clock_offsets(request: SetGpuClockOffsetsRequest) -> Result<(), String> {
    if request.graphics_offset_mhz.is_none() && request.memory_offset_mhz.is_none() {
        return Err("No clock offset values were provided.".into());
    }

    with_device_mut(&request.gpu_id, |device| {
        if let Some(graphics_offset) = request.graphics_offset_mhz {
            device
                .set_clock_offset(Clock::Graphics, OFFSET_POWER_STATE, graphics_offset)
                .map_err(|error| map_mutation_error("Setting the graphics clock offset", error))?;
        }

        if let Some(memory_offset) = request.memory_offset_mhz {
            device
                .set_clock_offset(Clock::Memory, OFFSET_POWER_STATE, memory_offset)
                .map_err(|error| map_mutation_error("Setting the memory clock offset", error))?;
        }

        Ok(())
    })
}

pub fn reset_nvidia_clock_offsets(request: GpuTargetRequest) -> Result<(), String> {
    with_device_mut(&request.gpu_id, |device| {
        let graphics_supported = device
            .clock_offset(Clock::Graphics, OFFSET_POWER_STATE)
            .is_ok();
        let memory_supported = device
            .clock_offset(Clock::Memory, OFFSET_POWER_STATE)
            .is_ok();

        if !graphics_supported && !memory_supported {
            return Err("Clock offset controls are not supported on this GPU.".into());
        }

        if graphics_supported {
            device
                .set_clock_offset(Clock::Graphics, OFFSET_POWER_STATE, 0)
                .map_err(|error| {
                    map_mutation_error("Resetting the graphics clock offset", error)
                })?;
        }

        if memory_supported {
            device
                .set_clock_offset(Clock::Memory, OFFSET_POWER_STATE, 0)
                .map_err(|error| map_mutation_error("Resetting the memory clock offset", error))?;
        }

        Ok(())
    })
}

fn collect_nvml_inventory() -> Result<ProviderOutput, NvidiaProviderError> {
    let nvml = shared_nvml().map_err(NvidiaProviderError::NvmlInit)?;
    let device_count = nvml
        .device_count()
        .map_err(NvidiaProviderError::DeviceCount)?;
    let driver_version = nvml.sys_driver_version().ok();
    let cuda_driver_version = nvml
        .sys_cuda_driver_version()
        .ok()
        .map(format_cuda_driver_version);

    let mut output = ProviderOutput::default();

    for index in 0..device_count {
        match nvml.device_by_index(index) {
            Ok(device) => {
                let snapshot = NvidiaSnapshot {
                    id: device
                        .uuid()
                        .ok()
                        .unwrap_or_else(|| format!("nvidia-{index}")),
                    name: device
                        .name()
                        .unwrap_or_else(|_| format!("NVIDIA GPU {index}")),
                    architecture: device.architecture().ok().map(format_architecture),
                    driver_version: driver_version.clone(),
                    firmware_version: device.vbios_version().ok(),
                    bus_id: device.pci_info().ok().map(|info| info.bus_id),
                    memory_total_bytes: device.memory_info().ok().map(|info| info.total),
                    compute_api_version: cuda_driver_version.clone(),
                    compute_capability: device
                        .cuda_compute_capability()
                        .ok()
                        .map(|capability| format!("{}.{}", capability.major, capability.minor)),
                    core_count: device.num_cores().ok(),
                    pcie_gen_max: device.max_pcie_link_gen().ok(),
                    pcie_width_max: device.max_pcie_link_width().ok(),
                    power_management_limit_mw: device.power_management_limit().ok(),
                    enforced_power_limit_mw: device.enforced_power_limit().ok(),
                    default_power_limit_mw: device.power_management_limit_default().ok(),
                    min_power_limit_mw: device
                        .power_management_limit_constraints()
                        .ok()
                        .map(|limits| limits.min_limit),
                    max_power_limit_mw: device
                        .power_management_limit_constraints()
                        .ok()
                        .map(|limits| limits.max_limit),
                    graphics_current_mhz: device.clock_info(Clock::Graphics).ok(),
                    graphics_max_mhz: device.max_clock_info(Clock::Graphics).ok(),
                    sm_max_mhz: device.max_clock_info(Clock::SM).ok(),
                    memory_max_mhz: device.max_clock_info(Clock::Memory).ok(),
                    supported_gpu_locked_clocks_mhz: supported_gpu_locked_clocks(&device),
                    supported_memory_locked_clocks_mhz: device
                        .supported_memory_clocks()
                        .unwrap_or_default(),
                    graphics_offset: query_offset(&device, Clock::Graphics).ok(),
                    memory_offset: query_offset(&device, Clock::Memory).ok(),
                };

                output.gpus.push(map_snapshot(snapshot));
            }
            Err(error) => output
                .warnings
                .push(format!("Skipping NVIDIA GPU at index {index}: {error}")),
        }
    }

    Ok(output)
}

fn collect_nvml_performance() -> Result<PerformanceProviderOutput, NvidiaProviderError> {
    let nvml = shared_nvml().map_err(NvidiaProviderError::NvmlInit)?;
    let device_count = nvml
        .device_count()
        .map_err(NvidiaProviderError::DeviceCount)?;
    let mut output = PerformanceProviderOutput::default();

    for index in 0..device_count {
        match nvml.device_by_index(index) {
            Ok(device) => {
                let mut warnings = Vec::new();
                output
                    .gpus
                    .push(query_performance_device(&device, index, &mut warnings));
                output.warnings.extend(warnings);
            }
            Err(error) => output
                .warnings
                .push(format!("Skipping NVIDIA GPU at index {index}: {error}")),
        }
    }

    Ok(output)
}

fn query_performance_device(
    device: &Device<'_>,
    index: u32,
    warnings: &mut Vec<String>,
) -> GpuPerformanceDevice {
    let stable_id = device.uuid().ok().filter(|id| !id.trim().is_empty());
    let id = stable_id
        .clone()
        .unwrap_or_else(|| format!("nvidia-{index}"));
    let name = device
        .name()
        .unwrap_or_else(|_| format!("NVIDIA GPU {index}"));
    let power = query_power_controls(device, &name, warnings);
    let cooling = query_cooling_controls(device, &name, warnings);
    let clocks = query_clock_offsets(device, &name, warnings);
    let telemetry = query_telemetry(device, &name, &power, &cooling, warnings);

    GpuPerformanceDevice {
        id,
        stable_id,
        vendor: GpuVendor::Nvidia,
        name: name.clone(),
        architecture: device.architecture().ok().map(format_architecture),
        bus_id: device.pci_info().ok().map(|info| info.bus_id),
        power,
        cooling,
        clocks,
        telemetry,
    }
}

fn query_power_controls(
    device: &Device<'_>,
    device_name: &str,
    warnings: &mut Vec<String>,
) -> PowerControlInfo {
    power_control_info(
        PowerQueryResults {
            constraints: device
                .power_management_limit_constraints()
                .map(|limits| (limits.min_limit, limits.max_limit)),
            current: device.power_management_limit(),
            default: device.power_management_limit_default(),
            enforced: device.enforced_power_limit(),
            draw: device.power_usage(),
        },
        device_name,
        warnings,
    )
}

struct PowerQueryResults {
    constraints: Result<(u32, u32), NvmlError>,
    current: Result<u32, NvmlError>,
    default: Result<u32, NvmlError>,
    enforced: Result<u32, NvmlError>,
    draw: Result<u32, NvmlError>,
}

fn power_control_info(
    results: PowerQueryResults,
    device_name: &str,
    warnings: &mut Vec<String>,
) -> PowerControlInfo {
    let support = control_support(&results.constraints);
    let reason = results
        .constraints
        .as_ref()
        .err()
        .map(|error| format!("{device_name}: unable to query power limit controls: {error}"));
    if support == ControlSupport::Unknown {
        warnings.extend(reason.clone());
    }
    let current_value_error = results
        .current
        .as_ref()
        .err()
        .map(|error| format!("{device_name}: unable to read configured power limit: {error}"));
    let limits = results.constraints.ok();
    PowerControlInfo {
        support,
        reason,
        current_value_error,
        current_draw_watts: query_optional_power_value(
            results.draw,
            device_name,
            "current power draw",
            warnings,
        ),
        current_limit_watts: query_optional_power_value(
            results.current,
            device_name,
            "configured power limit",
            warnings,
        ),
        default_limit_watts: query_optional_power_value(
            results.default,
            device_name,
            "default power limit",
            warnings,
        ),
        enforced_limit_watts: query_optional_power_value(
            results.enforced,
            device_name,
            "enforced power limit",
            warnings,
        ),
        min_limit_watts: limits.map(|(min, _)| milliwatts_to_watts(min)),
        max_limit_watts: limits.map(|(_, max)| milliwatts_to_watts(max)),
    }
}

fn query_cooling_controls(
    device: &Device<'_>,
    device_name: &str,
    warnings: &mut Vec<String>,
) -> CoolingInfo {
    let (max_operating_temperature_c, max_operating_temperature_reason) =
        max_operating_temperature(query_thermal_limits(device), device_name, warnings);
    let cooling =
        |is_supported, unsupported_reason: Option<&str>, fans: Vec<FanControlInfo>| CoolingInfo {
            is_supported,
            unsupported_reason: unsupported_reason.map(Into::into),
            fan_count: fans.len() as u32,
            fans,
            max_operating_temperature_c,
            max_operating_temperature_reason: max_operating_temperature_reason.clone(),
            curve_support: None,
            control: None,
        };
    match device.num_fans() {
        Ok(fan_count) if fan_count > 0 => {
            let speed_bounds = match device.min_max_fan_speed() {
                Ok(bounds) => Some(bounds),
                Err(NvmlError::NotSupported) => None,
                Err(error) => {
                    warnings.push(format!(
                        "{device_name}: unable to query adjustable fan limits: {error}"
                    ));
                    None
                }
            };

            cooling(
                true,
                None,
                (0..fan_count)
                    .map(|fan_index| {
                        query_fan_control(device, device_name, fan_index, speed_bounds, warnings)
                    })
                    .collect(),
            )
        }
        Ok(_) => cooling(
            false,
            Some("No programmable fans were detected for this GPU."),
            Vec::new(),
        ),
        Err(NvmlError::NotSupported) => cooling(
            false,
            Some("Fan control is not exposed by this GPU or driver."),
            Vec::new(),
        ),
        Err(error) => {
            warnings.push(format!(
                "{device_name}: unable to query cooling controls: {error}"
            ));
            cooling(
                false,
                Some("Cooling controls are currently unavailable."),
                Vec::new(),
            )
        }
    }
}

fn query_thermal_limits(device: &Device<'_>) -> ThermalLimitReadings {
    let gpu_max_tlimit_offset = device
        .field_values_for(&[FieldId(GPU_MAX_TLIMIT_FIELD)])
        .ok()
        .and_then(|samples| samples.into_iter().next())
        .and_then(|sample| sample.ok())
        .and_then(|sample| sample.value.ok())
        .and_then(|value| match value {
            SampleValue::I64(value) => Some(value),
            SampleValue::U32(value) => Some(i64::from(value)),
            SampleValue::U64(value) => i64::try_from(value).ok(),
            SampleValue::F64(_) => None,
        });
    ThermalLimitReadings {
        gpu_max_threshold: device.temperature_threshold(TemperatureThreshold::GpuMax),
        gpu_max_tlimit_offset,
    }
}

/// The trustworthy maximum operating temperature in °C, or why it is unknown. A T.Limit offset
/// is a margin from an unreported limit, so it never stands in for a temperature.
fn max_operating_temperature(
    readings: ThermalLimitReadings,
    device_name: &str,
    warnings: &mut Vec<String>,
) -> (Option<u32>, Option<String>) {
    match readings.gpu_max_threshold {
        Ok(celsius) if PLAUSIBLE_MAX_OPERATING_TEMPERATURE_C.contains(&celsius) => {
            (Some(celsius), None)
        }
        Ok(celsius) => {
            warnings.push(format!(
                "{device_name}: ignoring an implausible maximum operating temperature of {celsius}°C"
            ));
            (
                None,
                Some(format!(
                    "The GPU reported an implausible maximum operating temperature ({celsius}°C)."
                )),
            )
        }
        Err(_) if readings.gpu_max_tlimit_offset.is_some() => (
            None,
            Some(
                "This GPU reports its thermal limit only as a T.Limit margin, not as a temperature."
                    .into(),
            ),
        ),
        Err(NvmlError::NotSupported) => (
            None,
            Some("This GPU or driver doesn't report a maximum operating temperature.".into()),
        ),
        Err(error) => {
            warnings.push(format!(
                "{device_name}: unable to query the maximum operating temperature: {error}"
            ));
            (
                None,
                Some(format!(
                    "Unable to read the maximum operating temperature: {error}"
                )),
            )
        }
    }
}

fn query_fan_control(
    device: &Device<'_>,
    device_name: &str,
    fan_index: u32,
    speed_bounds: Option<(u32, u32)>,
    warnings: &mut Vec<String>,
) -> FanControlInfo {
    let current_speed_percent = match device.fan_speed(fan_index) {
        Ok(speed) => Some(speed),
        Err(NvmlError::NotSupported) => None,
        Err(error) => {
            warnings.push(format!(
                "{device_name}: unable to query fan {fan_index} speed: {error}"
            ));
            None
        }
    };

    let mode = match device.fan_control_policy(fan_index) {
        Ok(FanControlPolicy::Manual) => Some(FanControlMode::Manual),
        Ok(FanControlPolicy::TemperatureContinousSw) => Some(FanControlMode::Auto),
        Err(NvmlError::NotSupported) => None,
        Err(error) => {
            warnings.push(format!(
                "{device_name}: unable to query fan {fan_index} control mode: {error}"
            ));
            None
        }
    };

    FanControlInfo {
        index: fan_index,
        current_speed_percent,
        min_speed_percent: speed_bounds.map(|(min, _)| min),
        max_speed_percent: speed_bounds.map(|(_, max)| max),
        mode,
    }
}

fn query_clock_offsets(
    device: &Device<'_>,
    device_name: &str,
    warnings: &mut Vec<String>,
) -> ClockOffsetsInfo {
    let graphics = query_clock_offset(device, device_name, Clock::Graphics, "graphics", warnings);
    let memory = query_clock_offset(device, device_name, Clock::Memory, "memory", warnings);
    ClockOffsetsInfo { graphics, memory }
}

fn query_clock_offset(
    device: &Device<'_>,
    device_name: &str,
    clock_type: Clock,
    label: &str,
    warnings: &mut Vec<String>,
) -> ClockOffsetInfo {
    clock_offset_info(
        device.clock_offset(clock_type, OFFSET_POWER_STATE),
        device_name,
        label,
        warnings,
    )
}

fn control_support<T>(result: &Result<T, NvmlError>) -> ControlSupport {
    match result {
        Ok(_) => ControlSupport::Supported,
        Err(NvmlError::NotSupported) => ControlSupport::Unsupported,
        Err(_) => ControlSupport::Unknown,
    }
}

fn clock_offset_info(
    result: Result<nvml_wrapper::struct_wrappers::device::ClockOffset, NvmlError>,
    device_name: &str,
    label: &str,
    warnings: &mut Vec<String>,
) -> ClockOffsetInfo {
    let support = control_support(&result);
    let reason = result.as_ref().err().map(|error| {
        format!("{device_name}: unable to query {label} clock offset controls: {error}")
    });
    if support == ControlSupport::Unknown {
        warnings.extend(reason.clone());
    }
    let offset = result.ok();
    ClockOffsetInfo {
        support,
        reason,
        current_value_error: None,
        current_offset_mhz: offset.as_ref().map(|value| value.clock_offset_mhz),
        min_offset_mhz: offset.as_ref().map(|value| value.min_clock_offset_mhz),
        max_offset_mhz: offset.as_ref().map(|value| value.max_clock_offset_mhz),
    }
}

fn query_telemetry(
    device: &Device<'_>,
    device_name: &str,
    power: &PowerControlInfo,
    cooling: &CoolingInfo,
    warnings: &mut Vec<String>,
) -> GpuTelemetryInfo {
    let results = TelemetryQueryResults {
        core_clock_mhz: device.clock_info(Clock::Graphics),
        memory_clock_mhz: device.clock_info(Clock::Memory),
        temperature_c: device.temperature(TemperatureSensor::Gpu),
        gpu_utilization_percent: device.utilization_rates().map(|value| value.gpu),
        memory_info: device.memory_info(),
        performance_state: device.performance_state(),
    };

    build_telemetry_info(device_name, power, cooling, results, warnings)
}

fn build_telemetry_info(
    device_name: &str,
    power: &PowerControlInfo,
    cooling: &CoolingInfo,
    results: TelemetryQueryResults,
    warnings: &mut Vec<String>,
) -> GpuTelemetryInfo {
    let memory_info =
        query_optional_telemetry_metric(results.memory_info, device_name, "memory usage", warnings);
    let performance_state = query_optional_telemetry_metric(
        results.performance_state,
        device_name,
        "performance state",
        warnings,
    )
    .and_then(format_performance_state);

    GpuTelemetryInfo {
        power_draw_watts: power.current_draw_watts,
        core_clock_mhz: query_optional_telemetry_metric(
            results.core_clock_mhz,
            device_name,
            "graphics clock",
            warnings,
        ),
        memory_clock_mhz: query_optional_telemetry_metric(
            results.memory_clock_mhz,
            device_name,
            "memory clock",
            warnings,
        ),
        fan_speed_percent: average_fan_speed_percent(cooling),
        voltage_millivolts: None,
        temperature_c: query_optional_telemetry_metric(
            results.temperature_c,
            device_name,
            "temperature",
            warnings,
        ),
        gpu_utilization_percent: query_optional_telemetry_metric(
            results.gpu_utilization_percent,
            device_name,
            "GPU utilization",
            warnings,
        ),
        memory_used_mib: memory_info.as_ref().map(|value| bytes_to_mib(value.used)),
        memory_total_mib: memory_info.as_ref().map(|value| bytes_to_mib(value.total)),
        performance_state,
    }
}

fn query_optional_telemetry_metric<T>(
    result: Result<T, NvmlError>,
    device_name: &str,
    metric_name: &str,
    warnings: &mut Vec<String>,
) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(NvmlError::NotSupported) => None,
        Err(error) => {
            warnings.push(format!(
                "{device_name}: unable to query {metric_name}: {error}"
            ));
            None
        }
    }
}

fn average_fan_speed_percent(cooling: &CoolingInfo) -> Option<u32> {
    let mut total = 0_u64;
    let mut count = 0_u64;

    for fan in &cooling.fans {
        if let Some(speed) = fan.current_speed_percent {
            total += u64::from(speed);
            count += 1;
        }
    }

    total.checked_div(count).map(|average| average as u32)
}

fn format_performance_state(state: PerformanceState) -> Option<String> {
    match state {
        PerformanceState::Zero => Some("P0".into()),
        PerformanceState::One => Some("P1".into()),
        PerformanceState::Two => Some("P2".into()),
        PerformanceState::Three => Some("P3".into()),
        PerformanceState::Four => Some("P4".into()),
        PerformanceState::Five => Some("P5".into()),
        PerformanceState::Six => Some("P6".into()),
        PerformanceState::Seven => Some("P7".into()),
        PerformanceState::Eight => Some("P8".into()),
        PerformanceState::Nine => Some("P9".into()),
        PerformanceState::Ten => Some("P10".into()),
        PerformanceState::Eleven => Some("P11".into()),
        PerformanceState::Twelve => Some("P12".into()),
        PerformanceState::Thirteen => Some("P13".into()),
        PerformanceState::Fourteen => Some("P14".into()),
        PerformanceState::Fifteen => Some("P15".into()),
        PerformanceState::Unknown => None,
    }
}

fn map_snapshot(snapshot: NvidiaSnapshot) -> GpuDevice {
    let power_limit_supported =
        snapshot.min_power_limit_mw.is_some() && snapshot.max_power_limit_mw.is_some();
    let gpu_lock_supported = !snapshot.supported_gpu_locked_clocks_mhz.is_empty();
    let memory_lock_supported = !snapshot.supported_memory_locked_clocks_mhz.is_empty();
    let graphics_offset_supported = snapshot.graphics_offset.is_some();
    let memory_offset_supported = snapshot.memory_offset.is_some();

    GpuDevice {
        id: snapshot.id.clone(),
        vendor: GpuVendor::Nvidia,
        name: snapshot.name,
        architecture: snapshot.architecture,
        driver_version: snapshot.driver_version,
        firmware_version: snapshot.firmware_version,
        bus_id: snapshot.bus_id,
        memory: MemoryInfo {
            total_mib: snapshot.memory_total_bytes.map(bytes_to_mib),
        },
        compute: ComputeInfo {
            api: Some(ComputeApi::Cuda),
            api_version: snapshot.compute_api_version,
            capability: snapshot.compute_capability,
            core_count: snapshot.core_count,
        },
        pcie: PcieInfo {
            gen_max: snapshot.pcie_gen_max,
            width_max: snapshot.pcie_width_max,
        },
        power: PowerInfo {
            enforced_limit_watts: snapshot.enforced_power_limit_mw.map(milliwatts_to_watts),
            default_limit_watts: snapshot.default_power_limit_mw.map(milliwatts_to_watts),
        },
        clocks: ClockInfo {
            graphics_current_mhz: snapshot.graphics_current_mhz,
            graphics_max_mhz: snapshot.graphics_max_mhz,
            sm_max_mhz: snapshot.sm_max_mhz,
            memory_max_mhz: snapshot.memory_max_mhz,
        },
        control: GpuControlInfo {
            supported_features: GpuSupportedFeatures {
                power_limit: power_limit_supported,
                gpu_clock_lock: gpu_lock_supported,
                memory_clock_lock: memory_lock_supported,
                graphics_offset: graphics_offset_supported,
                memory_offset: memory_offset_supported,
                reset: power_limit_supported
                    || gpu_lock_supported
                    || memory_lock_supported
                    || graphics_offset_supported
                    || memory_offset_supported,
            },
            power_limit: PowerLimitControl {
                current_watts: snapshot.power_management_limit_mw.map(milliwatts_to_watts),
                default_watts: snapshot.default_power_limit_mw.map(milliwatts_to_watts),
                min_watts: snapshot.min_power_limit_mw.map(milliwatts_to_watts),
                max_watts: snapshot.max_power_limit_mw.map(milliwatts_to_watts),
            },
            gpu_clock_lock: ClockLockControl {
                current_min_mhz: None,
                current_max_mhz: None,
                min_allowed_mhz: snapshot
                    .supported_gpu_locked_clocks_mhz
                    .iter()
                    .min()
                    .copied(),
                max_allowed_mhz: snapshot
                    .supported_gpu_locked_clocks_mhz
                    .iter()
                    .max()
                    .copied(),
                supported: gpu_lock_supported,
            },
            memory_clock_lock: ClockLockControl {
                current_min_mhz: None,
                current_max_mhz: None,
                min_allowed_mhz: snapshot
                    .supported_memory_locked_clocks_mhz
                    .iter()
                    .min()
                    .copied(),
                max_allowed_mhz: snapshot
                    .supported_memory_locked_clocks_mhz
                    .iter()
                    .max()
                    .copied(),
                supported: memory_lock_supported,
            },
            offsets: OffsetControl {
                graphics_mhz: snapshot
                    .graphics_offset
                    .as_ref()
                    .map(|value| value.current_mhz),
                memory_mhz: snapshot
                    .memory_offset
                    .as_ref()
                    .map(|value| value.current_mhz),
                min_graphics_mhz: snapshot.graphics_offset.as_ref().map(|value| value.min_mhz),
                max_graphics_mhz: snapshot.graphics_offset.as_ref().map(|value| value.max_mhz),
                min_memory_mhz: snapshot.memory_offset.as_ref().map(|value| value.min_mhz),
                max_memory_mhz: snapshot.memory_offset.as_ref().map(|value| value.max_mhz),
            },
            warnings: Vec::new(),
        },
    }
}

fn supported_gpu_locked_clocks(device: &Device<'_>) -> Vec<u32> {
    let Ok(memory_clocks) = device.supported_memory_clocks() else {
        return Vec::new();
    };

    let mut supported = BTreeSet::new();

    for memory_clock in memory_clocks {
        if let Ok(graphics_clocks) = device.supported_graphics_clocks(memory_clock) {
            supported.extend(graphics_clocks);
        }
    }

    supported.into_iter().collect()
}

fn query_offset(device: &Device<'_>, clock_type: Clock) -> Result<ClockOffsetSnapshot, NvmlError> {
    let offset = device.clock_offset(clock_type, OFFSET_POWER_STATE)?;

    Ok(ClockOffsetSnapshot {
        current_mhz: offset.clock_offset_mhz,
        min_mhz: offset.min_clock_offset_mhz,
        max_mhz: offset.max_clock_offset_mhz,
    })
}

fn with_device_mut<T>(
    gpu_id: &str,
    operation: impl FnOnce(&mut Device<'_>) -> Result<T, String>,
) -> Result<T, String> {
    let nvml = shared_nvml().map_err(|error| format!("Unable to initialize NVML: {error}"))?;
    let mut device = find_device_by_id(nvml, gpu_id)?;

    operation(&mut device)
}

fn shared_nvml() -> Result<&'static Nvml, NvmlError> {
    NVML.get_or_try_init(Nvml::init)
}

fn find_device_by_id<'nvml>(nvml: &'nvml Nvml, gpu_id: &str) -> Result<Device<'nvml>, String> {
    if let Ok(device) = nvml.device_by_uuid(gpu_id) {
        return Ok(device);
    }

    if let Ok(device) = nvml.device_by_pci_bus_id(gpu_id) {
        return Ok(device);
    }

    if let Some(index) = parse_fallback_index(gpu_id) {
        return nvml
            .device_by_index(index)
            .map_err(|error| format!("Unable to find GPU `{gpu_id}`: {error}"));
    }

    Err(format!(
        "Unable to find GPU `{gpu_id}`. Refresh the page and try again."
    ))
}

fn parse_fallback_index(gpu_id: &str) -> Option<u32> {
    gpu_id
        .strip_prefix("nvidia-")
        .and_then(|index| index.parse::<u32>().ok())
}

fn query_optional_power_value(
    result: Result<u32, NvmlError>,
    device_name: &str,
    field_name: &str,
    warnings: &mut Vec<String>,
) -> Option<u32> {
    match result {
        Ok(value) => Some(milliwatts_to_watts(value)),
        Err(NvmlError::NotSupported) => None,
        Err(error) => {
            warnings.push(format!(
                "{device_name}: unable to query {field_name}: {error}"
            ));
            None
        }
    }
}

fn map_mutation_error(action: &str, error: NvmlError) -> String {
    match error {
        NvmlError::NoPermission => format!("{action} requires elevated privileges."),
        NvmlError::NotSupported => {
            format!("{action} is not supported by this GPU or driver.")
        }
        NvmlError::InvalidArg => {
            format!("{action} failed because the requested value is outside the supported range.")
        }
        NvmlError::GpuLost => {
            "The GPU became unavailable while applying the requested change.".into()
        }
        other => format!("{action} failed: {other}"),
    }
}

fn format_architecture(architecture: DeviceArchitecture) -> String {
    architecture.to_string()
}

fn format_cuda_driver_version(version: i32) -> String {
    format!(
        "{}.{}",
        cuda_driver_version_major(version),
        cuda_driver_version_minor(version)
    )
}

fn bytes_to_mib(bytes: u64) -> u64 {
    bytes / (1024 * 1024)
}

fn milliwatts_to_watts(milliwatts: u32) -> u32 {
    milliwatts / 1000
}

#[cfg(test)]
mod tests {
    use super::{
        ClockOffsetSnapshot, NvidiaSnapshot, TelemetryQueryResults, ThermalLimitReadings,
        build_telemetry_info, bytes_to_mib, format_cuda_driver_version, map_mutation_error,
        map_snapshot, max_operating_temperature, milliwatts_to_watts, parse_fallback_index,
    };
    use crate::{
        ComputeApi, ControlSupport, CoolingInfo, FanControlInfo, FanControlMode, GpuVendor,
        PowerControlInfo,
    };
    use nvml_wrapper::enum_wrappers::device::PerformanceState;
    use nvml_wrapper::error::NvmlError;
    use nvml_wrapper::struct_wrappers::device::MemoryInfo as NvmlMemoryInfo;

    #[test]
    fn maps_snapshot_into_vendor_agnostic_device() {
        let device = map_snapshot(NvidiaSnapshot {
            id: "GPU-123".into(),
            name: "NVIDIA GeForce RTX 5090".into(),
            architecture: Some("Blackwell".into()),
            driver_version: Some("595.58.03".into()),
            firmware_version: Some("98.02.2E.00.03".into()),
            bus_id: Some("0000:01:00.0".into()),
            memory_total_bytes: Some(34_359_738_368),
            compute_api_version: Some("13.2".into()),
            compute_capability: Some("12.0".into()),
            core_count: Some(21_760),
            pcie_gen_max: Some(5),
            pcie_width_max: Some(16),
            power_management_limit_mw: Some(475_000),
            enforced_power_limit_mw: Some(475_000),
            default_power_limit_mw: Some(575_000),
            min_power_limit_mw: Some(300_000),
            max_power_limit_mw: Some(575_000),
            graphics_current_mhz: Some(1_365),
            graphics_max_mhz: Some(3_300),
            sm_max_mhz: Some(3_300),
            memory_max_mhz: Some(14_001),
            supported_gpu_locked_clocks_mhz: vec![1800, 2000, 2400],
            supported_memory_locked_clocks_mhz: vec![1000, 1100],
            graphics_offset: Some(ClockOffsetSnapshot {
                current_mhz: 100,
                min_mhz: -200,
                max_mhz: 200,
            }),
            memory_offset: Some(ClockOffsetSnapshot {
                current_mhz: 500,
                min_mhz: -1000,
                max_mhz: 1000,
            }),
        });

        assert_eq!(device.vendor, GpuVendor::Nvidia);
        assert_eq!(device.memory.total_mib, Some(32_768));
        assert_eq!(device.power.enforced_limit_watts, Some(475));
        assert_eq!(device.compute.api, Some(ComputeApi::Cuda));
        assert_eq!(device.compute.capability.as_deref(), Some("12.0"));
    }

    #[test]
    fn conversion_helpers_use_expected_units() {
        assert_eq!(bytes_to_mib(1_073_741_824), 1_024);
        assert_eq!(milliwatts_to_watts(475_000), 475);
        assert_eq!(format_cuda_driver_version(13_020), "13.2");
    }

    #[test]
    fn mutation_error_messages_are_user_friendly() {
        assert_eq!(
            map_mutation_error("Setting the fan speed", NvmlError::NoPermission),
            "Setting the fan speed requires elevated privileges."
        );
        assert_eq!(
            map_mutation_error("Setting the power limit", NvmlError::InvalidArg),
            "Setting the power limit failed because the requested value is outside the supported range."
        );
    }

    #[test]
    fn telemetry_helper_nulls_only_failed_metric() {
        let power = PowerControlInfo {
            support: ControlSupport::Supported,
            reason: None,
            current_value_error: None,
            current_draw_watts: Some(312),
            current_limit_watts: Some(450),
            default_limit_watts: Some(450),
            enforced_limit_watts: Some(450),
            min_limit_watts: Some(300),
            max_limit_watts: Some(600),
        };
        let cooling = CoolingInfo {
            is_supported: true,
            unsupported_reason: None,
            fan_count: 2,
            fans: vec![
                FanControlInfo {
                    index: 0,
                    current_speed_percent: Some(35),
                    min_speed_percent: Some(30),
                    max_speed_percent: Some(100),
                    mode: Some(FanControlMode::Auto),
                },
                FanControlInfo {
                    index: 1,
                    current_speed_percent: Some(45),
                    min_speed_percent: Some(30),
                    max_speed_percent: Some(100),
                    mode: Some(FanControlMode::Auto),
                },
            ],
            max_operating_temperature_c: Some(90),
            max_operating_temperature_reason: None,
            curve_support: None,
            control: None,
        };
        let mut warnings = Vec::new();

        let telemetry = build_telemetry_info(
            "RTX Test",
            &power,
            &cooling,
            TelemetryQueryResults {
                core_clock_mhz: Err(NvmlError::Unknown),
                memory_clock_mhz: Ok(14_001),
                temperature_c: Ok(71),
                gpu_utilization_percent: Ok(96),
                memory_info: Ok(NvmlMemoryInfo {
                    free: 1_073_741_824,
                    reserved: 0,
                    total: 34_359_738_368,
                    used: 12_884_901_888,
                    version: 2,
                }),
                performance_state: Ok(PerformanceState::Zero),
            },
            &mut warnings,
        );

        assert_eq!(telemetry.power_draw_watts, Some(312));
        assert_eq!(telemetry.core_clock_mhz, None);
        assert_eq!(telemetry.memory_clock_mhz, Some(14_001));
        assert_eq!(telemetry.fan_speed_percent, Some(40));
        assert_eq!(telemetry.temperature_c, Some(71));
        assert_eq!(telemetry.gpu_utilization_percent, Some(96));
        assert_eq!(telemetry.memory_used_mib, Some(12_288));
        assert_eq!(telemetry.memory_total_mib, Some(32_768));
        assert_eq!(telemetry.performance_state.as_deref(), Some("P0"));
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("graphics clock"));
    }

    #[test]
    fn reads_an_absolute_maximum_operating_temperature() {
        let mut warnings = Vec::new();
        let limit = max_operating_temperature(
            ThermalLimitReadings {
                gpu_max_threshold: Ok(90),
                gpu_max_tlimit_offset: None,
            },
            "RTX Test",
            &mut warnings,
        );

        assert_eq!(limit, (Some(90), None));
        assert!(warnings.is_empty());
    }

    #[test]
    fn prefers_the_absolute_temperature_when_a_t_limit_margin_is_also_reported() {
        let mut warnings = Vec::new();
        let limit = max_operating_temperature(
            ThermalLimitReadings {
                gpu_max_threshold: Ok(88),
                gpu_max_tlimit_offset: Some(0),
            },
            "RTX Test",
            &mut warnings,
        );

        assert_eq!(limit.0, Some(88));
    }

    #[test]
    fn never_takes_a_t_limit_margin_for_a_temperature() {
        let mut warnings = Vec::new();
        let (celsius, reason) = max_operating_temperature(
            ThermalLimitReadings {
                gpu_max_threshold: Err(NvmlError::NotSupported),
                gpu_max_tlimit_offset: Some(-2),
            },
            "RTX Test",
            &mut warnings,
        );

        assert_eq!(celsius, None);
        assert!(reason.unwrap().contains("T.Limit"));
    }

    #[test]
    fn reports_a_missing_maximum_operating_temperature_as_unknown() {
        let mut warnings = Vec::new();
        let (celsius, reason) = max_operating_temperature(
            ThermalLimitReadings {
                gpu_max_threshold: Err(NvmlError::NotSupported),
                gpu_max_tlimit_offset: None,
            },
            "RTX Test",
            &mut warnings,
        );
        assert_eq!(celsius, None);
        assert!(
            reason
                .unwrap()
                .contains("doesn't report a maximum operating temperature")
        );
        assert!(warnings.is_empty());

        let (celsius, reason) = max_operating_temperature(
            ThermalLimitReadings {
                gpu_max_threshold: Err(NvmlError::Unknown),
                gpu_max_tlimit_offset: None,
            },
            "RTX Test",
            &mut warnings,
        );
        assert_eq!(celsius, None);
        assert!(reason.unwrap().contains("Unable to read"));
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn rejects_an_implausible_maximum_operating_temperature() {
        // A zero is what a T.Limit offset would look like if read as a temperature.
        for implausible in [0, 2, 40, 200, 255] {
            let mut warnings = Vec::new();
            let (celsius, reason) = max_operating_temperature(
                ThermalLimitReadings {
                    gpu_max_threshold: Ok(implausible),
                    gpu_max_tlimit_offset: None,
                },
                "RTX Test",
                &mut warnings,
            );

            assert_eq!(celsius, None, "{implausible}°C");
            assert!(reason.unwrap().contains("implausible"));
            assert_eq!(warnings.len(), 1);
        }
    }

    #[test]
    fn parses_fallback_gpu_ids() {
        assert_eq!(parse_fallback_index("nvidia-3"), Some(3));
        assert_eq!(parse_fallback_index("GPU-123"), None);
    }
}
