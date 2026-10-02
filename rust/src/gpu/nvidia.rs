use std::collections::BTreeSet;

use nvml_wrapper::enum_wrappers::device::{Clock, PerformanceState, TemperatureSensor};
use nvml_wrapper::enums::device::{DeviceArchitecture, FanControlPolicy, GpuLockedClocksSetting};
use nvml_wrapper::error::NvmlError;
use nvml_wrapper::struct_wrappers::device::MemoryInfo as NvmlMemoryInfo;
use nvml_wrapper::{cuda_driver_version_major, cuda_driver_version_minor, Device, Nvml};
use once_cell::sync::OnceCell;
use thiserror::Error;

use super::{
    ApplyGpuConfigRequest, ClockInfo, ClockLockControl, ClockOffsetInfo, ClockOffsetsInfo,
    ClockRangeInput, ComputeApi, ComputeInfo, ControlSupport, CoolingInfo, FanControlInfo,
    FanControlMode, GpuCommandError, GpuCommandErrorCode, GpuControlInfo, GpuControlProvider,
    GpuDevice, GpuInventoryProvider, GpuPerformanceDevice, GpuSupportedFeatures, GpuTargetRequest,
    GpuTelemetryInfo, GpuVendor, InventoryContext, MemoryInfo, OffsetControl, PcieInfo,
    PerformanceProviderOutput, PersistedGpuConfig, PowerControlInfo, PowerInfo, PowerLimitControl,
    ProviderOutput, SetGpuClockOffsetsRequest, SetGpuFanModeAutoRequest, SetGpuFanSpeedRequest,
    SetGpuPowerLimitRequest,
};

