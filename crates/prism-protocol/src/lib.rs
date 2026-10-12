pub mod client;
pub mod framing;

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use prism_drivers::{DriverJob, DriverJobRequest, DriverStatusResponse, StartDriverActionRequest};
use prism_gpu::fan_curve::{FanCurvePreview, PreviewFanCurveRequest};
use prism_gpu::{GpuCommandError, GpuInventoryResponse, GpuPerformanceResponse, TuningOutcome};

pub const DEFAULT_SOCKET_PATH: &str = "/run/prism/prismd.sock";

/// The version of the protocol Prism and prismd speak, as `major.minor`. Minor versions only add
/// to it: a side ignores events it doesn't know and answers requests it doesn't know with
/// [`DaemonResponse::Unsupported`]. Before the first release it is `0.x`, and the versions must
/// match exactly; it becomes `1.0` at the first release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProtocolVersion {
    pub major: u32,
    pub minor: u32,
}

impl ProtocolVersion {
    /// The version this build speaks.
    pub const CURRENT: Self = Self { major: 0, minor: 1 };

    /// Whether a side speaking this version can talk to one speaking `other`: the same major
    /// version, and before the first release the same minor version too.
    pub fn is_compatible_with(self, other: Self) -> bool {
        self.major == other.major && (self.major > 0 || self.minor == other.minor)
    }
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor)
    }
}

impl FromStr for ProtocolVersion {
    type Err = String;

    fn from_str(version: &str) -> Result<Self, Self::Err> {
        let invalid = || format!("invalid protocol version {version:?}; expected major.minor");
        let (major, minor) = version.split_once('.').ok_or_else(invalid)?;
        let number = |part: &str| {
            // Only plain digits, so "+1" and " 1" aren't versions.
            if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(invalid());
            }
            part.parse().map_err(|_| invalid())
        };
        Ok(Self {
            major: number(major)?,
            minor: number(minor)?,
        })
    }
}

impl Serialize for ProtocolVersion {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ProtocolVersion {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// The first request on a session connection: the protocol and Prism version the GUI speaks.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HelloRequest {
    pub protocol: ProtocolVersion,
    pub prism_version: String,
}

impl HelloRequest {
    /// This build's protocol, from a Prism of `prism_version`.
    pub fn current(prism_version: impl Into<String>) -> Self {
        Self {
            protocol: ProtocolVersion::CURRENT,
            prism_version: prism_version.into(),
        }
    }
}

/// prismd's answer to a compatible [`HelloRequest`]: the session is open, and its snapshot
/// follows.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Welcome {
    pub protocol: ProtocolVersion,
    pub prismd_version: String,
    /// Whether this session may change GPU tuning.
    pub can_tune: bool,
    /// Why it may not, when it may not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// How prismd decided, for the GUI to explain; `can_tune` alone decides what the session may
    /// do, so another authorization model can leave this out (ADR 0011).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization: Option<Authorization>,
}

/// How prismd's group model decided whether a session's user may tune.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Authorization {
    /// The user is root.
    Root {},
    /// prismd's config names the user as `admin_user`.
    AdminUser { user: String },
    /// The user is in prismd's admin group.
    AdminGroup { user: String, group: String },
    /// The user is neither, so the session is read-only.
    NotInAdminGroup { user: String, group: String },
}

/// prismd's answer to a [`HelloRequest`] whose protocol it can't speak; the connection closes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Incompatible {
    pub protocol: ProtocolVersion,
    pub prismd_version: String,
}

/// What prismd answers a request it doesn't know, such as one from a newer Prism.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Unsupported {
    pub message: String,
}

/// The daemon state a session starts from, sent once right after [`Welcome`]. After it, state
/// changes only through events, each carrying the full new value of its scope.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DaemonSnapshot {
    pub inventory: GpuInventoryResponse,
    pub store_health: TuningStoreHealth,
    pub tuning_states: Vec<GpuTuningState>,
    pub cooling_controls: Vec<GpuCoolingControl>,
    pub profiles: ProfilesState,
    pub games: GamesState,
    pub running_games: RunningGamesState,
    pub automatic_switching: AutomaticSwitchingStatus,
    /// The startup choices, with this boot's startup outcomes the caller hasn't dismissed.
    pub startup: StartupState,
    /// What prismd did this boot that the caller may see, oldest first.
    pub activity: Vec<ActivityEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonCommandResponse {
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GpuDaemonStatus {
    pub reachable: bool,
    pub managed_by_daemon: bool,
    pub socket_path: String,
    pub message: Option<String>,
    /// Unknown when the daemon can't be reached or predates the tuning store.
    #[serde(default)]
    pub tuning_store: Option<TuningStoreHealth>,
}

impl GpuDaemonStatus {
    pub fn connected(socket_path: impl Into<String>, tuning_store: TuningStoreHealth) -> Self {
        Self {
            reachable: true,
            managed_by_daemon: true,
            socket_path: socket_path.into(),
            message: None,
            tuning_store: Some(tuning_store),
        }
    }

    pub fn disconnected(socket_path: impl Into<String>, message: Option<String>) -> Self {
        Self {
            reachable: false,
            managed_by_daemon: false,
            socket_path: socket_path.into(),
            message,
            tuning_store: None,
        }
    }
}

/// Whether the daemon can use its persistent tuning store. An unhealthy store is preserved
/// untouched: startup and automatic writes are suspended, and manual live tuning keeps working.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum TuningStoreHealth {
    Healthy {},
    Unhealthy {
        problem: TuningStoreProblem,
        path: String,
        detail: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TuningStoreProblem {
    /// The document couldn't be read or parsed.
    Unreadable,
    /// The document was written by a newer schema version than this daemon understands.
    NewerVersion,
}

/// A desired tuning instruction: a fixed number, or the default resolved at application time.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProfileSetting {
    Value { value: i64 },
    // A struct variant so deny_unknown_fields also rejects `{ "kind": "default", "value": .. }`.
    Default {},
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProfileSettings {
    #[serde(
        default,
        deserialize_with = "included_cooling",
        skip_serializing_if = "Option::is_none"
    )]
    pub cooling: Option<prism_gpu::cooling::CoolingConfiguration>,
    #[serde(
        default,
        deserialize_with = "included_setting",
        skip_serializing_if = "Option::is_none"
    )]
    pub power_limit_watts: Option<ProfileSetting>,
    #[serde(
        default,
        deserialize_with = "included_setting",
        skip_serializing_if = "Option::is_none"
    )]
    pub graphics_offset_mhz: Option<ProfileSetting>,
    #[serde(
        default,
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
        *self.setting_mut(key) = Some(setting);
    }

    /// Leaves the setting unchanged again, as if it were omitted.
    pub fn clear(&mut self, key: ProfileSettingKey) {
        *self.setting_mut(key) = None;
    }

    fn setting_mut(&mut self, key: ProfileSettingKey) -> &mut Option<ProfileSetting> {
        match key {
            ProfileSettingKey::PowerLimitWatts => &mut self.power_limit_watts,
            ProfileSettingKey::GraphicsOffsetMhz => &mut self.graphics_offset_mhz,
            ProfileSettingKey::MemoryOffsetMhz => &mut self.memory_offset_mhz,
        }
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
) -> Result<Option<prism_gpu::cooling::CoolingConfiguration>, D::Error> {
    prism_gpu::cooling::CoolingConfiguration::deserialize(deserializer).map(Some)
}

