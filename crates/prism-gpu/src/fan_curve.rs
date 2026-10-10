//! Fan curves: a user-defined relationship between GPU temperature and requested fan speed.
//!
//! One shared curve controls every fan on a GPU, so it is judged against every fan's limits.
//! Custom curves need known speed limits and a trustworthy maximum operating temperature; no
//! limit is ever guessed.

use serde::{Deserialize, Serialize};

use crate::CoolingInfo;

pub const MIN_POINTS: usize = 2;
pub const MAX_POINTS: usize = 16;
/// Every curve must reach maximum speed this far below the maximum operating temperature.
pub const FULL_SPEED_MARGIN_C: i64 = 5;

/// A requested fan speed at a GPU temperature, in whole degrees Celsius and whole percent.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FanCurvePoint {
    pub temperature_c: i64,
    pub speed_percent: i64,
}

impl FanCurvePoint {
    pub fn new(temperature_c: i64, speed_percent: i64) -> Self {
        Self {
            temperature_c,
            speed_percent,
        }
    }
}

/// Whether one shared curve can control every fan on a GPU, and within which limits.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum FanCurveSupport {
    Available {
        /// The highest temperature a point can use.
        max_operating_temperature_c: i64,
        /// The temperature by which the curve must request the maximum speed.
        full_speed_temperature_c: i64,
        /// The lowest speed every fan supports; zero only when every fan supports it.
        min_speed_percent: i64,
        max_speed_percent: i64,
        /// The starter fan curve: editable, and not the GPU's own firmware curve.
        starter: Vec<FanCurvePoint>,
    },
    Unavailable {
        reason: String,
    },
}

/// A reason a curve can't be used; `point` is the zero-based index of the point at fault.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FanCurveProblem {
    pub point: Option<usize>,
    pub message: String,
}

impl FanCurveProblem {
    fn curve(message: impl Into<String>) -> Self {
        Self {
            point: None,
            message: message.into(),
        }
    }

    fn point(index: usize, message: impl Into<String>) -> Self {
        Self {
            point: Some(index),
            message: format!("Point {}: {}", index + 1, message.into()),
        }
    }
}

/// A request to check a draft shared curve against a GPU's current limits. Nothing is applied.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PreviewFanCurveRequest {
    pub gpu_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fan_index: Option<u32>,
    pub points: Vec<FanCurvePoint>,
}

/// What keeps a previewed curve from being usable; no problems means it is valid.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FanCurvePreview {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub support: Option<FanCurveSupport>,
    pub problems: Vec<FanCurveProblem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CurveLimits {
    max_operating_temperature_c: i64,
    full_speed_temperature_c: i64,
    min_speed_percent: i64,
    max_speed_percent: i64,
}

/// Judges whether a shared curve can control every fan the GPU reports.
pub fn curve_support(cooling: &CoolingInfo) -> FanCurveSupport {
    curve_support_for(cooling, &every_fan(cooling))
}

pub fn curve_support_for(cooling: &CoolingInfo, fan_indexes: &[u32]) -> FanCurveSupport {
    match curve_limits_for(cooling, fan_indexes) {
        Ok(limits) => {
            let starter = starter_curve(limits);
            if !validate_points(limits, &starter).is_empty() {
                return FanCurveSupport::Unavailable {
                    reason: "Prism can't build a valid starter fan curve within this GPU's limits."
                        .into(),
                };
            }
            FanCurveSupport::Available {
                max_operating_temperature_c: limits.max_operating_temperature_c,
                full_speed_temperature_c: limits.full_speed_temperature_c,
                min_speed_percent: limits.min_speed_percent,
                max_speed_percent: limits.max_speed_percent,
                starter,
            }
        }
        Err(reason) => FanCurveSupport::Unavailable { reason },
    }
}

/// The problems that keep the points from forming a usable shared curve; none means valid.
pub fn validate(cooling: &CoolingInfo, points: &[FanCurvePoint]) -> Vec<FanCurveProblem> {
    validate_for(cooling, &every_fan(cooling), points)
}

fn every_fan(cooling: &CoolingInfo) -> Vec<u32> {
    cooling.fans.iter().map(|fan| fan.index).collect()
}

/// The problems that keep the points from forming one curve shared by these fans.
pub fn validate_for(
    cooling: &CoolingInfo,
    fan_indexes: &[u32],
    points: &[FanCurvePoint],
) -> Vec<FanCurveProblem> {
    match curve_limits_for(cooling, fan_indexes) {
        Ok(limits) => validate_points(limits, points),
        Err(reason) => vec![FanCurveProblem::curve(reason)],
    }
}

/// The speed the curve requests at the temperature: a straight line between neighboring points,
/// rounded up to whole percent, and the nearest end point's speed beyond either end.
pub fn requested_speed(points: &[FanCurvePoint], temperature_c: i64) -> i64 {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return 0;
    };
    if temperature_c <= first.temperature_c {
        return first.speed_percent;
    }
    if temperature_c >= last.temperature_c {
        return last.speed_percent;
    }
    let mut speed = last.speed_percent;
    for pair in points.windows(2) {
        let (low, high) = (pair[0], pair[1]);
        if temperature_c <= high.temperature_c {
            let span = high.temperature_c - low.temperature_c;
            let rise =
                (high.speed_percent - low.speed_percent) * (temperature_c - low.temperature_c);
            speed = low.speed_percent + (rise + span - 1).div_euclid(span);
            break;
        }
    }
    speed
}

