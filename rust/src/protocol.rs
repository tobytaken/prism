use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::cooling::{ApplyCoolingRequest, CoolingApplication};
use crate::drivers::{DriverJob, DriverJobRequest, DriverStatusResponse, StartDriverActionRequest};
use crate::fan_curve::{FanCurvePreview, PreviewFanCurveRequest};
use crate::gpu::{
    GpuCommandError, GpuDaemonStatus, GpuInventoryResponse, GpuPerformanceResponse,
    GpuTargetRequest, SetGpuFanModeAutoRequest, SetGpuFanSpeedRequest, SetGpuPowerLimitRequest,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileSettings {
    #[serde(
        default,
        deserialize_with = "included_cooling",
        skip_serializing_if = "Option::is_none"
    )]
    pub cooling: Option<crate::cooling::CoolingConfiguration>,
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
        self.cooling.is_none()
            && ProfileSettingKey::ALL
                .iter()
                .all(|key| self.get(*key).is_none())
    }
}

fn included_cooling<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<crate::cooling::CoolingConfiguration>, D::Error> {
    crate::cooling::CoolingConfiguration::deserialize(deserializer).map(Some)
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
    /// Automatic operations only. Manual applications never roll back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollback: Option<RollbackResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooling: Option<crate::cooling::CoolingApplication>,
    pub power: Option<TuningOutcome>,
    pub graphics: Option<TuningOutcome>,
    pub memory: Option<TuningOutcome>,
    // Present only when out-of-range numeric values are the sole obstacle; nothing was written.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corrections: Vec<ProfileRangeCorrection>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RollbackResult {
    pub status: RollbackStatus,
    pub power: Option<TuningOutcome>,
    pub graphics: Option<TuningOutcome>,
    pub memory: Option<TuningOutcome>,
    pub cooling: Option<crate::cooling::CoolingApplication>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cooling_restoration: Vec<crate::cooling::FanRestoration>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RollbackStatus {
    Restored,
    RestoreFailed,
    #[default]
    NotNeeded,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
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
#[serde(untagged, deny_unknown_fields)]
pub enum ProfileRangeCorrection {
    Numeric(NumericRangeCorrection),
    Cooling {
        setting: CoolingSettingKey,
        #[serde(rename = "savedValue")]
        saved_value: crate::cooling::CoolingConfiguration,
        #[serde(rename = "proposedValue")]
        proposed_value: crate::cooling::CoolingConfiguration,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum CoolingSettingKey {
    #[serde(rename = "cooling")]
    Cooling,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NumericRangeCorrection {
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
    /// The GPU's state revision the caller last saw.
    pub revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveToCurrentProfileRequest {
    /// The UUID of the GPU whose current profile is overwritten.
    pub gpu_id: String,
    /// The GPU's state revision the caller last saw.
    pub revision: String,
}

/// A GPU's current profile, visible by name to every user whoever owns it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CurrentProfile {
    pub id: String,
    pub name: String,
}

/// What a GPU's profile features rely on: its current profile, and the revision of its
/// reported and recorded tuning state that a save must echo.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GpuTuningState {
    /// The GPU's UUID.
    pub gpu_id: String,
    /// Changes whenever the GPU's reported or recorded tuning state changes.
    pub revision: String,
    /// The current tuning values the revision covers, so the app can tell whether they are the
    /// values it shows.
    pub reported: ReportedTuningValues,
    pub current_profile: Option<CurrentProfile>,
}

/// Each profile setting's current tuning value, or null when it can't be read.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReportedTuningValues {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooling: Option<crate::cooling::CoolingConfiguration>,
    pub power_limit_watts: Option<i64>,
    #[serde(rename = "graphicsOffsetMHz")]
    pub graphics_offset_mhz: Option<i64>,
    #[serde(rename = "memoryOffsetMHz")]
    pub memory_offset_mhz: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileIdRequest {
    pub profile_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenameProfileRequest {
    pub profile_id: String,
    pub name: String,
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

/// The host-wide choice of what the daemon puts into effect when the host starts.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StartupBehavior {
    #[default]
    Off,
    StartupProfiles,
    RestoreLastApplied,
}

impl StartupBehavior {
    pub fn is_off(&self) -> bool {
        *self == Self::Off
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetStartupBehaviorRequest {
    pub behavior: StartupBehavior,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetStartupProfileRequest {
    pub gpu_id: String,
    /// None clears the designation, including one belonging to another user.
    #[serde(deserialize_with = "explicit_option")]
    pub profile_id: Option<String>,
}

// Require an explicit null to clear a choice; a missing field is a malformed request.
fn explicit_option<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartupGpuRequest {
    pub gpu_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartupProfile {
    pub gpu_id: String,
    /// Visible by name to all users; another user's values stay private.
    pub profile: CurrentProfile,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StartupOutcomeStatus {
    Applied,
    Failed,
    NotAttempted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartupOutcome {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_failure: Option<String>,
    pub gpu_id: String,
    /// The startup behavior this boot ran.
    #[serde(default)]
    pub behavior: StartupBehavior,
    /// The startup profile applied, or none when restoring last applied tuning values.
    pub profile: Option<CurrentProfile>,
    pub status: StartupOutcomeStatus,
    pub reason: Option<String>,
    pub application: Option<ProfileApplicationResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartupState {
    pub behavior: StartupBehavior,
    pub profiles: Vec<StartupProfile>,
    pub outcomes: Vec<StartupOutcome>,
    /// GPU UUIDs whose last applied tuning values miss a manual write that couldn't be recorded.
    #[serde(default)]
    pub possibly_stale: Vec<String>,
}

/// A fully installed Steam app from the caller's Steam libraries.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SteamApp {
    pub app_id: u32,
    /// From the app's manifest.
    pub name: String,
}

/// What identifies a game: a Steam app by its app ID, whether it runs natively or through
/// Proton.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum GameId {
    #[serde(rename_all = "camelCase")]
    Steam {
        app_id: u32,
    },
    Program {
        path: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AddProgramRequest {
    pub path: String,
}

/// One of the caller's running processes, with a verified host location when recoverable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunningProgram {
    pub pid: u32,
    pub name: String,
    pub path: Option<String>,
    pub launcher: Option<String>,
    pub environment: Vec<String>,
    pub helper: bool,
    pub recognition_warning: Option<String>,
}

/// One of the caller's game associations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GameEntry {
    pub game: GameId,
    pub name: String,
    /// Whether the game is installed now; an association is kept while it isn't.
    pub installed: bool,
    /// The linked profile id per GPU UUID. GPUs without one are left unchanged.
    pub profiles: BTreeMap<String, String>,
}

/// A recognized game, identified by its earliest recognized process on this host boot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunningGame {
    pub game: GameId,
    pub pid: u32,
    /// Kernel clock ticks since boot, not wall-clock time.
    pub start_time_ticks: u64,
}

/// Unavailable is deliberately distinct from a successful scan with no games.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "camelCase", deny_unknown_fields)]
pub enum RunningGamesState {
    Available { games: Vec<RunningGame> },
    Unavailable { reason: String },
}

/// The caller's game associations and automatic switching opt-in.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GamesState {
    pub automatic_switching: bool,
    pub games: Vec<GameEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AddSteamGameRequest {
    pub app_id: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetGameProfileRequest {
    pub game: GameId,
    /// The GPU's UUID.
    pub gpu_id: String,
    /// None leaves the GPU unchanged while the game runs.
    #[serde(deserialize_with = "explicit_option")]
    pub profile_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GameRequest {
    pub game: GameId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetAutomaticSwitchingRequest {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetGpuSwitchingPausedRequest {
    pub gpu_id: String,
    pub paused: bool,
}

/// The caller's view of automatic switching: what holds it back overall, and each GPU that
/// automation drives or failed to drive. Other GPUs are left unchanged. It doesn't establish
/// which tuning values are in effect.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AutomaticSwitchingStatus {
    pub condition: SwitchingCondition,
    pub gpus: Vec<GpuSwitchingStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "camelCase", deny_unknown_fields)]
pub enum SwitchingCondition {
    // Struct variants, so deny_unknown_fields also rejects extra fields.
    /// The caller's games switch profiles as they run.
    Ready {},
    /// The caller hasn't opted in.
    Off {},
    /// No automatic writes or restorations happen, such as while the tuning store is unhealthy.
    Blocked { reason: String },
    /// Current values are held, and nothing is restored, until a complete scan succeeds.
    RecognitionUnavailable { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GpuSwitchingStatus {
    /// The GPU's UUID.
    pub gpu_id: String,
    pub state: GpuSwitchingState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application: Option<ProfileApplicationResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "camelCase", deny_unknown_fields)]
pub enum GpuSwitchingState {
    /// An explicit host-wide hold on this GPU, retained until Resume.
    Paused {},
    /// The caller's running game drives the GPU with its linked profile.
    #[serde(rename_all = "camelCase")]
    Active {
        game: GameId,
        game_name: String,
        profile: CurrentProfile,
    },
    WaitingForEdits {
        profile: CurrentProfile,
    },
    /// Another user's running game drives the GPU; its details stay private.
    AnotherUser {},
    /// The latest automatic operation fully restored the pre-game tuning values.
    Restored {},
    #[serde(rename_all = "camelCase")]
    ManualOverride {
        changed_outside_prism: bool,
    },
    /// The latest automatic application or restoration didn't complete.
    Failed {
        message: String,
    },
    /// Boot-scoped recovery state is unreadable; this GPU is left unchanged.
    RecoveryRequired {
        message: String,
    },
}

/// Replaces this connection's complete set of GPUs with unapplied profile-scope edits.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetEditHoldsRequest {
    pub gpu_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum DaemonRequest {
    OpenSession,
    SetEditHolds(SetEditHoldsRequest),
    GetInventory,
    GetPerformance,
    GetDaemonStatus,
    ApplyProfile(ApplyGpuProfileRequest),
    SetPowerLimit(SetGpuPowerLimitRequest),
    ResetPowerLimit(GpuTargetRequest),
    SetFanSpeed(SetGpuFanSpeedRequest),
    SetFanModeAuto(SetGpuFanModeAutoRequest),
    /// Checks a draft shared fan curve without changing any GPU setting.
    PreviewFanCurve(PreviewFanCurveRequest),
    /// Validates a cooling configuration against fresh readings, then makes it the GPU's applied
    /// cooling, replacing any curve its fans followed.
    ApplyCooling(ApplyCoolingRequest),
    SetClockOffsets(LiveClockOffsetsRequest),
    ResetClockOffsets(GpuTargetRequest),
    GetMigrationState,
    AdoptMigratedProfile(ProfileIdRequest),
    DismissMigrationNotice,
    GetStartupState,
    SetStartupBehavior(SetStartupBehaviorRequest),
    SetStartupProfile(SetStartupProfileRequest),
    DismissStartupOutcome(StartupGpuRequest),
    ListProfiles,
    GetTuningStates,
    SaveProfileAsNew(SaveProfileAsNewRequest),
    SaveToCurrentProfile(SaveToCurrentProfileRequest),
    RenameProfile(RenameProfileRequest),
    DeleteProfile(ProfileIdRequest),
    ApplyProfileById(ProfileIdRequest),
    CorrectAndApplyProfile(CorrectAndApplyProfileRequest),
    ImportProfiles(ImportProfilesRequest),
    ListSteamApps,
    AddProgram(AddProgramRequest),
    ListRunningPrograms,
    GetGames,
    GetRunningGames,
    AddSteamGame(AddSteamGameRequest),
    SetGameProfile(SetGameProfileRequest),
    RemoveGame(GameRequest),
    SetAutomaticSwitching(SetAutomaticSwitchingRequest),
    SetGpuSwitchingPaused(SetGpuSwitchingPausedRequest),
    GetAutomaticSwitchingStatus,
    RetryAutomaticSwitching(StartupGpuRequest),
    GetDriverStatus,
    StartDriverAction(StartDriverActionRequest),
    GetDriverJob(DriverJobRequest),
    CancelDriverJob(DriverJobRequest),
    RequestHostReboot,
}

impl DaemonRequest {
    /// Game requests, which read the caller's Steam libraries.
    pub fn is_game_request(&self) -> bool {
        matches!(
            self,
            Self::ListSteamApps
                | Self::AddProgram(_)
                | Self::ListRunningPrograms
                | Self::GetGames
                | Self::AddSteamGame(_)
                | Self::SetGameProfile(_)
                | Self::RemoveGame(_)
                | Self::SetAutomaticSwitching(_)
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum DaemonResponse {
    Inventory(GpuInventoryResponse),
    Performance(GpuPerformanceResponse),
    DaemonStatus(GpuDaemonStatus),
    Command(DaemonCommandResponse),
    ClockOffsets(ClockOffsetResponse),
    FanCurvePreview(FanCurvePreview),
    CoolingApplication(CoolingApplication),
    ProfileApplication(ProfileApplicationResponse),
    MigrationState(MigrationState),
    StartupState(StartupState),
    Profiles(Vec<GpuTuningProfile>),
    Profile(GpuTuningProfile),
    TuningStates(Vec<GpuTuningState>),
    CorrectedProfileApplication(CorrectedProfileApplication),
    ProfileImport(ProfileImportResponse),
    SteamApps(Vec<SteamApp>),
    RunningPrograms(Vec<RunningProgram>),
    Games(GamesState),
    RunningGames(RunningGamesState),
    AutomaticSwitchingStatus(AutomaticSwitchingStatus),
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
    TuningStateChanged(GpuTuningState),
    RunningGamesChanged(RunningGamesState),
    AutomaticSwitchingChanged(AutomaticSwitchingStatus),
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
    fn startup_state_round_trips_the_shared_contract_and_explicit_clearing() {
        let state: serde_json::Value =
            serde_json::from_str(include_str!("../../src/test/contracts/startup-state.json"))
                .unwrap();
        let typed: StartupState = serde_json::from_value(state.clone()).unwrap();
        assert_eq!(
            serde_json::to_value(DaemonResponse::StartupState(typed)).unwrap(),
            serde_json::json!({ "type": "startup_state", "payload": state })
        );
        for request in [
            serde_json::json!({ "type": "set_startup_profile", "payload": { "gpuId": "GPU-123", "profileId": null } }),
            serde_json::json!({ "type": "set_startup_profile", "payload": { "gpuId": "GPU-123", "profileId": "quiet" } }),
            serde_json::json!({ "type": "set_startup_behavior", "payload": { "behavior": "startupProfiles" } }),
            serde_json::json!({ "type": "dismiss_startup_outcome", "payload": { "gpuId": "GPU-123" } }),
        ] {
            let typed: DaemonRequest = serde_json::from_value(request.clone()).unwrap();
            assert_eq!(serde_json::to_value(typed).unwrap(), request);
        }
    }

    #[test]
    fn startup_requests_reject_missing_choices_and_client_ownership() {
        for malformed in [
            serde_json::json!({ "type": "set_startup_profile", "payload": { "gpuId": "GPU-123" } }),
            serde_json::json!({ "type": "set_startup_profile", "payload": { "gpuId": "GPU-123", "profileId": 7 } }),
            serde_json::json!({ "type": "set_startup_profile", "payload": { "gpuId": "GPU-123", "profileId": null, "uid": 0 } }),
            serde_json::json!({ "type": "set_startup_behavior", "payload": { "behavior": "always" } }),
            serde_json::json!({ "type": "dismiss_startup_outcome", "payload": { "gpuId": "GPU-123", "username": "alice" } }),
        ] {
            assert!(
                serde_json::from_value::<DaemonRequest>(malformed.clone()).is_err(),
                "{malformed}"
            );
        }
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
    fn tuning_states_round_trip_the_shared_contract_as_a_response_and_as_pushed_events() {
        let states: serde_json::Value = serde_json::from_str(include_str!(
            "../../src/test/contracts/gpu-tuning-states.json"
        ))
        .unwrap();
        let typed: Vec<GpuTuningState> = serde_json::from_value(states.clone()).unwrap();
        assert_eq!(
            serde_json::to_value(DaemonResponse::TuningStates(typed.clone())).unwrap(),
            serde_json::json!({ "type": "tuning_states", "payload": states })
        );
        let event = SessionMessage::Event {
            event: DaemonEvent::TuningStateChanged(typed[0].clone()),
        };
        let json = serde_json::json!({
            "type": "event", "event": { "type": "tuningStateChanged", "payload": states[0] }
        });
        assert_eq!(serde_json::to_value(&event).unwrap(), json);
        assert_eq!(
            serde_json::from_value::<SessionMessage>(json).unwrap(),
            event
        );
    }

    #[test]
    fn profile_management_requests_round_trip_and_reject_client_supplied_ownership() {
        for request in [
            serde_json::json!({ "type": "rename_profile", "payload": { "profileId": "quiet", "name": "Silent" } }),
            serde_json::json!({ "type": "delete_profile", "payload": { "profileId": "quiet" } }),
        ] {
            let typed: DaemonRequest = serde_json::from_value(request.clone()).unwrap();
            assert_eq!(serde_json::to_value(typed).unwrap(), request);
            let mut malformed = request;
            malformed["payload"]["uid"] = serde_json::json!(0);
            assert!(serde_json::from_value::<DaemonRequest>(malformed).is_err());
        }
        for malformed in [
            serde_json::json!({ "type": "rename_profile", "payload": { "profileId": "quiet" } }),
            serde_json::json!({ "type": "rename_profile", "payload": { "profileId": "quiet", "name": 7 } }),
            serde_json::json!({ "type": "delete_profile", "payload": { "profileId": 7 } }),
        ] {
            assert!(serde_json::from_value::<DaemonRequest>(malformed).is_err());
        }
    }

    #[test]
    fn saves_carry_the_revision_the_caller_saw() {
        let request: DaemonRequest = serde_json::from_value(serde_json::json!({
            "type": "save_to_current_profile",
            "payload": { "gpuId": "GPU-123", "revision": "4f1c2b9d0e8a7c35" }
        }))
        .unwrap();
        assert_eq!(
            request,
            DaemonRequest::SaveToCurrentProfile(SaveToCurrentProfileRequest {
                gpu_id: "GPU-123".into(),
                revision: "4f1c2b9d0e8a7c35".into(),
            })
        );
        for malformed in [
            serde_json::json!({"type": "save_to_current_profile", "payload": {"gpuId": "GPU-123"}}),
            serde_json::json!({"type": "save_to_current_profile", "payload": {"gpuId": "GPU-123", "revision": 7}}),
            serde_json::json!({"type": "save_to_current_profile", "payload": {"gpuId": "GPU-123", "revision": "r", "profileId": "quiet"}}),
            serde_json::json!({"type": "save_profile_as_new", "payload": {"gpuId": "GPU-123", "name": "Quiet"}}),
            serde_json::json!({"type": "get_tuning_states", "payload": {"gpuId": "GPU-123"}}),
        ] {
            assert!(
                serde_json::from_value::<DaemonRequest>(malformed.clone()).is_err(),
                "{malformed}"
            );
        }
        let malformed = serde_json::json!({"type": "event", "event": {"type": "tuningStateChanged", "payload": {"gpuId": "GPU-123", "revision": "r", "currentProfile": null, "uid": 0}}});
        assert!(serde_json::from_value::<SessionMessage>(malformed).is_err());
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
    fn game_payloads_round_trip_the_shared_contract_and_reject_client_ownership() {
        let state: serde_json::Value =
            serde_json::from_str(include_str!("../../src/test/contracts/games-state.json"))
                .unwrap();
        let typed: GamesState = serde_json::from_value(state.clone()).unwrap();
        assert_eq!(
            serde_json::to_value(DaemonResponse::Games(typed)).unwrap(),
            serde_json::json!({ "type": "games", "payload": state })
        );
        assert_eq!(
            serde_json::to_value(DaemonResponse::SteamApps(vec![SteamApp {
                app_id: 570,
                name: "Dota 2".into()
            }]))
            .unwrap(),
            serde_json::json!({ "type": "steam_apps", "payload": [{ "appId": 570, "name": "Dota 2" }] })
        );
        for request in [
            serde_json::json!({ "type": "list_steam_apps" }),
            serde_json::json!({ "type": "list_running_programs" }),
            serde_json::json!({ "type": "add_program", "payload": { "path": "/games/game.exe" } }),
            serde_json::json!({ "type": "set_game_profile", "payload": { "game": { "kind": "program", "path": "/games/game.exe" }, "gpuId": "GPU-123", "profileId": "quiet" } }),
            serde_json::json!({ "type": "get_games" }),
            serde_json::json!({ "type": "add_steam_game", "payload": { "appId": 570 } }),
            serde_json::json!({ "type": "set_game_profile", "payload": { "game": { "kind": "steam", "appId": 570 }, "gpuId": "GPU-123", "profileId": "quiet" } }),
            serde_json::json!({ "type": "set_game_profile", "payload": { "game": { "kind": "steam", "appId": 570 }, "gpuId": "GPU-123", "profileId": null } }),
            serde_json::json!({ "type": "remove_game", "payload": { "game": { "kind": "steam", "appId": 570 } } }),
            serde_json::json!({ "type": "set_automatic_switching", "payload": { "enabled": true } }),
            serde_json::json!({ "type": "set_gpu_switching_paused", "payload": { "gpuId": "GPU-123", "paused": true } }),
            serde_json::json!({ "type": "set_gpu_switching_paused", "payload": { "gpuId": "GPU-123", "paused": false } }),
        ] {
            let typed: DaemonRequest = serde_json::from_value(request.clone()).unwrap();
            assert_eq!(serde_json::to_value(typed).unwrap(), request);
        }
        for malformed in [
            serde_json::json!({ "type": "add_steam_game", "payload": { "appId": "570" } }),
            serde_json::json!({ "type": "add_steam_game", "payload": { "appId": -1 } }),
            serde_json::json!({ "type": "add_steam_game", "payload": { "appId": 570, "name": "Dota 2" } }),
            serde_json::json!({ "type": "set_game_profile", "payload": { "game": { "kind": "steam", "appId": 570 }, "gpuId": "GPU-123" } }),
            serde_json::json!({ "type": "set_game_profile", "payload": { "game": { "kind": "program", "appId": 570 }, "gpuId": "GPU-123", "profileId": null } }),
            serde_json::json!({ "type": "set_game_profile", "payload": { "game": { "kind": "steam", "appId": 570, "uid": 0 }, "gpuId": "GPU-123", "profileId": null } }),
            serde_json::json!({ "type": "remove_game", "payload": { "game": { "kind": "steam" } } }),
            serde_json::json!({ "type": "set_automatic_switching", "payload": { "enabled": "yes" } }),
            serde_json::json!({ "type": "set_gpu_switching_paused", "payload": { "gpuId": "GPU-123", "paused": "yes" } }),
            serde_json::json!({ "type": "set_gpu_switching_paused", "payload": { "gpuId": "GPU-123" } }),
            serde_json::json!({ "type": "set_gpu_switching_paused", "payload": { "gpuId": "GPU-123", "paused": true, "uid": 0 } }),
            serde_json::json!({ "type": "set_automatic_switching", "payload": { "enabled": true, "uid": 0 } }),
            serde_json::json!({ "type": "get_games", "payload": { "uid": 0 } }),
        ] {
            assert!(
                serde_json::from_value::<DaemonRequest>(malformed.clone()).is_err(),
                "{malformed}"
            );
        }
    }

    #[test]
    fn automatic_switching_status_round_trips_the_shared_contract_and_rejects_extras() {
        let statuses: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../src/test/contracts/automatic-switching-status.json"
        ))
        .unwrap();
        for status in statuses {
            let typed: AutomaticSwitchingStatus = serde_json::from_value(status.clone()).unwrap();
            assert_eq!(
                serde_json::to_value(DaemonResponse::AutomaticSwitchingStatus(typed)).unwrap(),
                serde_json::json!({ "type": "automatic_switching_status", "payload": status })
            );
        }
        let request = serde_json::json!({ "type": "get_automatic_switching_status" });
        let typed: DaemonRequest = serde_json::from_value(request.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), request);
        for malformed in [
            serde_json::json!({ "condition": { "status": "ready", "reason": "x" }, "gpus": [] }),
            serde_json::json!({ "condition": { "status": "paused" }, "gpus": [] }),
            serde_json::json!({ "condition": { "status": "off" }, "gpus": [{ "gpuId": "GPU-123", "state": { "status": "anotherUser", "uid": 1000 } }] }),
            serde_json::json!({ "condition": { "status": "off" }, "gpus": [{ "gpuId": "GPU-123", "state": { "status": "active", "gameName": "Dota 2" } }] }),
        ] {
            assert!(
                serde_json::from_value::<AutomaticSwitchingStatus>(malformed.clone()).is_err(),
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

/// Ownerless profiles are readable by every authorized caller, separately from their own.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MigrationState {
    pub profiles: Vec<GpuTuningProfile>,
    pub can_adopt: bool,
    pub notice: Option<MigrationNotice>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MigrationNotice {
    pub dropped: Vec<DroppedLegacyConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DroppedLegacyConfig {
    pub key: String,
    pub values: serde_json::Value,
    pub reason: String,
}
