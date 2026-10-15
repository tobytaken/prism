//! The fan curve editor: the curve a GPU's fans follow or the one pending in its place, its points
//! moved by dragging or with the keyboard, what keeps a pending curve from being applied, and
//! where the GPU's temperature and fan speed are now. While each fan has its own settings, it
//! edits the picked fan's curve.

use prism_gpu::cooling::{CoolingConfiguration, FanNumber, FanSetting};
use prism_gpu::fan_curve::{self, FanCurvePoint, FanCurveProblem, FanCurveSupport};

use crate::cooling::{Fans, cooling_read, setting_of, speed_now};
use crate::core::{Core, Effect, Gpu, Latest};
use crate::format::Readout;
use crate::telemetry::ChartPoint;
use crate::view::{CurveMarker, CurvePoint, FanCurveEditor};

/// The temperatures the editor's axis covers at least, in °C, widened to fit the GPU's maximum
/// operating temperature and every point.
const AXIS: Axis = Axis { from: 20, to: 100 };
/// The step a widened axis snaps to, so its ends are round numbers.
const AXIS_STEP: i64 = 10;

/// Why a pending curve can't be checked without a reading of the GPU's fans.
const NO_READING: &str = "Prism needs a reading of the GPU's fans to check the curve.";

/// The points of the curve `fans` follow in `configuration`, if they follow one.
fn curve_of(configuration: &CoolingConfiguration, fans: Fans) -> Option<&[FanCurvePoint]> {
    match setting_of(configuration, fans)? {
        FanSetting::Curve { points } => Some(points),
        FanSetting::Automatic {} | FanSetting::Fixed { .. } => None,
    }
}

/// Where temperatures and speeds are drawn in the editor: temperatures from `from` °C at the left
/// edge to `to` °C at the right, and speeds from 0% at the bottom to 100% at the top.
#[derive(Debug, Clone, Copy)]
struct Axis {
    from: i64,
    to: i64,
}

/// What a custom curve on the GPU's fans must keep within.
#[derive(Debug, Clone, Copy)]
struct Limits {
    max_operating_temperature_c: i64,
    full_speed_temperature_c: i64,
    min_speed_percent: i64,
    max_speed_percent: i64,
}

impl Limits {
    fn of(support: Option<FanCurveSupport>) -> Option<Self> {
        match support? {
            FanCurveSupport::Available {
                max_operating_temperature_c,
                full_speed_temperature_c,
                min_speed_percent,
                max_speed_percent,
                ..
            } => Some(Self {
                max_operating_temperature_c,
                full_speed_temperature_c,
                min_speed_percent,
                max_speed_percent,
            }),
            FanCurveSupport::Unavailable { .. } => None,
        }
    }
}

impl Axis {
    /// Where `temperature_c` and `speed_percent` are drawn, from 0 to 1 across and down.
    fn drawn_at(self, temperature_c: i64, speed_percent: i64) -> ChartPoint {
        ChartPoint {
            x: (temperature_c - self.from) as f32 / (self.to - self.from) as f32,
            y: (100 - speed_percent) as f32 / 100.0,
        }
    }

    /// The temperature and speed drawn at `x` across and `y` down, from 0 to 1, to the nearest
    /// whole degree and percent.
    fn curve_point_at(self, x: f32, y: f32) -> FanCurvePoint {
        let span = (self.to - self.from) as f32;
        FanCurvePoint::new(
            self.from + (x.clamp(0.0, 1.0) * span).round() as i64,
            100 - (y.clamp(0.0, 1.0) * 100.0).round() as i64,
        )
    }

    /// A curve as a line across the editor: its speed at the first point from the left edge, and
    /// at the last point to the right edge, as the curve requests beyond either end.
    fn line(self, points: &[FanCurvePoint]) -> Vec<ChartPoint> {
        let (Some(first), Some(last)) = (points.first(), points.last()) else {
            return Vec::new();
        };
        let start = FanCurvePoint::new(self.from.min(first.temperature_c), first.speed_percent);
        let end = FanCurvePoint::new(self.to.max(last.temperature_c), last.speed_percent);
        std::iter::once(start)
            .chain(points.iter().copied())
            .chain(std::iter::once(end))
            .map(|point| self.drawn_at(point.temperature_c, point.speed_percent))
            .collect()
    }
}

impl Core {
    /// The curve the fans the GPU's controls set follow, if they follow one.
    fn curve_in_effect(&self, gpu: &Gpu) -> Option<Vec<FanCurvePoint>> {
        curve_of(&self.cooling_in_effect(gpu)?, self.controlled_fans(gpu)).map(<[_]>::to_vec)
    }