fn curve_limits_for(cooling: &CoolingInfo, fan_indexes: &[u32]) -> Result<CurveLimits, String> {
    if !cooling.is_supported || cooling.fans.is_empty() || fan_indexes.is_empty() {
        return Err(format!(
            "Fan curves need controllable fans. {}",
            cooling
                .unsupported_reason
                .as_deref()
                .unwrap_or("This GPU reports no controllable fans.")
        ));
    }
    let mut min_speed = 0;
    let mut max_speed = None;
    for &index in fan_indexes {
        let number = index + 1;
        let Some(fan) = cooling.fans.iter().find(|fan| fan.index == index) else {
            return Err(format!("Fan {number} isn't reported by this GPU."));
        };
        let (Some(min), Some(max)) = (fan.min_speed_percent, fan.max_speed_percent) else {
            return Err(format!(
                "Fan {number} doesn't report its supported speed range."
            ));
        };
        if min > max || max > 100 {
            return Err(format!(
                "Fan {number} reports an invalid speed range ({min}-{max}%)."
            ));
        }
        if max_speed.is_some_and(|shared| shared != max) {
            return Err(
                "This GPU's fans report different maximum speeds, so one shared curve can't reach each fan's maximum."
                    .into(),
            );
        }
        min_speed = min_speed.max(min);
        max_speed = Some(max);
    }
    let Some(max_operating_temperature_c) = cooling.max_operating_temperature_c else {
        return Err(format!(
            "Fan curves need the GPU's maximum operating temperature. {}",
            cooling
                .max_operating_temperature_reason
                .as_deref()
                .unwrap_or("This GPU doesn't report one.")
        ));
    };
    let max_operating_temperature_c = i64::from(max_operating_temperature_c);
    Ok(CurveLimits {
        max_operating_temperature_c,
        full_speed_temperature_c: max_operating_temperature_c - FULL_SPEED_MARGIN_C,
        min_speed_percent: i64::from(min_speed),
        max_speed_percent: i64::from(max_speed.unwrap_or_default()),
    })
}

/// T−40, T−20, T−10 and T at the minimum speed, halfway, three quarters of the way, and the
/// maximum speed, rounded to whole percent.
fn starter_curve(limits: CurveLimits) -> Vec<FanCurvePoint> {
    let full = limits.full_speed_temperature_c;
    let min = limits.min_speed_percent;
    let range = limits.max_speed_percent - min;
    let fraction = |quarters: i64| min + (range * quarters + 2) / 4;
    vec![
        FanCurvePoint::new(full - 40, min),
        FanCurvePoint::new(full - 20, fraction(2)),
        FanCurvePoint::new(full - 10, fraction(3)),
        FanCurvePoint::new(full, limits.max_speed_percent),
    ]
}

fn validate_points(limits: CurveLimits, points: &[FanCurvePoint]) -> Vec<FanCurveProblem> {
    let mut problems = Vec::new();
    if !(MIN_POINTS..=MAX_POINTS).contains(&points.len()) {
        problems.push(FanCurveProblem::curve(format!(
            "A curve needs {MIN_POINTS} to {MAX_POINTS} points."
        )));
    }
    let CurveLimits {
        max_operating_temperature_c: max_temperature,
        full_speed_temperature_c: full_speed_temperature,
        min_speed_percent: min_speed,
        max_speed_percent: max_speed,
    } = limits;
    for (index, point) in points.iter().enumerate() {
        if !(0..=max_temperature).contains(&point.temperature_c) {
            problems.push(FanCurveProblem::point(
                index,
                format!("temperature must be between 0 and {max_temperature}°C."),
            ));
        }
        if !(min_speed..=max_speed).contains(&point.speed_percent) {
            problems.push(FanCurveProblem::point(
                index,
                format!("speed must be between {min_speed}% and {max_speed}%."),
            ));
        }
        if let Some(previous) = index.checked_sub(1).map(|previous| points[previous]) {
            if point.temperature_c <= previous.temperature_c {
                problems.push(FanCurveProblem::point(
                    index,
                    format!(
                        "temperature must be higher than point {index}'s {}°C.",
                        previous.temperature_c
                    ),
                ));
            }
            if point.speed_percent < previous.speed_percent {
                problems.push(FanCurveProblem::point(
                    index,
                    format!(
                        "speed can't be lower than point {index}'s {}%.",
                        previous.speed_percent
                    ),
                ));
            }
        }
    }
    // With nondecreasing speeds, the curve requests the maximum at T exactly when a point at or
    // below T does.
    let reaches_full_speed = points.iter().any(|point| {
        point.temperature_c <= full_speed_temperature && point.speed_percent == max_speed
    });
    if !points.is_empty() && !reaches_full_speed {
        problems.push(FanCurveProblem::curve(format!(
            "The curve must reach {max_speed}% by {full_speed_temperature}°C, {FULL_SPEED_MARGIN_C}°C below the GPU's {max_temperature}°C maximum operating temperature."
        )));
    }
    problems
}
