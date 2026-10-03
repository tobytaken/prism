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

/// A desired tuning instruction: a fixed number, or the default resolved at application time.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ProfileSetting {
    Value { value: i64 },
    // A struct variant so deny_unknown_fields also rejects `{ "kind": "default", "value": .. }`.
    Default {},
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileSettings {
    #[serde(
        default,
        deserialize_with = "included_setting",
        skip_serializing_if = "Option::is_none"
    )]
    pub power_limit_watts: Option<ProfileSetting>,
    #[serde(
        default,
        rename = "graphicsOffsetMHz",
        deserialize_with = "included_setting",
        skip_serializing_if = "Option::is_none"
    )]
    pub graphics_offset_mhz: Option<ProfileSetting>,
    #[serde(
        default,
        rename = "memoryOffsetMHz",
        deserialize_with = "included_setting",
        skip_serializing_if = "Option::is_none"
    )]
    pub memory_offset_mhz: Option<ProfileSetting>,
}

// An included null is invalid; only an omitted instruction leaves a control alone.
fn included_setting<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<ProfileSetting>, D::Error> {
    ProfileSetting::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyGpuProfileRequest {
    pub gpu_id: String,
    pub settings: ProfileSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileApplicationResponse {
    pub power: Option<TuningOutcome>,
    pub graphics: Option<TuningOutcome>,
    pub memory: Option<TuningOutcome>,
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