    /// The curve the GPU's draft stages for the fans its controls set, if it stages one other
    /// than the curve they follow.
    fn staged_curve(&self, gpu: &Gpu) -> Option<&[FanCurvePoint]> {
        let staged = curve_of(self.staged_cooling(&gpu.id)?, self.controlled_fans(gpu))?;
        (self.curve_in_effect(gpu).as_deref() != Some(staged)).then_some(staged)
    }

    /// The curve the editor shows: the one pending, or else the one in effect, unless other
    /// cooling is pending.
    fn shown_curve(&self, gpu: &Gpu) -> Option<Vec<FanCurvePoint>> {
        curve_of(&self.shown_cooling(gpu)?, self.controlled_fans(gpu)).map(<[_]>::to_vec)
    }

    /// The temperatures the editor's axis covers for the GPU: at least 20–100 °C, and widened
    /// to the maximum operating temperature and every point in effect or pending.
    fn curve_axis(&self, gpu: &Gpu) -> Axis {
        let temperatures: Vec<_> = [self.curve_in_effect(gpu), self.shown_curve(gpu)]
            .into_iter()
            .flatten()
            .flatten()
            .map(|point| point.temperature_c)
            .chain(
                cooling_read(gpu)
                    .and_then(|cooling| cooling.max_operating_temperature_c)
                    .map(i64::from),
            )
            .collect();
        let lowest = temperatures.iter().min().copied().unwrap_or(AXIS.from);
        let highest = temperatures.iter().max().copied().unwrap_or(AXIS.to);
        Axis {
            from: AXIS.from.min(lowest.div_euclid(AXIS_STEP) * AXIS_STEP),
            to: AXIS
                .to
                .max((highest + AXIS_STEP - 1).div_euclid(AXIS_STEP) * AXIS_STEP),
        }
    }

    /// What keeps the curves the GPU's draft stages from being applied, each fan's own curve's
    /// problems naming the fan; none while it stages none. Without a reading of the GPU's fans,
    /// there's nothing to check them against.
    pub(crate) fn curve_problems(&self, gpu_id: &str) -> Vec<String> {
        let curves: Vec<(Fans, &[FanCurvePoint])> = match self.staged_cooling(gpu_id) {
            Some(CoolingConfiguration::Shared {
                setting: FanSetting::Curve { points },
            }) => vec![(Fans::Every, points)],
            Some(CoolingConfiguration::PerFan { fans }) => fans
                .iter()
                .filter_map(|fan| match &fan.setting {
                    FanSetting::Curve { points } => Some((Fans::One(fan.fan_index), &points[..])),
                    FanSetting::Automatic {} | FanSetting::Fixed { .. } => None,
                })
                .collect(),
            Some(CoolingConfiguration::Shared { .. }) | None => Vec::new(),
        };
        if curves.is_empty() {
            return Vec::new();
        }
        let Some(cooling) = self
            .gpus()
            .iter()
            .find(|gpu| gpu.id == gpu_id)
            .and_then(cooling_read)
        else {
            return vec![NO_READING.into()];
        };
        curves
            .into_iter()
            .flat_map(|(fans, points)| {
                fan_curve::validate_for(cooling, &fans.indexes(cooling), points)
                    .into_iter()
                    .map(move |problem| match fans {
                        Fans::Every => problem.message,
                        Fans::One(index) => {
                            format!("Fan {}: {}", FanNumber(index), problem.message)
                        }
                    })
            })
            .collect()
    }

    /// What keeps the curve pending in the editor from being applied; none while none is pending.
    fn shown_curve_problems(&self, gpu: &Gpu) -> Vec<FanCurveProblem> {
        let Some(points) = self.staged_curve(gpu) else {
            return Vec::new();
        };
        match cooling_read(gpu) {
            Some(cooling) => fan_curve::validate_for(
                cooling,
                &self.controlled_fans(gpu).indexes(cooling),
                points,
            ),
            None => vec![FanCurveProblem {
                point: None,
                message: NO_READING.into(),
            }],
        }
    }

