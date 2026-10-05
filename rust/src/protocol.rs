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
    OpenSession,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionRequest {
    pub id: u64,
    pub request: DaemonRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum SessionMessage {
    Response { id: u64, response: DaemonResponse },
    Event { event: DaemonEvent },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "type",
    content = "payload",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum DaemonEvent {
    StoreHealthChanged(crate::gpu::TuningStoreHealth),
}

#[cfg(test)]
mod session_tests {
    use super::*;
    use crate::framing::{read_frame, write_frame};

    #[test]
    fn session_frames_round_trip_and_keep_response_ids_separate_from_events() {
        let request = SessionRequest {
            id: 42,
            request: DaemonRequest::GetDaemonStatus,
        };
        let event: SessionMessage = serde_json::from_str(include_str!(
            "../../src/test/contracts/session-store-health.json"
        ))
        .unwrap();
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &request).unwrap();
        write_frame(&mut bytes, &event).unwrap();
        let mut reader = std::io::Cursor::new(bytes);
        assert_eq!(
            read_frame::<SessionRequest>(&mut reader).unwrap(),
            Some(request)
        );
        assert_eq!(
            read_frame::<SessionMessage>(&mut reader).unwrap(),
            Some(event.clone())
        );
        assert_eq!(
            serde_json::to_value(event).unwrap(),
            serde_json::json!({
                "type": "event", "event": {"type": "storeHealthChanged", "payload": {"status": "healthy"}}
            })
        );
        assert!(read_frame::<SessionMessage>(&mut reader).unwrap().is_none());
    }

    #[test]
    fn session_frames_reject_truncation_oversize_and_malformed_payloads() {
        for bytes in [
            vec![0, 0],
            vec![0, 0, 0, 0],
            vec![0, 16, 0, 1],
            vec![0, 0, 0, 4, b'{'],
        ] {
            assert!(read_frame::<SessionRequest>(&mut std::io::Cursor::new(bytes)).is_err());
        }
        for malformed in [
            serde_json::json!({"id": -1, "request": {"type": "get_daemon_status"}}),
            serde_json::json!({"id": 1, "request": {"type": "unknown"}}),
            serde_json::json!({"id": 1, "request": null}),
            serde_json::json!({"id": 1, "request": {"type": "get_daemon_status"}, "uid": 0}),
        ] {
            let mut bytes = Vec::new();
            write_frame(&mut bytes, &malformed).unwrap();
            assert!(read_frame::<SessionRequest>(&mut std::io::Cursor::new(bytes)).is_err());
        }
        let malformed = serde_json::json!({"type": "event", "event": {"type": "storeHealthChanged", "payload": {"status": "healthy", "problem": "unreadable"}}});
        assert!(serde_json::from_value::<SessionMessage>(malformed).is_err());
    }
}
