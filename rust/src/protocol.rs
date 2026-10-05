use serde::{Deserialize, Serialize};

use crate::drivers::{DriverJob, DriverJobRequest, DriverStatusResponse, StartDriverActionRequest};
use crate::gpu::{
    ApplyGpuConfigRequest, GpuCommandError, GpuDaemonStatus, GpuInventoryResponse,
    GpuMutationResponse, GpuPerformanceResponse, GpuTargetRequest, ResetGpuConfigRequest,
    SetGpuFanModeAutoRequest, SetGpuFanSpeedRequest, SetGpuPowerLimitRequest,
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

impl ProfileSettings {
    pub fn get(&self, key: ProfileSettingKey) -> Option<ProfileSetting> {
        match key {
            ProfileSettingKey::PowerLimitWatts => self.power_limit_watts,
            ProfileSettingKey::GraphicsOffsetMhz => self.graphics_offset_mhz,
            ProfileSettingKey::MemoryOffsetMhz => self.memory_offset_mhz,
        }
    }

    pub fn set(&mut self, key: ProfileSettingKey, setting: ProfileSetting) {
        *match key {
            ProfileSettingKey::PowerLimitWatts => &mut self.power_limit_watts,
            ProfileSettingKey::GraphicsOffsetMhz => &mut self.graphics_offset_mhz,
            ProfileSettingKey::MemoryOffsetMhz => &mut self.memory_offset_mhz,
        } = Some(setting);
    }

    pub fn is_empty(&self) -> bool {
        ProfileSettingKey::ALL
            .iter()
            .all(|key| self.get(*key).is_none())
    }
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

impl ProfileSettingKey {
    pub const ALL: [Self; 3] = [
        Self::PowerLimitWatts,
        Self::GraphicsOffsetMhz,
        Self::MemoryOffsetMhz,
    ];
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

/// A live clock offset instruction: a number of MHz, or an explicit request for the default.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum OffsetInstruction {
    Value(i32),
    Default(DefaultKeyword),
}

/// The literal `"default"`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DefaultKeyword {
    #[serde(rename = "default")]
    Default,
}

/// A live clock offset change. Omitted offsets are left unchanged.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiveClockOffsetsRequest {
    pub gpu_id: String,
    #[serde(default, rename = "graphicsOffsetMHz")]
    pub graphics_offset_mhz: Option<OffsetInstruction>,
    #[serde(default, rename = "memoryOffsetMHz")]
    pub memory_offset_mhz: Option<OffsetInstruction>,
}

/// A GPU tuning profile, owned by the user who saved or imported it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GpuTuningProfile {
    /// Opaque, assigned by the daemon or kept from an imported profile.
    pub id: String,
    pub name: String,
    /// The owning GPU's UUID.
    pub gpu_id: String,
    pub settings: ProfileSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveProfileAsNewRequest {
    /// The owning GPU's UUID.
    pub gpu_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileRequest {
    pub profile_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CorrectAndApplyProfileRequest {
    pub profile_id: String,
    /// The reviewed corrections; each must still match the profile's saved value.
    pub corrections: Vec<ProfileRangeCorrection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CorrectedProfileApplication {
    /// The profile as saved with the corrections, whatever the application's outcome.
    pub profile: GpuTuningProfile,
    pub application: ProfileApplicationResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportProfilesRequest {
    pub profiles: Vec<GpuTuningProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileImportResponse {
    /// The profiles added, with any name changed to avoid a clash.
    pub imported: Vec<GpuTuningProfile>,
    /// Ids of profiles skipped because a profile with that id already exists.
    pub skipped: Vec<String>,
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
    SetClockOffsets(LiveClockOffsetsRequest),
    ResetClockOffsets(GpuTargetRequest),
    ListProfiles,
    SaveProfileAsNew(SaveProfileAsNewRequest),
    ApplyProfileById(ProfileRequest),
    CorrectAndApplyProfile(CorrectAndApplyProfileRequest),
    ImportProfiles(ImportProfilesRequest),
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
    Profiles(Vec<GpuTuningProfile>),
    Profile(GpuTuningProfile),
    CorrectedProfileApplication(CorrectedProfileApplication),
    ProfileImport(ProfileImportResponse),
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
    fn profile_payloads_round_trip_the_shared_contracts() {
        let profiles: serde_json::Value = serde_json::from_str(include_str!(
            "../../src/test/contracts/gpu-tuning-profiles.json"
        ))
        .unwrap();
        let typed: Vec<GpuTuningProfile> = serde_json::from_value(profiles.clone()).unwrap();
        assert_eq!(serde_json::to_value(&typed).unwrap(), profiles);
        let corrected: serde_json::Value = serde_json::from_str(include_str!(
            "../../src/test/contracts/corrected-profile-application.json"
        ))
        .unwrap();
        let typed: CorrectedProfileApplication = serde_json::from_value(corrected.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), corrected);
    }

    #[test]
    fn profile_requests_reject_malformed_payloads() {
        for malformed in [
            serde_json::json!({"type": "save_profile_as_new", "payload": {"gpuId": "GPU-123"}}),
            serde_json::json!({"type": "save_profile_as_new", "payload": {"gpuId": "GPU-123", "name": "Quiet", "settings": {}}}),
            serde_json::json!({"type": "apply_profile_by_id", "payload": {"profileId": 7}}),
            serde_json::json!({"type": "correct_and_apply_profile", "payload": {"profileId": "quiet", "corrections": [{"setting": "fanSpeed", "savedValue": 1, "proposedValue": 0, "bound": "maximum"}]}}),
            serde_json::json!({"type": "import_profiles", "payload": {"profiles": [{"id": "a", "name": "A", "gpuId": "GPU-123", "settings": {"powerLimitWatts": 300}}]}}),
            serde_json::json!({"type": "import_profiles", "payload": {"profiles": [{"id": "a", "name": "A", "gpuId": "GPU-123", "settings": {}, "uid": 0}]}}),
        ] {
            assert!(
                serde_json::from_value::<DaemonRequest>(malformed.clone()).is_err(),
                "{malformed}"
            );
        }
    }

    #[test]
    fn live_clock_offsets_accept_numbers_default_requests_and_null_for_unchanged() {
        let request: LiveClockOffsetsRequest = serde_json::from_value(serde_json::json!({
            "gpuId": "GPU-123", "graphicsOffsetMHz": "default", "memoryOffsetMHz": null
        }))
        .unwrap();
        assert_eq!(
            request.graphics_offset_mhz,
            Some(OffsetInstruction::Default(DefaultKeyword::Default))
        );
        assert_eq!(request.memory_offset_mhz, None);
        let request: LiveClockOffsetsRequest = serde_json::from_value(serde_json::json!({
            "gpuId": "GPU-123", "memoryOffsetMHz": -80
        }))
        .unwrap();
        assert_eq!(
            request.memory_offset_mhz,
            Some(OffsetInstruction::Value(-80))
        );
        for malformed in [
            serde_json::json!({"gpuId": "GPU-123", "graphicsOffsetMHz": "reset"}),
            serde_json::json!({"gpuId": "GPU-123", "graphicsOffsetMHz": 1.5}),
            serde_json::json!({"gpuId": "GPU-123", "graphicsOffsetMhz": 100}),
        ] {
            assert!(
                serde_json::from_value::<LiveClockOffsetsRequest>(malformed.clone()).is_err(),
                "{malformed}"
            );
        }
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
