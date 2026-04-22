use serde::{Deserialize, Serialize};

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
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum DaemonRequest {
    GetInventory,
    GetPerformance,
    GetDaemonStatus,
    ApplyConfig(ApplyGpuConfigRequest),
    ResetConfig(ResetGpuConfigRequest),
    ReapplySavedConfig,
    SetPowerLimit(SetGpuPowerLimitRequest),
    ResetPowerLimit(GpuTargetRequest),
    SetFanSpeed(SetGpuFanSpeedRequest),
    SetFanModeAuto(SetGpuFanModeAutoRequest),
    SetClockOffsets(SetGpuClockOffsetsRequest),
    ResetClockOffsets(GpuTargetRequest),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum DaemonResponse {
    Inventory(GpuInventoryResponse),
    Performance(GpuPerformanceResponse),
    DaemonStatus(GpuDaemonStatus),
    Mutation(GpuMutationResponse),
    Command(DaemonCommandResponse),
    Error(GpuCommandError),
}
