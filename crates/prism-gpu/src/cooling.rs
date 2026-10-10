//! Cooling configurations: what Prism applies to a GPU's fans, kept distinct from the speed a
//! curve requests at any moment and from what the driver reports.

use serde::{Deserialize, Serialize};

use crate::TuningOutcome;
use crate::fan_curve::FanCurvePoint;

/// One fan's control mode: the GPU's default automatic cooling, a fixed requested speed, or a
/// fan curve that Prism follows as the GPU's temperature changes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "camelCase", deny_unknown_fields)]
pub enum FanSetting {
    // Struct variants, so deny_unknown_fields also rejects extra fields.
    Automatic {},
    #[serde(rename_all = "camelCase")]
    Fixed {
        speed_percent: i64,
    },
    Curve {
        points: Vec<FanCurvePoint>,
    },
}

/// A fan's own setting in a per-fan configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FanConfiguration {
    pub fan_index: u32,
    pub setting: FanSetting,
}

/// A GPU's cooling configuration: one setting shared by every fan, or a setting per fan. Fans a
/// per-fan configuration omits are left as they are.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "arrangement", rename_all = "camelCase", deny_unknown_fields)]
pub enum CoolingConfiguration {
    Shared { setting: FanSetting },
    PerFan { fans: Vec<FanConfiguration> },
}

impl CoolingConfiguration {
    pub fn contains_curve(&self) -> bool {
        match self {
            Self::Shared { setting } => matches!(setting, FanSetting::Curve { .. }),
            Self::PerFan { fans } => fans
                .iter()
                .any(|fan| matches!(fan.setting, FanSetting::Curve { .. })),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyCoolingRequest {
    /// The fan inventory reviewed by the caller; a changed inventory requires a new review.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fan_indexes: Option<Vec<u32>>,
    /// The GPU's UUID.
    pub gpu_id: String,
    pub configuration: CoolingConfiguration,
    /// Draft revision, checked on the GPU queue before any write. Omitted by legacy callers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
}

/// What an application did: succeeded only when every targeted fan took its setting.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CoolingApplication {
    /// Revision after this attempt, captured before releasing the GPU queue.
    pub revision: u64,
    pub outcome: TuningOutcome,
    pub fans: Vec<FanOutcome>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FanOutcome {
    pub fan_index: u32,
    pub outcome: TuningOutcome,
}

/// Prism's control of a GPU's fans. The applied configuration, the speeds a curve requests now,
/// and the driver's reported fan modes and speeds are separate things: a curve runs through the
/// driver's manual mode without being a fixed speed.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CoolingControl {
    /// The caller's last successful manual curve configuration, independent of applied mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remembered_curve: Option<CoolingConfiguration>,
    /// Changes only when configuration or control ownership changes, never on curve ticks.
    #[serde(default)]
    pub revision: u64,
    /// The configuration Prism last applied, or none when it hasn't applied cooling to this GPU.
    /// A suspended curve stays applied until another configuration replaces it.
    pub applied: Option<CoolingConfiguration>,
    /// Each fan following a curve, with the speed the curve requests now.
    pub requested: Vec<FanRequest>,
    /// The GPU temperature the curve last followed.
    pub curve_temperature_c: Option<i64>,
    /// Why curve control stopped, until an application brings the curve back.
    pub suspension: Option<CurveSuspension>,
    /// What became of the applied curves when curve control restarted within this host boot,
    /// until an application or a later suspension replaces it.
    #[serde(default)]
    pub recovery: Option<CoolingRecovery>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FanRequest {
    pub fan_index: u32,
    pub speed_percent: i64,
}

/// Curve control stopped after a fault or an observable competing control change. The curve is kept, and only an explicit application
/// resumes it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CurveSuspension {
    #[serde(default)]
    pub kind: SuspensionKind,
    pub reason: String,
    /// Each curve fan's attempt to return to the GPU's default automatic cooling.
    pub restoration: Vec<FanRestoration>,
}

/// Restarted curve control's attempt to resume the curves it was following when it stopped in
/// the same host boot. A curve that couldn't resume is suspended and waits for a retry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CoolingRecovery {
    pub status: RecoveryStatus,
    /// Each interrupted curve fan's resumption.
    pub fans: Vec<FanOutcome>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RecoveryStatus {
    /// Fresh readings matched the GPU, its fans and the curve, and every fan follows it again.
    Resumed,
    /// The curve didn't resume; its fans were returned to automatic cooling where possible.
    RetryRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FanRestoration {
    pub fan_index: u32,
    pub status: RestorationStatus,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RestorationStatus {
    /// The driver accepted the request and reports automatic control.
    Restored,
    /// The driver accepted the request, but doesn't report automatic control.
    Unverified,
    /// The driver rejected the request, or it never ran.
    Failed,
}

/// Provenance for consumers such as automatic switching: yielding is not fault recovery.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SuspensionKind {
    #[default]
    Fault,
    ExternalControl,
}