// An included null is invalid; only an omitted instruction leaves a control alone.
fn included_setting<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<ProfileSetting>, D::Error> {
    ProfileSetting::deserialize(deserializer).map(Some)
}

/// A staged Apply: one manual write of a GPU's unapplied tuning changes, recorded as its last
/// applied tuning values without making any profile current.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApplyGpuProfileRequest {
    pub gpu_id: String,
    pub settings: ProfileSettings,
    /// The GPU's tuning-state revision the changes were made against. When the GPU's tuning state
    /// has changed since, the Apply is refused for review and nothing is written.
    pub expected_revision: String,
    /// The fans reviewed for the cooling change; when the GPU's fans differ, cooling isn't written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fan_indexes: Option<Vec<u32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileApplicationResponse {
    /// Automatic operations only. Manual applications never roll back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollback: Option<RollbackResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooling: Option<prism_gpu::cooling::CoolingApplication>,
    pub power: Option<TuningOutcome>,
    pub graphics: Option<TuningOutcome>,
    pub memory: Option<TuningOutcome>,
    // Present only when out-of-range numeric values are the sole obstacle; nothing was written.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corrections: Vec<ProfileRangeCorrection>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RollbackResult {
    pub status: RollbackStatus,
    pub power: Option<TuningOutcome>,
    pub graphics: Option<TuningOutcome>,
    pub memory: Option<TuningOutcome>,
    pub cooling: Option<prism_gpu::cooling::CoolingApplication>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cooling_restoration: Vec<prism_gpu::cooling::FanRestoration>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RollbackStatus {
    Restored,
    RestoreFailed,
    #[default]
    NotNeeded,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ProfileSettingKey {
    PowerLimitWatts,
    GraphicsOffsetMhz,
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
#[serde(rename_all = "snake_case")]
pub enum RangeBound {
    Minimum,
    Maximum,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged, deny_unknown_fields)]
pub enum ProfileRangeCorrection {
    Numeric(NumericRangeCorrection),
    Cooling {
        setting: CoolingSettingKey,
        saved_value: prism_gpu::cooling::CoolingConfiguration,
        proposed_value: prism_gpu::cooling::CoolingConfiguration,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CoolingSettingKey {
    Cooling,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NumericRangeCorrection {
    pub setting: ProfileSettingKey,
    pub saved_value: i64,
    pub proposed_value: i64,
    pub bound: RangeBound,
}

/// A GPU tuning profile, owned by the user who saved or imported it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GpuTuningProfile {
    /// Opaque, assigned by the daemon or kept from an imported profile.
    pub id: String,
    pub name: String,
    /// The owning GPU's UUID.
    pub gpu_id: String,
    pub settings: ProfileSettings,
}

/// The caller's GPU tuning profiles, with each GPU's startup and current profile by name, whoever
/// owns them.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProfilesState {
    /// The caller's own profiles, of every GPU, present or not.
    pub profiles: Vec<GpuTuningProfile>,
    /// Each GPU's host-wide startup profile, by GPU UUID.
    pub startup_profiles: BTreeMap<String, CurrentProfile>,
    /// Each GPU's current profile, by GPU UUID.
    pub current_profiles: BTreeMap<String, CurrentProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SaveProfileAsNewRequest {
    /// The owning GPU's UUID.
    pub gpu_id: String,
    pub name: String,
    /// The GPU's state revision the caller last saw.
    pub revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SaveToCurrentProfileRequest {
    /// The UUID of the GPU whose current profile is overwritten.
    pub gpu_id: String,
    /// The GPU's state revision the caller last saw.
    pub revision: String,
}

/// A GPU's current profile, visible by name to every user whoever owns it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CurrentProfile {
    pub id: String,
    pub name: String,
}

/// What a GPU's profile features rely on: its current profile, and the revision of its
/// reported and recorded tuning state that a save must echo.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GpuTuningState {
    /// The GPU's UUID.
    pub gpu_id: String,
    /// Changes whenever the GPU's reported or recorded tuning state changes.
    pub revision: String,
    /// The current tuning values the revision covers, so the app can tell whether they are the
    /// values it shows.
    pub reported: ReportedTuningValues,
    /// The settings whose current tuning value follows a request for the default the user last
    /// applied, which saving keeps as a default rather than a number.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub defaults: Vec<ProfileSettingKey>,
    pub current_profile: Option<CurrentProfile>,
}

/// Each profile setting's current tuning value, or null when it can't be read.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReportedTuningValues {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooling: Option<prism_gpu::cooling::CoolingConfiguration>,
    pub power_limit_watts: Option<i64>,
    pub graphics_offset_mhz: Option<i64>,
    pub memory_offset_mhz: Option<i64>,
}

/// Prism's control of a GPU's fans as one caller sees it: the cooling configuration prismd
/// applied, whether its curve control stopped and how the fans fell back, what became of the
/// curves when prismd restarted, and the caller's remembered fan curve. What a running curve
/// requests from moment to moment comes with telemetry instead.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GpuCoolingControl {
    /// The GPU's UUID.
    pub gpu_id: String,
    /// Changes whenever the applied configuration or Prism's control of the fans changes, never
    /// as a curve follows the temperature.
    pub revision: u64,
    /// The configuration Prism last applied, or none when it hasn't applied cooling to this GPU.
    /// A suspended curve stays applied until another configuration replaces it.
    pub applied: Option<prism_gpu::cooling::CoolingConfiguration>,
    /// Why curve control stopped, with each fan's fallback to automatic cooling, until an
    /// application brings the curve back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suspension: Option<prism_gpu::cooling::CurveSuspension>,
    /// Whether the applied curves resumed after prismd restarted within this host boot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<prism_gpu::cooling::CoolingRecovery>,
    /// The caller's last manually applied custom curve for the GPU, whatever is applied now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remembered_curve: Option<prism_gpu::cooling::CoolingConfiguration>,
}