const OFFSET_POWER_STATE: PerformanceState = PerformanceState::Zero;
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct NvidiaCapabilities {
    power_limit_min_mw: Option<u32>,
    power_limit_max_mw: Option<u32>,
    power_limit_default_mw: Option<u32>,
    supported_gpu_locked_clocks_mhz: Vec<u32>,
    supported_memory_locked_clocks_mhz: Vec<u32>,
    graphics_offset: Option<ClockOffsetSnapshot>,
    memory_offset: Option<ClockOffsetSnapshot>,
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
    fn collect(&self, context: InventoryContext<'_>) -> ProviderOutput {
        match collect_nvml_inventory(context) {
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

impl GpuControlProvider for NvmlNvidiaProvider {
    fn apply_config(&self, request: &ApplyGpuConfigRequest) -> Result<(), GpuCommandError> {
        let gpu_id = request.gpu_id.as_deref().ok_or_else(|| {
            GpuCommandError::new(
                GpuCommandErrorCode::InvalidArgument,
                "gpuId is required when applying a GPU configuration",
            )
        })?;

        let nvml = shared_nvml().map_err(map_nvml_error)?;
        let mut device = find_device_by_uuid(&nvml, gpu_id)?;
        let capabilities = query_capabilities(&device);

        validate_apply_request(request, &capabilities)?;
        enable_persistence_if_supported(&mut device)?;

        if let Some(power_limit_watts) = request.power_limit_watts {
            device
                .set_power_management_limit(power_limit_watts.saturating_mul(1_000))
                .map_err(map_nvml_error)?;
        }

        if let Some(clock_lock) = request.gpu_clock_lock {
            device
                .set_gpu_locked_clocks(GpuLockedClocksSetting::Numeric {
                    min_clock_mhz: clock_lock.min_mhz,
                    max_clock_mhz: clock_lock.max_mhz,
                })
                .map_err(map_nvml_error)?;
        }

        if let Some(clock_lock) = request.memory_clock_lock {
            device
                .set_mem_locked_clocks(clock_lock.min_mhz, clock_lock.max_mhz)
                .map_err(map_nvml_error)?;
        }

        if let Some(offset) = request.graphics_offset_mhz {
            device
                .set_clock_offset(Clock::Graphics, OFFSET_POWER_STATE, offset)
                .map_err(map_nvml_error)?;
        }

        if let Some(offset) = request.memory_offset_mhz {
            device
                .set_clock_offset(Clock::Memory, OFFSET_POWER_STATE, offset)
                .map_err(map_nvml_error)?;
        }

        Ok(())
    }

    fn reset_config(&self, gpu_id: &str) -> Result<(), GpuCommandError> {
        let nvml = shared_nvml().map_err(map_nvml_error)?;
        let mut device = find_device_by_uuid(&nvml, gpu_id)?;
        let capabilities = query_capabilities(&device);

        enable_persistence_if_supported(&mut device)?;

        if let Some(default_limit) = capabilities.power_limit_default_mw {
            device
                .set_power_management_limit(default_limit)
                .map_err(map_nvml_error)?;
        }

        reset_if_supported(device.reset_gpu_locked_clocks())?;
        reset_if_supported(device.reset_mem_locked_clocks())?;

        if capabilities.graphics_offset.is_some() {
            device
                .set_clock_offset(Clock::Graphics, OFFSET_POWER_STATE, 0)
                .map_err(map_nvml_error)?;
        }

        if capabilities.memory_offset.is_some() {
            device
                .set_clock_offset(Clock::Memory, OFFSET_POWER_STATE, 0)
                .map_err(map_nvml_error)?;
        }

        Ok(())
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

fn collect_nvml_inventory(
    context: InventoryContext<'_>,
) -> Result<ProviderOutput, NvidiaProviderError> {
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

                let persisted = context
                    .persisted_state
                    .and_then(|state| state.record_for(&snapshot.id));

                output.gpus.push(map_snapshot(snapshot, context, persisted));
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

            CoolingInfo {
                is_supported: true,
                unsupported_reason: None,
                fan_count,
                fans: (0..fan_count)
                    .map(|fan_index| {
                        query_fan_control(device, device_name, fan_index, speed_bounds, warnings)
                    })
                    .collect(),
            }
        }
        Ok(_) => CoolingInfo {
            is_supported: false,
            unsupported_reason: Some("No programmable fans were detected for this GPU.".into()),
            fan_count: 0,
            fans: Vec::new(),
        },
        Err(NvmlError::NotSupported) => CoolingInfo {
            is_supported: false,
            unsupported_reason: Some("Fan control is not exposed by this GPU or driver.".into()),
            fan_count: 0,
            fans: Vec::new(),
        },
        Err(error) => {
            warnings.push(format!(
                "{device_name}: unable to query cooling controls: {error}"
            ));

            CoolingInfo {
                is_supported: false,
                unsupported_reason: Some("Cooling controls are currently unavailable.".into()),
                fan_count: 0,
                fans: Vec::new(),
            }
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
        memory_used_mi_b: memory_info.as_ref().map(|value| bytes_to_mib(value.used)),
        memory_total_mi_b: memory_info.as_ref().map(|value| bytes_to_mib(value.total)),
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

    if count == 0 {
        None
    } else {
        Some((total / count) as u32)
    }
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

fn map_snapshot(
    snapshot: NvidiaSnapshot,
    context: InventoryContext<'_>,
    persisted: Option<&PersistedGpuConfig>,
) -> GpuDevice {
    let desired = persisted.map(|record| &record.desired);
    let last_apply = persisted.and_then(|record| record.last_apply.as_ref());

    let power_limit_supported =
        snapshot.min_power_limit_mw.is_some() && snapshot.max_power_limit_mw.is_some();
    let gpu_lock_supported = !snapshot.supported_gpu_locked_clocks_mhz.is_empty();
    let memory_lock_supported = !snapshot.supported_memory_locked_clocks_mhz.is_empty();
    let graphics_offset_supported = snapshot.graphics_offset.is_some();
    let memory_offset_supported = snapshot.memory_offset.is_some();

    let mut warnings = Vec::new();

    if let Some(message) = context.daemon_warning {
        warnings.push(message.to_string());
    }

    if let Some(last_apply) = last_apply {
        if !last_apply.success {
            if let Some(message) = last_apply.message.as_ref() {
                warnings.push(message.clone());
            }
        }
    }

    GpuDevice {
        id: snapshot.id.clone(),
        vendor: GpuVendor::Nvidia,
        name: snapshot.name,
        architecture: snapshot.architecture,
        driver_version: snapshot.driver_version,
        firmware_version: snapshot.firmware_version,
        bus_id: snapshot.bus_id,
        memory: MemoryInfo {
            total_mi_b: snapshot.memory_total_bytes.map(bytes_to_mib),
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
            managed_by_daemon: context.managed_by_daemon,
            daemon_reachable: context.daemon_reachable,
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
                desired_watts: desired.and_then(|config| config.power_limit_watts),
            },
            gpu_clock_lock: ClockLockControl {
                current_min_mhz: desired
                    .and_then(|config| config.gpu_clock_lock)
                    .map(|value| value.min_mhz),
                current_max_mhz: desired
                    .and_then(|config| config.gpu_clock_lock)
                    .map(|value| value.max_mhz),
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
                current_min_mhz: desired
                    .and_then(|config| config.memory_clock_lock)
                    .map(|value| value.min_mhz),
                current_max_mhz: desired
                    .and_then(|config| config.memory_clock_lock)
                    .map(|value| value.max_mhz),
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
            warnings,
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

fn query_capabilities(device: &Device<'_>) -> NvidiaCapabilities {
    let power_limits = device.power_management_limit_constraints().ok();

    NvidiaCapabilities {
        power_limit_min_mw: power_limits.as_ref().map(|limits| limits.min_limit),
        power_limit_max_mw: power_limits.as_ref().map(|limits| limits.max_limit),
        power_limit_default_mw: device.power_management_limit_default().ok(),
        supported_gpu_locked_clocks_mhz: supported_gpu_locked_clocks(device),
        supported_memory_locked_clocks_mhz: device.supported_memory_clocks().unwrap_or_default(),
        graphics_offset: query_offset(device, Clock::Graphics).ok(),
        memory_offset: query_offset(device, Clock::Memory).ok(),
    }
}

fn validate_apply_request(
    request: &ApplyGpuConfigRequest,
    capabilities: &NvidiaCapabilities,
) -> Result<(), GpuCommandError> {
    if let Some(power_limit_watts) = request.power_limit_watts {
        let (min_limit, max_limit) = match (
            capabilities.power_limit_min_mw,
            capabilities.power_limit_max_mw,
        ) {
            (Some(min_limit), Some(max_limit)) => (min_limit, max_limit),
            _ => {
                return Err(GpuCommandError::new(
                    GpuCommandErrorCode::NotSupported,
                    "power limit changes are not supported on this GPU",
                ));
            }
        };

        let power_limit_mw = power_limit_watts.saturating_mul(1_000);
        if power_limit_mw < min_limit || power_limit_mw > max_limit {
            return Err(GpuCommandError::new(
                GpuCommandErrorCode::InvalidArgument,
                format!(
                    "power limit must be between {} W and {} W",
                    min_limit / 1000,
                    max_limit / 1000
                ),
            ));
        }
    }

    if let Some(clock_range) = request.gpu_clock_lock {
        validate_discrete_clock_range(
            "GPU clock lock",
            clock_range,
            &capabilities.supported_gpu_locked_clocks_mhz,
        )?;
    }

    if let Some(clock_range) = request.memory_clock_lock {
        validate_discrete_clock_range(
            "memory clock lock",
            clock_range,
            &capabilities.supported_memory_locked_clocks_mhz,
        )?;
    }

    if let Some(offset) = request.graphics_offset_mhz {
        let Some(range) = capabilities.graphics_offset.as_ref() else {
            return Err(GpuCommandError::new(
                GpuCommandErrorCode::NotSupported,
                "graphics clock offsets are not supported on this GPU",
            ));
        };

        validate_offset_range("graphics offset", offset, range)?;
    }

    if let Some(offset) = request.memory_offset_mhz {
        let Some(range) = capabilities.memory_offset.as_ref() else {
            return Err(GpuCommandError::new(
                GpuCommandErrorCode::NotSupported,
                "memory clock offsets are not supported on this GPU",
            ));
        };

        validate_offset_range("memory offset", offset, range)?;
    }

    Ok(())
}

fn validate_discrete_clock_range(
    label: &str,
    requested: ClockRangeInput,
    supported_values: &[u32],
) -> Result<(), GpuCommandError> {
    if supported_values.is_empty() {
        return Err(GpuCommandError::new(
            GpuCommandErrorCode::NotSupported,
            format!("{label} is not supported on this GPU"),
        ));
    }

    if !supported_values.contains(&requested.min_mhz)
        || !supported_values.contains(&requested.max_mhz)
    {
        let min_supported = supported_values.iter().min().copied().unwrap_or_default();
        let max_supported = supported_values.iter().max().copied().unwrap_or_default();

        return Err(GpuCommandError::new(
            GpuCommandErrorCode::InvalidArgument,
            format!(
                "{label} values must match supported clocks between {min_supported} MHz and {max_supported} MHz"
            ),
        ));
    }

    Ok(())
}

fn validate_offset_range(
    label: &str,
    value: i32,
    range: &ClockOffsetSnapshot,
) -> Result<(), GpuCommandError> {
    if value < range.min_mhz || value > range.max_mhz {
        return Err(GpuCommandError::new(
            GpuCommandErrorCode::InvalidArgument,
            format!(
                "{label} must be between {} MHz and {} MHz",
                range.min_mhz, range.max_mhz
            ),
        ));
    }

    Ok(())
}

fn enable_persistence_if_supported(device: &mut Device<'_>) -> Result<(), GpuCommandError> {
    match device.set_persistent(true) {
        Ok(()) | Err(NvmlError::NotSupported) => Ok(()),
        Err(error) => Err(map_nvml_error(error)),
    }
}

fn reset_if_supported(result: Result<(), NvmlError>) -> Result<(), GpuCommandError> {
    match result {
        Ok(()) | Err(NvmlError::NotSupported) => Ok(()),
        Err(error) => Err(map_nvml_error(error)),
    }
}

fn with_device_mut<T>(
    gpu_id: &str,
    operation: impl FnOnce(&mut Device<'_>) -> Result<T, String>,
) -> Result<T, String> {
    let nvml = shared_nvml().map_err(|error| format!("Unable to initialize NVML: {error}"))?;
    let mut device = find_device_by_id(&nvml, gpu_id)?;

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

fn find_device_by_uuid<'a>(nvml: &'a Nvml, gpu_id: &str) -> Result<Device<'a>, GpuCommandError> {
    let device_count = nvml.device_count().map_err(map_nvml_error)?;

    for index in 0..device_count {
        let device = nvml.device_by_index(index).map_err(map_nvml_error)?;
        if device.uuid().ok().as_deref() == Some(gpu_id) {
            return Ok(device);
        }
    }

    Err(GpuCommandError::new(
        GpuCommandErrorCode::NotFound,
        format!("GPU '{gpu_id}' was not found"),
    ))
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

fn map_nvml_error(error: NvmlError) -> GpuCommandError {
    match error {
        NvmlError::NoPermission => GpuCommandError::new(
            GpuCommandErrorCode::NoPermission,
            "insufficient privileges to change GPU configuration",
        ),
        NvmlError::InvalidArg => GpuCommandError::new(
            GpuCommandErrorCode::InvalidArgument,
            "the requested GPU configuration is invalid",
        ),
        NvmlError::NotSupported => GpuCommandError::new(
            GpuCommandErrorCode::NotSupported,
            "the requested GPU configuration is not supported",
        ),
        NvmlError::GpuLost => GpuCommandError::new(
            GpuCommandErrorCode::GpuLost,
            "the GPU is no longer accessible",
        ),
        NvmlError::LibraryNotFound
        | NvmlError::DriverNotLoaded
        | NvmlError::LibRmVersionMismatch
        | NvmlError::LibloadingError(_)
        | NvmlError::FailedToLoadSymbol(_)
        | NvmlError::FunctionNotFound => {
            GpuCommandError::new(GpuCommandErrorCode::NvmlUnavailable, error.to_string())
        }
        _ => GpuCommandError::new(GpuCommandErrorCode::Internal, error.to_string()),
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
        build_telemetry_info, bytes_to_mib, format_cuda_driver_version, map_mutation_error,
        map_snapshot, milliwatts_to_watts, parse_fallback_index, validate_discrete_clock_range,
        validate_offset_range, ClockOffsetSnapshot, NvidiaSnapshot, TelemetryQueryResults,
    };
    use crate::gpu::{
        ClockRangeInput, ComputeApi, ControlSupport, CoolingInfo, DesiredGpuConfig, FanControlInfo,
        FanControlMode, GpuApplyRecord, GpuCommandErrorCode, GpuVendor, InventoryContext,
        PersistedGpuConfig, PersistedGpuState, PowerControlInfo,
    };
    use nvml_wrapper::enum_wrappers::device::PerformanceState;
    use nvml_wrapper::error::NvmlError;
    use nvml_wrapper::struct_wrappers::device::MemoryInfo as NvmlMemoryInfo;

    #[test]
    fn maps_snapshot_into_vendor_agnostic_device() {
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
                    ..Default::default()
                },
                last_apply: Some(GpuApplyRecord::failure("failed to reapply saved config")),
            },
        );

        let device = map_snapshot(
            NvidiaSnapshot {
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
            },
            InventoryContext {
                managed_by_daemon: true,
                daemon_reachable: true,
                persisted_state: Some(&state),
                daemon_warning: None,
            },
            state.record_for("GPU-123"),
        );

        assert_eq!(device.vendor, GpuVendor::Nvidia);
        assert_eq!(device.memory.total_mi_b, Some(32_768));
        assert_eq!(device.power.enforced_limit_watts, Some(475));
        assert_eq!(device.compute.api, Some(ComputeApi::Cuda));
        assert_eq!(device.compute.capability.as_deref(), Some("12.0"));
        assert_eq!(device.control.power_limit.desired_watts, Some(450));
        assert!(device
            .control
            .warnings
            .iter()
            .any(|warning| warning.contains("failed to reapply")));
    }

    #[test]
    fn conversion_helpers_use_expected_units() {
        assert_eq!(bytes_to_mib(1_073_741_824), 1_024);
        assert_eq!(milliwatts_to_watts(475_000), 475);
        assert_eq!(format_cuda_driver_version(13_020), "13.2");
    }

    #[test]
    fn validates_discrete_clock_range_against_supported_values() {
        let error = validate_discrete_clock_range(
            "GPU clock lock",
            ClockRangeInput {
                min_mhz: 1550,
                max_mhz: 2400,
            },
            &[1500, 1800, 2400],
        )
        .expect_err("validation should fail");

        assert_eq!(error.code, GpuCommandErrorCode::InvalidArgument);
    }

    #[test]
    fn validates_offset_against_bounds() {
        let error = validate_offset_range(
            "graphics offset",
            250,
            &ClockOffsetSnapshot {
                current_mhz: 0,
                min_mhz: -200,
                max_mhz: 200,
            },
        )
        .expect_err("validation should fail");

        assert_eq!(error.code, GpuCommandErrorCode::InvalidArgument);
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
        assert_eq!(telemetry.memory_used_mi_b, Some(12_288));
        assert_eq!(telemetry.memory_total_mi_b, Some(32_768));
        assert_eq!(telemetry.performance_state.as_deref(), Some("P0"));
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("graphics clock"));
    }

    #[test]
    fn parses_fallback_gpu_ids() {
        assert_eq!(parse_fallback_index("nvidia-3"), Some(3));
        assert_eq!(parse_fallback_index("GPU-123"), None);
    }
}
