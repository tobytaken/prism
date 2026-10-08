//! Cooling configurations: what Prism applies to a GPU's fans, kept distinct from the speed a
//! curve requests at any moment and from what the driver reports.

use serde::{Deserialize, Serialize};

use crate::fan_curve::FanCurvePoint;
use crate::protocol::TuningOutcome;

/// One fan's control mode: the GPU's default automatic cooling, a fixed requested speed, or a
/// fan curve that the daemon follows as the GPU's temperature changes.
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyCoolingRequest {
    /// The fan inventory reviewed by the caller; a changed inventory requires a new review.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fan_indexes: Option<Vec<u32>>,
    /// The GPU's UUID.
    pub gpu_id: String,
    pub configuration: CoolingConfiguration,
}

/// What an application did: succeeded only when every targeted fan took its setting.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CoolingApplication {
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
    /// The configuration Prism last applied, or none when it hasn't applied cooling to this GPU.
    /// A suspended curve stays applied until another configuration replaces it.
    pub applied: Option<CoolingConfiguration>,
    /// Each fan following a curve, with the speed the curve requests now.
    pub requested: Vec<FanRequest>,
    /// The GPU temperature the curve last followed.
    pub curve_temperature_c: Option<i64>,
    /// Why curve control stopped, until an application brings the curve back.
    pub suspension: Option<CurveSuspension>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FanRequest {
    pub fan_index: u32,
    pub speed_percent: i64,
}

/// Curve control stopped after a fault. The curve is kept, and only an explicit application
/// resumes it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CurveSuspension {
    pub reason: String,
    /// Each curve fan's attempt to return to the GPU's default automatic cooling.
    pub restoration: Vec<FanRestoration>,
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
