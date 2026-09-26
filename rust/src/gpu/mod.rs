mod nvidia;

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use nvidia::{
    collect_nvidia_performance, reset_nvidia_clock_offsets, reset_nvidia_power_limit,
    set_nvidia_clock_offsets, set_nvidia_fan_auto, set_nvidia_fan_speed, set_nvidia_power_limit,
    NvmlNvidiaProvider,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GpuInventoryResponse {
    pub gpus: Vec<GpuDevice>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
pub struct MemoryInfo {
    pub total_mi_b: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ComputeInfo {
    pub api: Option<ComputeApi>,
    pub api_version: Option<String>,
    pub capability: Option<String>,
    pub core_count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PcieInfo {
    pub gen_max: Option<u32>,
    pub width_max: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PowerInfo {
    pub enforced_limit_watts: Option<u32>,
    pub default_limit_watts: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClockInfo {
    #[serde(rename = "graphicsCurrentMHz")]
    pub graphics_current_mhz: Option<u32>,
    #[serde(rename = "graphicsMaxMHz")]
    pub graphics_max_mhz: Option<u32>,
    #[serde(rename = "smMaxMHz")]
    pub sm_max_mhz: Option<u32>,
    #[serde(rename = "memoryMaxMHz")]
    pub memory_max_mhz: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GpuPerformanceResponse {
    pub gpus: Vec<GpuPerformanceDevice>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GpuPerformanceDevice {
    pub id: String,
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
#[serde(rename_all = "camelCase")]
pub struct GpuTelemetryInfo {
    pub power_draw_watts: Option<u32>,
    #[serde(rename = "coreClockMHz")]
    pub core_clock_mhz: Option<u32>,
    #[serde(rename = "memoryClockMHz")]
    pub memory_clock_mhz: Option<u32>,
    pub fan_speed_percent: Option<u32>,
    pub voltage_millivolts: Option<u32>,
    pub temperature_c: Option<u32>,
    pub gpu_utilization_percent: Option<u32>,
    pub memory_used_mi_b: Option<u64>,
    pub memory_total_mi_b: Option<u64>,
    pub performance_state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PowerControlInfo {
    pub is_supported: bool,
    pub unsupported_reason: Option<String>,
    pub current_draw_watts: Option<u32>,
    pub current_limit_watts: Option<u32>,
    pub default_limit_watts: Option<u32>,
    pub enforced_limit_watts: Option<u32>,
    pub min_limit_watts: Option<u32>,
    pub max_limit_watts: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CoolingInfo {
    pub is_supported: bool,
    pub unsupported_reason: Option<String>,
    pub fan_count: u32,
    pub fans: Vec<FanControlInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
pub struct ClockOffsetsInfo {
    pub is_supported: bool,
    pub unsupported_reason: Option<String>,
    pub graphics: ClockOffsetInfo,
    pub memory: ClockOffsetInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClockOffsetInfo {
    pub is_supported: bool,
    pub unsupported_reason: Option<String>,
    #[serde(rename = "currentOffsetMHz")]
    pub current_offset_mhz: Option<i32>,
    #[serde(rename = "minOffsetMHz")]
    pub min_offset_mhz: Option<i32>,
    #[serde(rename = "maxOffsetMHz")]
    pub max_offset_mhz: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SetGpuPowerLimitRequest {
    pub gpu_id: String,
    pub limit_watts: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GpuTargetRequest {
    pub gpu_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SetGpuFanSpeedRequest {
    pub gpu_id: String,
    pub fan_index: u32,
    pub speed_percent: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SetGpuFanModeAutoRequest {
    pub gpu_id: String,
    pub fan_index: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SetGpuClockOffsetsRequest {
    pub gpu_id: String,
    pub graphics_offset_mhz: Option<i32>,
    pub memory_offset_mhz: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GpuControlInfo {
    pub managed_by_daemon: bool,
    pub daemon_reachable: bool,
    pub supported_features: GpuSupportedFeatures,
    pub power_limit: PowerLimitControl,
    pub gpu_clock_lock: ClockLockControl,
    pub memory_clock_lock: ClockLockControl,
    pub offsets: OffsetControl,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GpuSupportedFeatures {
    pub power_limit: bool,
    pub gpu_clock_lock: bool,
    pub memory_clock_lock: bool,
    pub graphics_offset: bool,
    pub memory_offset: bool,
    pub reset: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PowerLimitControl {
    pub current_watts: Option<u32>,
    pub default_watts: Option<u32>,
    pub min_watts: Option<u32>,
    pub max_watts: Option<u32>,
    pub desired_watts: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClockLockControl {
    pub current_min_mhz: Option<u32>,
    pub current_max_mhz: Option<u32>,
    pub min_allowed_mhz: Option<u32>,
    pub max_allowed_mhz: Option<u32>,
    pub supported: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct OffsetControl {
    #[serde(rename = "graphicsMHz")]
    pub graphics_mhz: Option<i32>,
    #[serde(rename = "memoryMHz")]
    pub memory_mhz: Option<i32>,
    #[serde(rename = "minGraphicsMHz")]
    pub min_graphics_mhz: Option<i32>,
    #[serde(rename = "maxGraphicsMHz")]
    pub max_graphics_mhz: Option<i32>,
    #[serde(rename = "minMemoryMHz")]
    pub min_memory_mhz: Option<i32>,
    #[serde(rename = "maxMemoryMHz")]
    pub max_memory_mhz: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DesiredGpuConfig {
    pub power_limit_watts: Option<u32>,
    pub gpu_clock_lock: Option<ClockRangeInput>,
    pub memory_clock_lock: Option<ClockRangeInput>,
    #[serde(rename = "graphicsOffsetMHz")]
    pub graphics_offset_mhz: Option<i32>,
    #[serde(rename = "memoryOffsetMHz")]
    pub memory_offset_mhz: Option<i32>,
}

impl DesiredGpuConfig {
    pub fn is_empty(&self) -> bool {
        self.power_limit_watts.is_none()
            && self.gpu_clock_lock.is_none()
            && self.memory_clock_lock.is_none()
            && self.graphics_offset_mhz.is_none()
            && self.memory_offset_mhz.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GpuApplyRecord {
    pub applied_at_unix_ms: u64,
    pub success: bool,
    pub message: Option<String>,
}

impl GpuApplyRecord {
    pub fn success() -> Self {
        Self {
            applied_at_unix_ms: unix_timestamp_ms(),
            success: true,
            message: None,
        }
    }

    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            applied_at_unix_ms: unix_timestamp_ms(),
            success: false,
            message: Some(message.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PersistedGpuConfig {
    #[serde(default)]
    pub desired: DesiredGpuConfig,
    pub last_apply: Option<GpuApplyRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PersistedGpuState {
    #[serde(default)]
    pub gpus: BTreeMap<String, PersistedGpuConfig>,
}

impl PersistedGpuState {
    pub fn upsert_desired(&mut self, gpu_id: impl Into<String>, desired: DesiredGpuConfig) {
        let gpu_id = gpu_id.into();
        let record = self.gpus.entry(gpu_id.clone()).or_default();
        record.desired = desired;

        if record.desired.is_empty() {
            self.gpus.remove(&gpu_id);
        }
    }

    pub fn set_last_apply(&mut self, gpu_id: impl Into<String>, last_apply: GpuApplyRecord) {
        let gpu_id = gpu_id.into();
        let record = self.gpus.entry(gpu_id.clone()).or_default();
        record.last_apply = Some(last_apply);

        if record.desired.is_empty() && record.last_apply.is_none() {
            self.gpus.remove(&gpu_id);
        }
    }

    pub fn clear(&mut self, gpu_id: &str) {
        self.gpus.remove(gpu_id);
    }

    pub fn desired_for(&self, gpu_id: &str) -> Option<&DesiredGpuConfig> {
        self.gpus.get(gpu_id).map(|record| &record.desired)
    }

    pub fn record_for(&self, gpu_id: &str) -> Option<&PersistedGpuConfig> {
        self.gpus.get(gpu_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ApplyGpuConfigRequest {
    pub gpu_id: Option<String>,
    pub power_limit_watts: Option<u32>,
    pub gpu_clock_lock: Option<ClockRangeInput>,
    pub memory_clock_lock: Option<ClockRangeInput>,
    #[serde(rename = "graphicsOffsetMHz")]
    pub graphics_offset_mhz: Option<i32>,
    #[serde(rename = "memoryOffsetMHz")]
    pub memory_offset_mhz: Option<i32>,
    #[serde(default)]
    pub reapply_saved: bool,
}

impl ApplyGpuConfigRequest {
    pub fn validate(&self) -> Result<(), GpuCommandError> {
        if self.reapply_saved {
            if self.gpu_id.is_some()
                || self.power_limit_watts.is_some()
                || self.gpu_clock_lock.is_some()
                || self.memory_clock_lock.is_some()
                || self.graphics_offset_mhz.is_some()
                || self.memory_offset_mhz.is_some()
            {
                return Err(GpuCommandError::new(
                    GpuCommandErrorCode::InvalidArgument,
                    "reapplySaved cannot be combined with per-GPU settings",
                ));
            }

            return Ok(());
        }

        if self.gpu_id.as_deref().is_none() {
            return Err(GpuCommandError::new(
                GpuCommandErrorCode::InvalidArgument,
                "gpuId is required when applying a GPU configuration",
            ));
        }

        if self.power_limit_watts.is_none()
            && self.gpu_clock_lock.is_none()
            && self.memory_clock_lock.is_none()
            && self.graphics_offset_mhz.is_none()
            && self.memory_offset_mhz.is_none()
        {
            return Err(GpuCommandError::new(
                GpuCommandErrorCode::InvalidArgument,
                "at least one GPU setting must be provided",
            ));
        }

        if let Some(clock_range) = self.gpu_clock_lock.as_ref() {
            clock_range.validate("gpuClockLock")?;
        }

        if let Some(clock_range) = self.memory_clock_lock.as_ref() {
            clock_range.validate("memoryClockLock")?;
        }

        Ok(())
    }

    pub fn desired_config(&self) -> DesiredGpuConfig {
        DesiredGpuConfig {
            power_limit_watts: self.power_limit_watts,
            gpu_clock_lock: self.gpu_clock_lock,
            memory_clock_lock: self.memory_clock_lock,
            graphics_offset_mhz: self.graphics_offset_mhz,
            memory_offset_mhz: self.memory_offset_mhz,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResetGpuConfigRequest {
    pub gpu_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClockRangeInput {
    pub min_mhz: u32,
    pub max_mhz: u32,
}

impl ClockRangeInput {
    pub fn validate(&self, field: &str) -> Result<(), GpuCommandError> {
        if self.min_mhz > self.max_mhz {
            return Err(GpuCommandError::new(
                GpuCommandErrorCode::InvalidArgument,
                format!("{field} minMHz cannot be greater than maxMHz"),
            ));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GpuMutationResponse {
    pub success: bool,
    pub inventory: Option<GpuInventoryResponse>,
    pub error: Option<GpuCommandError>,
}

impl GpuMutationResponse {
    pub fn success(inventory: GpuInventoryResponse) -> Self {
        Self {
            success: true,
            inventory: Some(inventory),
            error: None,
        }
    }

    pub fn failure(error: GpuCommandError) -> Self {
        Self {
            success: false,
            inventory: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GpuDaemonStatus {
    pub reachable: bool,
    pub managed_by_daemon: bool,
    pub socket_path: String,
    pub message: Option<String>,
}

impl GpuDaemonStatus {
    pub fn connected(socket_path: impl Into<String>) -> Self {
        Self {
            reachable: true,
            managed_by_daemon: true,
            socket_path: socket_path.into(),
            message: None,
        }
    }

    pub fn disconnected(socket_path: impl Into<String>, message: Option<String>) -> Self {
        Self {
            reachable: false,
            managed_by_daemon: false,
            socket_path: socket_path.into(),
            message,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
pub enum GpuCommandErrorCode {
    DaemonUnavailable,
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

#[derive(Debug, Clone, Copy)]
pub struct InventoryContext<'a> {
    pub managed_by_daemon: bool,
    pub daemon_reachable: bool,
    pub persisted_state: Option<&'a PersistedGpuState>,
    pub daemon_warning: Option<&'a str>,
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
    fn collect(&self, context: InventoryContext<'_>) -> ProviderOutput;
}

pub(crate) trait GpuControlProvider {
    fn apply_config(&self, request: &ApplyGpuConfigRequest) -> Result<(), GpuCommandError>;
    fn reset_config(&self, gpu_id: &str) -> Result<(), GpuCommandError>;
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
    collect_inventory_with_context(InventoryContext {
        managed_by_daemon: false,
        daemon_reachable: false,
        persisted_state: None,
        daemon_warning: None,
    })
}

pub fn collect_inventory_with_context(context: InventoryContext<'_>) -> GpuInventoryResponse {
    let providers: [Box<dyn GpuInventoryProvider>; 1] = [Box::new(NvmlNvidiaProvider)];
    let mut output = ProviderOutput::default();

    for provider in providers {
        output.append(provider.collect(context));
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

pub fn apply_gpu_config(request: &ApplyGpuConfigRequest) -> Result<(), GpuCommandError> {
    request.validate()?;

    let providers: [Box<dyn GpuControlProvider>; 1] = [Box::new(NvmlNvidiaProvider)];
    let mut last_error = None;

    for provider in providers {
        match provider.apply_config(request) {
            Ok(()) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        GpuCommandError::new(
            GpuCommandErrorCode::NotFound,
            "the selected GPU could not be found",
        )
    }))
}

pub fn reset_gpu_config(gpu_id: &str) -> Result<(), GpuCommandError> {
    let providers: [Box<dyn GpuControlProvider>; 1] = [Box::new(NvmlNvidiaProvider)];
    let mut last_error = None;

    for provider in providers {
        match provider.reset_config(gpu_id) {
            Ok(()) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        GpuCommandError::new(
            GpuCommandErrorCode::NotFound,
            "the selected GPU could not be found",
        )
    }))
}

fn unix_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{
        ApplyGpuConfigRequest, ClockInfo, ClockLockControl, ClockOffsetInfo, ClockOffsetsInfo,
        ClockRangeInput, ComputeApi, ComputeInfo, CoolingInfo, DesiredGpuConfig, FanControlInfo,
        FanControlMode, GpuApplyRecord, GpuCommandErrorCode, GpuControlInfo, GpuDaemonStatus,
        GpuDevice, GpuInventoryResponse, GpuPerformanceDevice, GpuPerformanceResponse,
        GpuSupportedFeatures, GpuTelemetryInfo, GpuVendor, MemoryInfo, OffsetControl, PcieInfo,
        PersistedGpuState, PowerControlInfo, PowerInfo, PowerLimitControl,
    };

    #[test]
    fn apply_request_requires_gpu_id_for_non_reapply_calls() {
        let error = ApplyGpuConfigRequest {
            power_limit_watts: Some(250),
            ..Default::default()
        }
        .validate()
        .expect_err("validation should fail");

        assert_eq!(error.code, GpuCommandErrorCode::InvalidArgument);
    }

    #[test]
    fn apply_request_rejects_inverted_clock_ranges() {
        let error = ApplyGpuConfigRequest {
            gpu_id: Some("GPU-123".into()),
            gpu_clock_lock: Some(ClockRangeInput {
                min_mhz: 2200,
                max_mhz: 1800,
            }),
            ..Default::default()
        }
        .validate()
        .expect_err("validation should fail");

        assert_eq!(error.code, GpuCommandErrorCode::InvalidArgument);
    }

    #[test]
    fn apply_request_rejects_reapply_with_per_gpu_settings() {
        let error = ApplyGpuConfigRequest {
            reapply_saved: true,
            gpu_id: Some("GPU-123".into()),
            power_limit_watts: Some(250),
            ..Default::default()
        }
        .validate()
        .expect_err("validation should fail");

        assert_eq!(error.code, GpuCommandErrorCode::InvalidArgument);
    }

    #[test]
    fn apply_request_requires_at_least_one_setting() {
        let error = ApplyGpuConfigRequest {
            gpu_id: Some("GPU-123".into()),
            ..Default::default()
        }
        .validate()
        .expect_err("validation should fail");

        assert_eq!(error.code, GpuCommandErrorCode::InvalidArgument);
    }

    #[test]
    fn desired_config_reports_non_empty_for_each_setting() {
        assert!(!DesiredGpuConfig {
            power_limit_watts: Some(250),
            ..Default::default()
        }
        .is_empty());
        assert!(!DesiredGpuConfig {
            gpu_clock_lock: Some(ClockRangeInput {
                min_mhz: 1800,
                max_mhz: 2200,
            }),
            ..Default::default()
        }
        .is_empty());
        assert!(!DesiredGpuConfig {
            memory_clock_lock: Some(ClockRangeInput {
                min_mhz: 1000,
                max_mhz: 1200,
            }),
            ..Default::default()
        }
        .is_empty());
        assert!(!DesiredGpuConfig {
            graphics_offset_mhz: Some(100),
            ..Default::default()
        }
        .is_empty());
        assert!(!DesiredGpuConfig {
            memory_offset_mhz: Some(500),
            ..Default::default()
        }
        .is_empty());
    }

    #[test]
    fn persisted_state_removes_empty_desired_config() {
        let mut state = PersistedGpuState::default();
        state.upsert_desired("GPU-123", Default::default());

        assert!(state.gpus.is_empty());
    }

    #[test]
    fn persisted_state_clear_removes_existing_record() {
        let mut state = PersistedGpuState::default();
        state.upsert_desired(
            "GPU-123",
            DesiredGpuConfig {
                power_limit_watts: Some(250),
                ..Default::default()
            },
        );

        state.clear("GPU-123");

        assert!(state.gpus.is_empty());
    }

    #[test]
    fn mutation_failure_serializes_error_without_inventory() {
        let response = super::GpuMutationResponse::failure(super::GpuCommandError::new(
            GpuCommandErrorCode::Internal,
            "boom",
        ));

        let value = serde_json::to_value(response).expect("serialize response");

        assert_eq!(value["success"], false);
        assert_eq!(value["inventory"], serde_json::Value::Null);
        assert_eq!(value["error"]["code"], "internal");
        assert_eq!(value["error"]["message"], "boom");
    }

    #[test]
    fn inventory_response_serializes_camel_case() {
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
                    total_mi_b: Some(32_768),
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
                    managed_by_daemon: true,
                    daemon_reachable: true,
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
                        desired_watts: Some(450),
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

        assert_eq!(value["gpus"][0]["driverVersion"], "595.58.03");
        assert_eq!(value["gpus"][0]["memory"]["totalMiB"], 32_768);
        assert_eq!(value["gpus"][0]["compute"]["api"], "cuda");
        assert_eq!(value["gpus"][0]["clocks"]["graphicsCurrentMHz"], 1_365);
        assert_eq!(value["gpus"][0]["control"]["managedByDaemon"], true);
        assert_eq!(
            value["gpus"][0]["control"]["powerLimit"]["desiredWatts"],
            450
        );
    }

    #[test]
    fn performance_response_serializes_camel_case() {
        let response = GpuPerformanceResponse {
            gpus: vec![GpuPerformanceDevice {
                id: "GPU-123".into(),
                vendor: GpuVendor::Nvidia,
                name: "RTX".into(),
                architecture: Some("Blackwell".into()),
                bus_id: Some("0000:01:00.0".into()),
                power: PowerControlInfo {
                    is_supported: true,
                    unsupported_reason: None,
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
                },
                clocks: ClockOffsetsInfo {
                    is_supported: true,
                    unsupported_reason: None,
                    graphics: ClockOffsetInfo {
                        is_supported: true,
                        unsupported_reason: None,
                        current_offset_mhz: Some(120),
                        min_offset_mhz: Some(-200),
                        max_offset_mhz: Some(200),
                    },
                    memory: ClockOffsetInfo {
                        is_supported: true,
                        unsupported_reason: None,
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
                    memory_used_mi_b: Some(10_240),
                    memory_total_mi_b: Some(32_768),
                    performance_state: Some("P0".into()),
                },
            }],
            warnings: vec![],
        };

        let value = serde_json::to_value(response).expect("serialize response");

        assert_eq!(value["gpus"][0]["power"]["currentDrawWatts"], 312);
        assert_eq!(value["gpus"][0]["power"]["currentLimitWatts"], 475);
        assert_eq!(value["gpus"][0]["cooling"]["fans"][0]["mode"], "auto");
        assert_eq!(value["gpus"][0]["clocks"]["graphics"]["maxOffsetMHz"], 200);
        assert_eq!(value["gpus"][0]["telemetry"]["coreClockMHz"], 2_550);
        assert_eq!(value["gpus"][0]["telemetry"]["performanceState"], "P0");
    }

    #[test]
    fn apply_record_failure_sets_message() {
        let record = GpuApplyRecord::failure("boom");

        assert!(!record.success);
        assert_eq!(record.message.as_deref(), Some("boom"));
    }

    #[test]
    fn daemon_status_disconnected_preserves_socket_path() {
        let status = GpuDaemonStatus::disconnected("/run/prism/prismd.sock", None);

        assert!(!status.reachable);
        assert_eq!(status.socket_path, "/run/prism/prismd.sock");
    }
}