    /// The Fans track's fan curve editor, while the GPU's fans follow a curve or one is pending.
    pub(crate) fn fan_curve(&self, gpu: &Gpu) -> Option<FanCurveEditor> {
        let shown = self.shown_curve(gpu)?;
        let axis = self.curve_axis(gpu);
        let fans = self.controlled_fans(gpu);
        let pending = self.staged_curve(gpu).is_some();
        let problems = self.shown_curve_problems(gpu);
        let support = self.curve_support(gpu, fans);
        let read_only = self.cooling_read_only(gpu).or_else(|| match &support {
            Some(FanCurveSupport::Unavailable { reason }) => Some(reason.clone()),
            Some(FanCurveSupport::Available { .. }) | None => None,
        });
        let limits = Limits::of(support);
        let points = shown
            .iter()
            .enumerate()
            .map(|(index, point)| CurvePoint {
                temperature_c: point.temperature_c,
                speed_percent: point.speed_percent,
                at: axis.drawn_at(point.temperature_c, point.speed_percent),
                name: format!("Fan curve point {}", index + 1),
                value_text: format!(
                    "{} at {}",
                    percent(point.speed_percent).text(),
                    celsius(point.temperature_c).text()
                ),
                faulty: problems.iter().any(|problem| problem.point == Some(index)),
            })
            .collect();
        let temperature = match &gpu.latest {
            Latest::Read(device) => device.telemetry.temperature_c.map(i64::from),
            Latest::Waiting | Latest::Missing { .. } => None,
        };
        let marker = temperature
            .zip(speed_now(gpu, fans))
            .map(|(temperature, speed)| {
                let fans = match fans {
                    Fans::Every => "fans".to_owned(),
                    Fans::One(index) => format!("fan {}", FanNumber(index)),
                };
                CurveMarker {
                    at: axis.drawn_at(temperature, speed),
                    text: format!(
                        "Now {}, {fans} at {}",
                        celsius(temperature).text(),
                        percent(speed).text()
                    ),
                }
            });
        Some(FanCurveEditor {
            open: !self.fan_curve_collapsed,
            points,
            pending,
            in_effect_line: self.curve_in_effect(gpu).map(|points| axis.line(&points)),
            pending_line: pending.then(|| axis.line(&shown)),
            temperature_axis: (axis.from, axis.to),
            temperature_labels: [celsius(axis.from).text(), celsius(axis.to).text()],
            speeds: limits.map_or((0, 100), |limits| {
                (limits.min_speed_percent, limits.max_speed_percent)
            }),
            full_speed_x: limits.map(|limits| axis.drawn_at(limits.full_speed_temperature_c, 0).x),
            marker,
            problems: problems
                .into_iter()
                .map(|problem| problem.message)
                .collect(),
            read_only,
        })
    }

    /// Moves the selected GPU's curve point `index` by `temperature_by` °C and `speed_by` %, as
    /// the arrow keys do.
    pub(crate) fn move_curve_point(
        &mut self,
        index: usize,
        temperature_by: i64,
        speed_by: i64,
    ) -> Vec<Effect> {
        self.edit_curve_point(index, |point, _| {
            FanCurvePoint::new(
                point.temperature_c + temperature_by,
                point.speed_percent + speed_by,
            )
        })
    }

    /// Moves the selected GPU's curve point `index` to where `x` across and `y` down, from 0 to
    /// 1, are drawn in the editor, as dragging it does.
    pub(crate) fn drag_curve_point(&mut self, index: usize, x: f32, y: f32) -> Vec<Effect> {
        self.edit_curve_point(index, |_, axis| axis.curve_point_at(x, y))
    }

    /// Stages the selected GPU's curve with point `index` moved to where `to` puts it: between
    /// its neighbors' temperatures, no hotter than the maximum operating temperature, and within
    /// the speeds every fan supports. Whether the curve reaches full speed in time, or keeps its
    /// speeds rising, is left for the editor to explain.
    fn edit_curve_point(
        &mut self,
        index: usize,
        to: impl FnOnce(FanCurvePoint, Axis) -> FanCurvePoint,
    ) -> Vec<Effect> {
        self.stage_fan_setting(|core, gpu, _, fans| {
            let limits = Limits::of(core.curve_support(gpu, fans))?;
            let mut points = core.shown_curve(gpu)?;
            let point = *points.get(index)?;
            let wanted = to(point, core.curve_axis(gpu));
            let coolest = index
                .checked_sub(1)
                .map_or(0, |previous| points[previous].temperature_c + 1);
            let hottest = points
                .get(index + 1)
                .map_or(limits.max_operating_temperature_c, |next| {
                    next.temperature_c - 1
                })
                .min(limits.max_operating_temperature_c);
            points[index] = FanCurvePoint::new(
                if coolest <= hottest {
                    wanted.temperature_c.clamp(coolest, hottest)
                } else {
                    point.temperature_c
                },
                wanted
                    .speed_percent
                    .clamp(limits.min_speed_percent, limits.max_speed_percent),
            );
            Some(FanSetting::Curve { points })
        })
    }

    /// Collapses the fan curve editor, or opens it again.
    pub(crate) fn toggle_fan_curve(&mut self) -> Vec<Effect> {
        self.fan_curve_collapsed = !self.fan_curve_collapsed;
        Vec::new()
    }
}

/// A temperature as shown, such as "65 °C".
fn celsius(temperature_c: i64) -> Readout {
    Readout::celsius(temperature_c.try_into().ok())
}

/// A speed as shown, such as "65%".
fn percent(speed_percent: i64) -> Readout {
    Readout::percent(speed_percent.try_into().ok())
}
