use serde::{Deserialize, Serialize};

use crate::drivers::{DriverJob, DriverJobRequest, DriverStatusResponse, StartDriverActionRequest};
use crate::gpu::{
    ApplyGpuConfigRequest, GpuCommandError, GpuDaemonStatus, GpuInventoryResponse,
    GpuMutationResponse, GpuPerformanceResponse, GpuTargetRequest, ResetGpuConfigRequest,
    SetGpuClockOffsetsRequest, SetGpuFanModeAutoRequest, SetGpuFanSpeedRequest,
    SetGpuPowerLimitRequest,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DaemonCommandResponse {
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TuningOutcomeStatus {
    Succeeded,
    Failed,
    Unattempted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TuningOutcome {
    pub status: TuningOutcomeStatus,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClockOffsetResponse {
    pub graphics: Option<TuningOutcome>,
    pub memory: Option<TuningOutcome>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NumericProfileSettings {
    #[serde(
        default,
        deserialize_with = "included_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub power_limit_watts: Option<i64>,
    #[serde(
        default,
        rename = "graphicsOffsetMHz",
        deserialize_with = "included_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub graphics_offset_mhz: Option<i64>,
    #[serde(
        default,
        rename = "memoryOffsetMHz",
        deserialize_with = "included_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub memory_offset_mhz: Option<i64>,
}

// An included null is invalid; only an omitted instruction leaves a control alone.
fn included_value<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<i64>, D::Error> {
    i64::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyGpuProfileRequest {
    pub gpu_id: String,
    pub settings: NumericProfileSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileApplicationResponse {
    pub power: Option<TuningOutcome>,
    pub graphics: Option<TuningOutcome>,
    pub memory: Option<TuningOutcome>,
    // Present only when out-of-range numeric values are the sole obstacle; nothing was written.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corrections: Vec<ProfileRangeCorrection>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProfileSettingKey {
    #[serde(rename = "powerLimitWatts")]
    PowerLimitWatts,
    #[serde(rename = "graphicsOffsetMHz")]
    GraphicsOffsetMhz,
    #[serde(rename = "memoryOffsetMHz")]
    MemoryOffsetMhz,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RangeBound {
    Minimum,
    Maximum,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileRangeCorrection {
    pub setting: ProfileSettingKey,
    pub saved_value: i64,
    pub proposed_value: i64,
    pub bound: RangeBound,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum DaemonRequest {
    GetInventory,
    GetPerformance,
    GetDaemonStatus,
    ApplyConfig(ApplyGpuConfigRequest),
    ApplyProfile(ApplyGpuProfileRequest),
    ResetConfig(ResetGpuConfigRequest),
    ReapplySavedConfig,
    SetPowerLimit(SetGpuPowerLimitRequest),
    ResetPowerLimit(GpuTargetRequest),
    SetFanSpeed(SetGpuFanSpeedRequest),
    SetFanModeAuto(SetGpuFanModeAutoRequest),
    SetClockOffsets(SetGpuClockOffsetsRequest),
    ResetClockOffsets(GpuTargetRequest),
    GetDriverStatus,
    StartDriverAction(StartDriverActionRequest),
    GetDriverJob(DriverJobRequest),
    CancelDriverJob(DriverJobRequest),
    RequestHostReboot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum DaemonResponse {
    Inventory(GpuInventoryResponse),
    Performance(GpuPerformanceResponse),
    DaemonStatus(GpuDaemonStatus),
    Mutation(GpuMutationResponse),
    Command(DaemonCommandResponse),
    ClockOffsets(ClockOffsetResponse),
    ProfileApplication(ProfileApplicationResponse),
    DriverStatus(DriverStatusResponse),
    DriverJob(DriverJob),
    Error(GpuCommandError),
}