impl GpuCoolingControl {
    /// The GPU's cooling control without the output of its running curves.
    pub fn of(gpu_id: impl Into<String>, control: prism_gpu::cooling::CoolingControl) -> Self {
        Self {
            gpu_id: gpu_id.into(),
            revision: control.revision,
            applied: control.applied,
            suspension: control.suspension,
            recovery: control.recovery,
            remembered_curve: control.remembered_curve,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProfileIdRequest {
    pub profile_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RenameProfileRequest {
    pub profile_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CorrectAndApplyProfileRequest {
    pub profile_id: String,
    /// The reviewed corrections; each must still match the profile's saved value.
    pub corrections: Vec<ProfileRangeCorrection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CorrectedProfileApplication {
    /// The profile as saved with the corrections, whatever the application's outcome.
    pub profile: GpuTuningProfile,
    pub application: ProfileApplicationResponse,
}

/// The host-wide choice of what the daemon puts into effect when the host starts.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
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
#[serde(deny_unknown_fields)]
pub struct SetStartupBehaviorRequest {
    pub behavior: StartupBehavior,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct StartupGpuRequest {
    pub gpu_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StartupProfile {
    pub gpu_id: String,
    /// Visible by name to all users; another user's values stay private.
    pub profile: CurrentProfile,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StartupOutcomeStatus {
    Applied,
    Failed,
    NotAttempted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StartupState {
    pub behavior: StartupBehavior,
    pub profiles: Vec<StartupProfile>,
    pub outcomes: Vec<StartupOutcome>,
    /// GPU UUIDs whose last applied tuning values miss a manual write that couldn't be recorded.
    #[serde(default)]
    pub possibly_stale: Vec<String>,
}

/// Something prismd did on its own or for a user during this boot, kept until the host restarts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ActivityEntry {
    /// Grows with each entry this boot, so a session can tell an entry it already has.
    pub id: u64,
    pub at_unix_ms: u64,
    /// The GPU's UUID.
    pub gpu_id: String,
    pub event: ActivityEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ActivityEvent {
    /// The caller applied a profile, or with none, their unapplied tuning changes.
    ProfileApplied {
        profile: Option<CurrentProfile>,
        application: ProfileApplicationResponse,
    },
    /// Automatic switching applied the profile the caller linked to their running game.
    GameProfileApplied {
        game: GameId,
        game_name: String,
        profile: CurrentProfile,
        application: ProfileApplicationResponse,
    },
    /// Automatic switching put the pre-game tuning values back after the caller's game ended.
    PreGameValuesRestored {
        application: ProfileApplicationResponse,
    },
    /// Startup behavior ran on the GPU when the host started.
    StartupRan { outcome: StartupOutcome },
}

/// How many Activity entries are kept, apart from startup outcomes.
pub const ACTIVITY_LIMIT: usize = 200;

/// Drops the oldest of `entries` beyond [`ACTIVITY_LIMIT`]. Startup outcomes stay for the whole
/// boot, one per GPU at most, so an outcome not yet dismissed can still be shown.
pub fn trim_activity<T>(entries: &mut Vec<T>, entry: impl Fn(&T) -> &ActivityEntry) {
    let startup = |item: &T| matches!(entry(item).event, ActivityEvent::StartupRan { .. });
    let mut excess = entries
        .iter()
        .filter(|item| !startup(item))
        .count()
        .saturating_sub(ACTIVITY_LIMIT);
    entries.retain(|item| {
        let drop = excess > 0 && !startup(item);
        excess -= usize::from(drop);
        !drop
    });
}

/// A fully installed Steam app from the caller's Steam libraries.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SteamApp {
    pub app_id: u32,
    /// From the app's manifest.
    pub name: String,
}

/// What identifies a game: a Steam app by its app ID, whether it runs natively or through
/// Proton.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GameId {
    Steam { app_id: u32 },
    Program { path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AddProgramRequest {
    pub path: String,
}

/// One of the caller's running processes, with a verified host location when recoverable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct RunningGame {
    pub game: GameId,
    pub pid: u32,
    /// Kernel clock ticks since boot, not wall-clock time.
    pub start_time_ticks: u64,
}

/// Unavailable is deliberately distinct from a successful scan with no games.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunningGamesState {
    Available { games: Vec<RunningGame> },
    Unavailable { reason: String },
}

/// The caller's game associations and automatic switching opt-in.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GamesState {
    pub automatic_switching: bool,
    pub games: Vec<GameEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AddSteamGameRequest {
    pub app_id: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SetGameProfileRequest {
    pub game: GameId,
    /// The GPU's UUID.
    pub gpu_id: String,
    /// None leaves the GPU unchanged while the game runs.
    #[serde(deserialize_with = "explicit_option")]
    pub profile_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GameRequest {
    pub game: GameId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SetAutomaticSwitchingRequest {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SetGpuSwitchingPausedRequest {
    pub gpu_id: String,
    pub paused: bool,
}

/// The caller's view of automatic switching: what holds it back overall, and each GPU that
/// automation drives or failed to drive. Other GPUs are left unchanged. It doesn't establish
/// which tuning values are in effect.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AutomaticSwitchingStatus {
    pub condition: SwitchingCondition,
    pub gpus: Vec<GpuSwitchingStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct GpuSwitchingStatus {
    /// The GPU's UUID.
    pub gpu_id: String,
    pub state: GpuSwitchingState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application: Option<ProfileApplicationResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum GpuSwitchingState {
    /// An explicit host-wide hold on this GPU, retained until Resume.
    Paused {},
    /// The caller's running game drives the GPU with its linked profile.
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
#[serde(deny_unknown_fields)]
pub struct SetEditHoldsRequest {
    pub gpu_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum DaemonRequest {
    /// The handshake, which a session connection starts with.
    Hello(HelloRequest),
    SetEditHolds(SetEditHoldsRequest),
    /// Opts the session in to [`DaemonEvent::TelemetrySampled`].
    SubscribeTelemetry,
    GetInventory,
    GetDaemonStatus,
    /// Applies a GPU's staged changes.
    ApplyProfile(ApplyGpuProfileRequest),
    /// Checks a draft shared fan curve without changing any GPU setting.
    PreviewFanCurve(PreviewFanCurveRequest),
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
    Welcome(Welcome),
    Incompatible(Incompatible),
    Unsupported(Unsupported),
    Inventory(GpuInventoryResponse),
    DaemonStatus(GpuDaemonStatus),
    Command(DaemonCommandResponse),
    FanCurvePreview(FanCurvePreview),
    ProfileApplication(ProfileApplicationResponse),
    StartupState(StartupState),
    Profiles(Vec<GpuTuningProfile>),
    Profile(GpuTuningProfile),
    TuningStates(Vec<GpuTuningState>),
    CorrectedProfileApplication(CorrectedProfileApplication),
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
#[serde(deny_unknown_fields)]
pub struct SessionRequest {
    pub id: u64,
    pub request: DaemonRequest,
}

// Messages are short-lived frames, so their size doesn't warrant boxing every response.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SessionMessage {
    Response {
        id: u64,
        response: DaemonResponse,
    },
    /// Sent once, right after [`Welcome`].
    Snapshot {
        snapshot: DaemonSnapshot,
    },
    Event {
        event: DaemonEvent,
    },
}

// Events are short-lived frames, so their size doesn't warrant boxing every payload.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "type",
    content = "payload",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum DaemonEvent {
    StoreHealthChanged(TuningStoreHealth),
    /// The GPUs prismd can inspect changed, such as when one arrived or departed.
    InventoryChanged(GpuInventoryResponse),
    /// Every GPU read once on prismd's 1 Hz tick, with Prism's control of each GPU's fans and
    /// what its curve requests now; only for sessions that subscribed to telemetry.
    TelemetrySampled(GpuPerformanceResponse),
    TuningStateChanged(GpuTuningState),
    /// Prism's control of a GPU's fans changed, such as when cooling was applied, a curve
    /// faulted or yielded to another tool, or the caller's remembered curve changed.
    CoolingControlChanged(GpuCoolingControl),
    /// The caller's profiles changed, or a GPU's startup or current profile, or its name.
    ProfilesChanged(ProfilesState),
    /// The caller's games, their associations or the caller's automatic switching opt-in changed.
    GamesChanged(GamesState),
    RunningGamesChanged(RunningGamesState),
    AutomaticSwitchingChanged(AutomaticSwitchingStatus),
    /// The startup choices changed, or the caller's startup outcomes, such as when one finished or
    /// was dismissed.
    StartupStateChanged(StartupState),
    /// prismd did something the caller may see.
    ActivityRecorded(ActivityEntry),
}

#[cfg(test)]
mod daemon_status_tests {
    use serde_json::json;

    use super::{GpuDaemonStatus, TuningStoreHealth, TuningStoreProblem};

    #[test]
    fn inventory_leaves_daemon_status_out_of_each_gpu() {
        // Who can apply tuning is a fact about the install, not the GPU.
        let control = serde_json::to_value(prism_gpu::GpuControlInfo::default()).unwrap();
        assert_eq!(control.get("managed_by_daemon"), None);
        assert_eq!(control.get("daemon_reachable"), None);
    }

    #[test]
    fn daemon_status_disconnected_preserves_socket_path() {
        let status = GpuDaemonStatus::disconnected("/run/prism/prismd.sock", None);

        assert!(!status.reachable);
        assert_eq!(status.socket_path, "/run/prism/prismd.sock");
        assert_eq!(
            serde_json::to_value(status).unwrap()["tuning_store"],
            json!(null)
        );
    }

    #[test]
    fn daemon_status_round_trips_the_shared_unhealthy_store_contract() {
        let contract: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/contracts/daemon-status-unhealthy-store.json"
        ))
        .unwrap();

        let status: GpuDaemonStatus = serde_json::from_value(contract.clone()).unwrap();

        assert_eq!(
            status.tuning_store,
            Some(TuningStoreHealth::Unhealthy {
                problem: TuningStoreProblem::NewerVersion,
                path: "/var/lib/prism/tuning-state.json".into(),
                detail:
                    "it was written by schema version 2, and this daemon understands up to version 1"
                        .into(),
            })
        );
        assert_eq!(serde_json::to_value(status).unwrap(), contract);
    }

    #[test]
    fn daemon_status_round_trips_a_healthy_store_and_tolerates_a_daemon_without_one() {
        let healthy = json!({
            "reachable": true,
            "managed_by_daemon": true,
            "socket_path": "/run/prism/prismd.sock",
            "message": null,
            "tuning_store": { "status": "healthy" }
        });
        let status: GpuDaemonStatus = serde_json::from_value(healthy.clone()).unwrap();
        assert_eq!(status.tuning_store, Some(TuningStoreHealth::Healthy {}));
        assert_eq!(serde_json::to_value(status).unwrap(), healthy);

        let mut older = healthy;
        older.as_object_mut().unwrap().remove("tuning_store");
        let status: GpuDaemonStatus = serde_json::from_value(older).unwrap();
        assert_eq!(status.tuning_store, None);
    }

    #[test]
    fn daemon_status_rejects_an_unknown_store_problem() {
        let malformed = json!({
            "reachable": true,
            "managed_by_daemon": true,
            "socket_path": "/run/prism/prismd.sock",
            "message": null,
            "tuning_store": { "status": "unhealthy", "problem": "melted", "path": "/x", "detail": "" }
        });
        assert!(serde_json::from_value::<GpuDaemonStatus>(malformed).is_err());
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;
    use crate::framing::{read_frame, write_frame};

    fn version(major: u32, minor: u32) -> ProtocolVersion {
        ProtocolVersion { major, minor }
    }

    #[test]
    fn protocol_versions_match_on_major_and_before_the_first_release_on_minor_too() {
        assert!(version(0, 1).is_compatible_with(version(0, 1)));
        assert!(!version(0, 1).is_compatible_with(version(0, 2)));
        assert!(!version(0, 2).is_compatible_with(version(0, 1)));
        // From 1.0 on, minor versions only add, so either side's minor version works.
        assert!(version(1, 0).is_compatible_with(version(1, 3)));
        assert!(version(1, 3).is_compatible_with(version(1, 0)));
        assert!(!version(1, 0).is_compatible_with(version(2, 0)));
        assert!(!version(1, 0).is_compatible_with(version(0, 1)));
    }

    #[test]
    fn protocol_versions_are_major_dot_minor_on_the_wire() {
        assert_eq!(serde_json::to_value(version(0, 1)).unwrap(), "0.1");
        assert_eq!(
            serde_json::from_value::<ProtocolVersion>(serde_json::json!("12.30")).unwrap(),
            version(12, 30)
        );
        for malformed in ["1", "1.2.3", "a.b", ".1", "1.", "+1.0", " 1.0", ""] {
            assert!(
                serde_json::from_value::<ProtocolVersion>(serde_json::json!(malformed)).is_err(),
                "{malformed:?}"
            );
        }
        assert!(serde_json::from_value::<ProtocolVersion>(serde_json::json!(1.0)).is_err());
    }

    #[test]
    fn the_handshake_round_trips_the_shared_contract() {
        let handshake: serde_json::Value =
            serde_json::from_str(include_str!("../tests/contracts/session-handshake.json"))
                .unwrap();
        let hello: SessionRequest = serde_json::from_value(handshake["hello"].clone()).unwrap();
        assert_eq!(
            hello.request,
            DaemonRequest::Hello(HelloRequest::current("0.1.0"))
        );
        assert_eq!(serde_json::to_value(hello).unwrap(), handshake["hello"]);
        for answer in [
            "welcome",
            "read_only_welcome",
            "no_permission",
            "incompatible",
            "unsupported",
        ] {
            let typed: SessionMessage = serde_json::from_value(handshake[answer].clone()).unwrap();
            assert_eq!(
                serde_json::to_value(typed).unwrap(),
                handshake[answer],
                "{answer}"
            );
        }
        let mut extra = handshake["hello"].clone();
        extra["request"]["payload"]["uid"] = serde_json::json!(0);
        assert!(serde_json::from_value::<SessionRequest>(extra).is_err());
    }

    #[test]
    fn the_snapshot_round_trips_the_shared_contract() {
        let snapshot: serde_json::Value =
            serde_json::from_str(include_str!("../tests/contracts/daemon-snapshot.json")).unwrap();
        let typed: SessionMessage = serde_json::from_value(snapshot.clone()).unwrap();
        assert!(matches!(typed, SessionMessage::Snapshot { .. }));
        assert_eq!(serde_json::to_value(typed).unwrap(), snapshot);
    }

    #[test]
    fn sampled_telemetry_and_inventory_changes_round_trip_as_pushed_events() {
        let telemetry: serde_json::Value =
            serde_json::from_str(include_str!("../tests/contracts/tuning-observations.json"))
                .unwrap();
        let snapshot: serde_json::Value =
            serde_json::from_str(include_str!("../tests/contracts/daemon-snapshot.json")).unwrap();
        for event in [
            serde_json::json!({ "type": "telemetry_sampled", "payload": telemetry }),
            serde_json::json!({ "type": "inventory_changed", "payload": snapshot["snapshot"]["inventory"] }),
        ] {
            let message = serde_json::json!({ "type": "event", "event": event });
            let typed: SessionMessage = serde_json::from_value(message.clone()).unwrap();
            assert_eq!(serde_json::to_value(typed).unwrap(), message);
        }
        let request = serde_json::json!({ "type": "subscribe_telemetry" });
        let typed: DaemonRequest = serde_json::from_value(request.clone()).unwrap();
        assert_eq!(typed, DaemonRequest::SubscribeTelemetry);
        assert_eq!(serde_json::to_value(typed).unwrap(), request);
    }

    #[test]
    fn the_removed_performance_read_is_no_longer_a_request() {
        assert!(
            serde_json::from_value::<DaemonRequest>(
                serde_json::json!({ "type": "get_performance" })
            )
            .is_err()
        );
    }

    #[test]
    fn session_frames_round_trip_and_keep_response_ids_separate_from_events() {
        let request = SessionRequest {
            id: 42,
            request: DaemonRequest::GetDaemonStatus,
        };
        let event: SessionMessage =
            serde_json::from_str(include_str!("../tests/contracts/session-store-health.json"))
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
                "type": "event", "event": {"type": "store_health_changed", "payload": {"status": "healthy"}}
            })
        );
        assert!(read_frame::<SessionMessage>(&mut reader).unwrap().is_none());
    }

    #[test]
    fn startup_state_round_trips_the_shared_contract_and_explicit_clearing() {
        let state: serde_json::Value =
            serde_json::from_str(include_str!("../tests/contracts/startup-state.json")).unwrap();
        let typed: StartupState = serde_json::from_value(state.clone()).unwrap();
        assert_eq!(
            serde_json::to_value(DaemonResponse::StartupState(typed)).unwrap(),
            serde_json::json!({ "type": "startup_state", "payload": state })
        );
        for request in [
            serde_json::json!({ "type": "set_startup_profile", "payload": { "gpu_id": "GPU-123", "profile_id": null } }),
            serde_json::json!({ "type": "set_startup_profile", "payload": { "gpu_id": "GPU-123", "profile_id": "quiet" } }),
            serde_json::json!({ "type": "set_startup_behavior", "payload": { "behavior": "startup_profiles" } }),
            serde_json::json!({ "type": "dismiss_startup_outcome", "payload": { "gpu_id": "GPU-123" } }),
        ] {
            let typed: DaemonRequest = serde_json::from_value(request.clone()).unwrap();
            assert_eq!(serde_json::to_value(typed).unwrap(), request);
        }
    }

    #[test]
    fn activity_and_startup_state_round_trip_as_pushed_events() {
        let entries: serde_json::Value =
            serde_json::from_str(include_str!("../tests/contracts/activity-entries.json")).unwrap();
        let startup: serde_json::Value =
            serde_json::from_str(include_str!("../tests/contracts/startup-state.json")).unwrap();
        let events = entries
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| serde_json::json!({ "type": "activity_recorded", "payload": entry }))
            .chain([serde_json::json!({ "type": "startup_state_changed", "payload": startup })]);
        for event in events {
            let message = serde_json::json!({ "type": "event", "event": event });
            let typed: SessionMessage = serde_json::from_value(message.clone()).unwrap();
            assert_eq!(serde_json::to_value(typed).unwrap(), message);
        }
    }

    #[test]
    fn startup_requests_reject_missing_choices_and_client_ownership() {
        for malformed in [
            serde_json::json!({ "type": "set_startup_profile", "payload": { "gpu_id": "GPU-123" } }),
            serde_json::json!({ "type": "set_startup_profile", "payload": { "gpu_id": "GPU-123", "profile_id": 7 } }),
            serde_json::json!({ "type": "set_startup_profile", "payload": { "gpu_id": "GPU-123", "profile_id": null, "uid": 0 } }),
            serde_json::json!({ "type": "set_startup_behavior", "payload": { "behavior": "always" } }),
            serde_json::json!({ "type": "dismiss_startup_outcome", "payload": { "gpu_id": "GPU-123", "username": "alice" } }),
        ] {
            assert!(
                serde_json::from_value::<DaemonRequest>(malformed.clone()).is_err(),
                "{malformed}"
            );
        }
    }

    #[test]
    fn profile_payloads_round_trip_the_shared_contracts() {
        let profiles: serde_json::Value =
            serde_json::from_str(include_str!("../tests/contracts/gpu-tuning-profiles.json"))
                .unwrap();
        let typed: Vec<GpuTuningProfile> = serde_json::from_value(profiles.clone()).unwrap();
        assert_eq!(serde_json::to_value(&typed).unwrap(), profiles);
        let corrected: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/contracts/corrected-profile-application.json"
        ))
        .unwrap();
        let typed: CorrectedProfileApplication = serde_json::from_value(corrected.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), corrected);
    }

    #[test]
    fn tuning_states_round_trip_the_shared_contract_as_a_response_and_as_pushed_events() {
        let states: serde_json::Value =
            serde_json::from_str(include_str!("../tests/contracts/gpu-tuning-states.json"))
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
            "type": "event", "event": { "type": "tuning_state_changed", "payload": states[0] }
        });
        assert_eq!(serde_json::to_value(&event).unwrap(), json);
        assert_eq!(
            serde_json::from_value::<SessionMessage>(json).unwrap(),
            event
        );
    }

    #[test]
    fn cooling_control_round_trips_the_shared_contract_as_a_pushed_event() {
        let control: serde_json::Value =
            serde_json::from_str(include_str!("../tests/contracts/cooling-control.json")).unwrap();
        let event = serde_json::json!({
            "type": "event", "event": { "type": "cooling_control_changed", "payload": control }
        });
        let typed: SessionMessage = serde_json::from_value(event.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), event);
        // What a running curve requests now comes with telemetry, not with its control.
        let mut with_output = control;
        with_output["requested"] = serde_json::json!([]);
        assert!(serde_json::from_value::<GpuCoolingControl>(with_output).is_err());
    }

    #[test]
    fn profiles_round_trip_the_shared_contract_as_a_pushed_event() {
        let profiles: serde_json::Value =
            serde_json::from_str(include_str!("../tests/contracts/profiles-state.json")).unwrap();
        let event = serde_json::json!({
            "type": "event", "event": { "type": "profiles_changed", "payload": profiles }
        });
        let typed: SessionMessage = serde_json::from_value(event.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), event);
        // Another user's profile values stay private: only names cross for startup and current.
        let mut with_settings = profiles;
        with_settings["current_profiles"]["GPU-5a1b0000-0000-4000-8000-000000000001"]["settings"] =
            serde_json::json!({});
        assert!(serde_json::from_value::<ProfilesState>(with_settings).is_err());
    }

    #[test]
    fn profile_management_requests_round_trip_and_reject_client_supplied_ownership() {
        for request in [
            serde_json::json!({ "type": "rename_profile", "payload": { "profile_id": "quiet", "name": "Silent" } }),
            serde_json::json!({ "type": "delete_profile", "payload": { "profile_id": "quiet" } }),
        ] {
            let typed: DaemonRequest = serde_json::from_value(request.clone()).unwrap();
            assert_eq!(serde_json::to_value(typed).unwrap(), request);
            let mut malformed = request;
            malformed["payload"]["uid"] = serde_json::json!(0);
            assert!(serde_json::from_value::<DaemonRequest>(malformed).is_err());
        }
        for malformed in [
            serde_json::json!({ "type": "rename_profile", "payload": { "profile_id": "quiet" } }),
            serde_json::json!({ "type": "rename_profile", "payload": { "profile_id": "quiet", "name": 7 } }),
            serde_json::json!({ "type": "delete_profile", "payload": { "profile_id": 7 } }),
        ] {
            assert!(serde_json::from_value::<DaemonRequest>(malformed).is_err());
        }
    }

    #[test]
    fn saves_carry_the_revision_the_caller_saw() {
        let request: DaemonRequest = serde_json::from_value(serde_json::json!({
            "type": "save_to_current_profile",
            "payload": { "gpu_id": "GPU-123", "revision": "4f1c2b9d0e8a7c35" }
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
            serde_json::json!({"type": "save_to_current_profile", "payload": {"gpu_id": "GPU-123"}}),
            serde_json::json!({"type": "save_to_current_profile", "payload": {"gpu_id": "GPU-123", "revision": 7}}),
            serde_json::json!({"type": "save_to_current_profile", "payload": {"gpu_id": "GPU-123", "revision": "r", "profile_id": "quiet"}}),
            serde_json::json!({"type": "save_profile_as_new", "payload": {"gpu_id": "GPU-123", "name": "Quiet"}}),
            serde_json::json!({"type": "get_tuning_states", "payload": {"gpu_id": "GPU-123"}}),
        ] {
            assert!(
                serde_json::from_value::<DaemonRequest>(malformed.clone()).is_err(),
                "{malformed}"
            );
        }
        let malformed = serde_json::json!({"type": "event", "event": {"type": "tuning_state_changed", "payload": {"gpu_id": "GPU-123", "revision": "r", "current_profile": null, "uid": 0}}});
        assert!(serde_json::from_value::<SessionMessage>(malformed).is_err());
    }

    #[test]
    fn profile_requests_reject_malformed_payloads() {
        for malformed in [
            serde_json::json!({"type": "save_profile_as_new", "payload": {"gpu_id": "GPU-123"}}),
            serde_json::json!({"type": "save_profile_as_new", "payload": {"gpu_id": "GPU-123", "name": "Quiet", "settings": {}}}),
            serde_json::json!({"type": "apply_profile_by_id", "payload": {"profile_id": 7}}),
            serde_json::json!({"type": "correct_and_apply_profile", "payload": {"profile_id": "quiet", "corrections": [{"setting": "fan_speed", "saved_value": 1, "proposed_value": 0, "bound": "maximum"}]}}),
        ] {
            assert!(
                serde_json::from_value::<DaemonRequest>(malformed.clone()).is_err(),
                "{malformed}"
            );
        }
    }

    #[test]
    fn games_round_trip_the_shared_contract_as_a_pushed_event() {
        let games: serde_json::Value =
            serde_json::from_str(include_str!("../tests/contracts/games-state.json")).unwrap();
        let event = serde_json::json!({
            "type": "event", "event": { "type": "games_changed", "payload": games }
        });
        let typed: SessionMessage = serde_json::from_value(event.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), event);
    }

    #[test]
    fn game_payloads_round_trip_the_shared_contract_and_reject_client_ownership() {
        let state: serde_json::Value =
            serde_json::from_str(include_str!("../tests/contracts/games-state.json")).unwrap();
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
            serde_json::json!({ "type": "steam_apps", "payload": [{ "app_id": 570, "name": "Dota 2" }] })
        );
        for request in [
            serde_json::json!({ "type": "list_steam_apps" }),
            serde_json::json!({ "type": "list_running_programs" }),
            serde_json::json!({ "type": "add_program", "payload": { "path": "/games/game.exe" } }),
            serde_json::json!({ "type": "set_game_profile", "payload": { "game": { "kind": "program", "path": "/games/game.exe" }, "gpu_id": "GPU-123", "profile_id": "quiet" } }),
            serde_json::json!({ "type": "get_games" }),
            serde_json::json!({ "type": "add_steam_game", "payload": { "app_id": 570 } }),
            serde_json::json!({ "type": "set_game_profile", "payload": { "game": { "kind": "steam", "app_id": 570 }, "gpu_id": "GPU-123", "profile_id": "quiet" } }),
            serde_json::json!({ "type": "set_game_profile", "payload": { "game": { "kind": "steam", "app_id": 570 }, "gpu_id": "GPU-123", "profile_id": null } }),
            serde_json::json!({ "type": "remove_game", "payload": { "game": { "kind": "steam", "app_id": 570 } } }),
            serde_json::json!({ "type": "set_automatic_switching", "payload": { "enabled": true } }),
            serde_json::json!({ "type": "set_gpu_switching_paused", "payload": { "gpu_id": "GPU-123", "paused": true } }),
            serde_json::json!({ "type": "set_gpu_switching_paused", "payload": { "gpu_id": "GPU-123", "paused": false } }),
        ] {
            let typed: DaemonRequest = serde_json::from_value(request.clone()).unwrap();
            assert_eq!(serde_json::to_value(typed).unwrap(), request);
        }
        for malformed in [
            serde_json::json!({ "type": "add_steam_game", "payload": { "app_id": "570" } }),
            serde_json::json!({ "type": "add_steam_game", "payload": { "app_id": -1 } }),
            serde_json::json!({ "type": "add_steam_game", "payload": { "app_id": 570, "name": "Dota 2" } }),
            serde_json::json!({ "type": "set_game_profile", "payload": { "game": { "kind": "steam", "app_id": 570 }, "gpu_id": "GPU-123" } }),
            serde_json::json!({ "type": "set_game_profile", "payload": { "game": { "kind": "program", "app_id": 570 }, "gpu_id": "GPU-123", "profile_id": null } }),
            serde_json::json!({ "type": "set_game_profile", "payload": { "game": { "kind": "steam", "app_id": 570, "uid": 0 }, "gpu_id": "GPU-123", "profile_id": null } }),
            serde_json::json!({ "type": "remove_game", "payload": { "game": { "kind": "steam" } } }),
            serde_json::json!({ "type": "set_automatic_switching", "payload": { "enabled": "yes" } }),
            serde_json::json!({ "type": "set_gpu_switching_paused", "payload": { "gpu_id": "GPU-123", "paused": "yes" } }),
            serde_json::json!({ "type": "set_gpu_switching_paused", "payload": { "gpu_id": "GPU-123" } }),
            serde_json::json!({ "type": "set_gpu_switching_paused", "payload": { "gpu_id": "GPU-123", "paused": true, "uid": 0 } }),
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
            "../tests/contracts/automatic-switching-status.json"
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
            serde_json::json!({ "condition": { "status": "off" }, "gpus": [{ "gpu_id": "GPU-123", "state": { "status": "another_user", "uid": 1000 } }] }),
            serde_json::json!({ "condition": { "status": "off" }, "gpus": [{ "gpu_id": "GPU-123", "state": { "status": "active", "game_name": "Dota 2" } }] }),
        ] {
            assert!(
                serde_json::from_value::<AutomaticSwitchingStatus>(malformed.clone()).is_err(),
                "{malformed}"
            );
        }
    }

    #[test]
    fn running_games_round_trip_the_shared_contract_as_responses_and_session_events() {
        let states: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../tests/contracts/running-games.json")).unwrap();
        for state in states {
            let typed: RunningGamesState = serde_json::from_value(state.clone()).unwrap();
            assert_eq!(
                serde_json::to_value(DaemonResponse::RunningGames(typed)).unwrap(),
                serde_json::json!({ "type": "running_games", "payload": state })
            );
            let event = serde_json::json!({
                "type": "event", "event": { "type": "running_games_changed", "payload": state }
            });
            let typed: SessionMessage = serde_json::from_value(event.clone()).unwrap();
            assert_eq!(serde_json::to_value(typed).unwrap(), event);
        }
    }

    #[test]
    fn an_apply_carries_the_revision_it_was_made_against_and_the_reviewed_fans() {
        let request = serde_json::json!({ "type": "apply_profile", "payload": {
            "gpu_id": "GPU-123",
            "settings": { "power_limit_watts": { "kind": "value", "value": 450 } },
            "expected_revision": "4f1c2b9d0e8a7c35",
            "fan_indexes": [0, 1]
        } });
        let typed: DaemonRequest = serde_json::from_value(request.clone()).unwrap();
        assert_eq!(
            typed,
            DaemonRequest::ApplyProfile(ApplyGpuProfileRequest {
                gpu_id: "GPU-123".into(),
                settings: ProfileSettings {
                    power_limit_watts: Some(ProfileSetting::Value { value: 450 }),
                    ..ProfileSettings::default()
                },
                expected_revision: "4f1c2b9d0e8a7c35".into(),
                fan_indexes: Some(vec![0, 1]),
            })
        );
        assert_eq!(serde_json::to_value(typed).unwrap(), request);
        let mut unreviewed = request;
        unreviewed["payload"]
            .as_object_mut()
            .unwrap()
            .remove("expected_revision");
        assert!(serde_json::from_value::<DaemonRequest>(unreviewed).is_err());
    }

    #[test]
    fn the_per_control_live_requests_are_no_longer_requests() {
        for removed in [
            serde_json::json!({ "type": "set_power_limit", "payload": { "gpu_id": "GPU-123", "limit_watts": 300 } }),
            serde_json::json!({ "type": "reset_power_limit", "payload": { "gpu_id": "GPU-123" } }),
            serde_json::json!({ "type": "set_fan_speed", "payload": { "gpu_id": "GPU-123", "fan_index": 0, "speed_percent": 50 } }),
            serde_json::json!({ "type": "set_fan_mode_auto", "payload": { "gpu_id": "GPU-123", "fan_index": 0 } }),
            serde_json::json!({ "type": "set_clock_offsets", "payload": { "gpu_id": "GPU-123", "graphics_offset_mhz": 100 } }),
            serde_json::json!({ "type": "reset_clock_offsets", "payload": { "gpu_id": "GPU-123" } }),
            serde_json::json!({ "type": "apply_cooling", "payload": { "gpu_id": "GPU-123", "configuration": { "arrangement": "shared", "setting": { "mode": "automatic" } } } }),
        ] {
            assert!(
                serde_json::from_value::<DaemonRequest>(removed.clone()).is_err(),
                "{removed}"
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
        let malformed = serde_json::json!({"type": "event", "event": {"type": "store_health_changed", "payload": {"status": "healthy", "problem": "unreadable"}}});
        assert!(serde_json::from_value::<SessionMessage>(malformed).is_err());
    }
}
