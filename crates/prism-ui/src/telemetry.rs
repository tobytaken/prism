//! GPU telemetry history, kept in memory for the session, and its range statistics.

use std::collections::VecDeque;
use std::time::Duration;

/// One GPU's readings at one time; an unavailable reading is unknown, never zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Readings {
    pub temperature_c: Option<u32>,
    pub utilization_percent: Option<u32>,
    pub power_draw_watts: Option<u32>,
    pub fan_speed_percent: Option<u32>,
}

impl Readings {
    pub fn get(&self, metric: Metric) -> Option<u32> {
        match metric {
            Metric::Temperature => self.temperature_c,
            Metric::Utilization => self.utilization_percent,
            Metric::PowerDraw => self.power_draw_watts,
            Metric::FanSpeed => self.fan_speed_percent,
        }
    }
}

/// A measurement kept in the telemetry history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    Temperature,
    Utilization,
    PowerDraw,
    FanSpeed,
}

/// How far back a track looks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TelemetryRange {
    OneMinute,
    #[default]
    FiveMinutes,
    FifteenMinutes,
    OneHour,
}

impl Metric {
    /// The span a chart of this metric shows at least, widened to fit readings outside it.
    fn chart_span(self) -> (u32, u32) {
        match self {
            Self::Temperature => (20, 100),
            Self::Utilization | Self::PowerDraw | Self::FanSpeed => (0, 100),
        }
    }
}

impl TelemetryRange {
    /// The range as the range picker names it, such as "5 m".
    pub fn label(self) -> &'static str {
        match self {
            Self::OneMinute => "1 m",
            Self::FiveMinutes => "5 m",
            Self::FifteenMinutes => "15 m",
            Self::OneHour => "1 h",
        }
    }

    pub fn duration(self) -> Duration {
        Duration::from_secs(match self {
            Self::OneMinute => 60,
            Self::FiveMinutes => 5 * 60,
            Self::FifteenMinutes => 15 * 60,
            Self::OneHour => 60 * 60,
        })
    }
}

/// The largest, mean and smallest known reading of a metric within a range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RangeStats {
    pub max: u32,
    pub avg: f64,
    pub min: u32,
}

/// A metric's readings within a range, ready to draw: unbroken runs of known readings, with the
/// value axis running from `min` at the bottom to `max` at the top.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Chart {
    pub min: u32,
    pub max: u32,
    pub segments: Vec<Vec<ChartPoint>>,
}

/// A position in a chart: `x` runs from the start of the range (0) to the newest sample (1), and
/// `y` from the top (0) to the bottom (1), as drawing coordinates do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChartPoint {
    pub x: f32,
    pub y: f32,
}

/// One GPU's readings over the session, oldest first, each stamped with when Prism received it.
#[derive(Debug, Clone, Default)]
pub struct TelemetryHistory {
    samples: VecDeque<(Duration, Readings)>,
}

impl TelemetryHistory {
    /// Enough samples for the longest range at 1 Hz.
    pub const CAPACITY: usize = 3600;

    pub fn record(&mut self, received_at: Duration, readings: Readings) {
        if self.samples.len() == Self::CAPACITY {
            self.samples.pop_front();
        }
        self.samples.push_back((received_at, readings));
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// The statistics of `metric` over the samples received within `range` before `now`, or
    /// none when no reading in the range is known.
    pub fn stats(
        &self,
        metric: Metric,
        range: TelemetryRange,
        now: Duration,
    ) -> Option<RangeStats> {
        let mut known = self
            .in_range(range, now)
            .filter_map(|(_, readings)| readings.get(metric));
        let first = known.next()?;
        let (mut max, mut min, mut sum, mut count) = (first, first, u64::from(first), 1u64);
        for value in known {
            max = max.max(value);
            min = min.min(value);
            sum += u64::from(value);
            count += 1;
        }
        Some(RangeStats {
            max,
            avg: sum as f64 / count as f64,
            min,
        })
    }

    /// `metric` over `range` before `now`, as a chart.
    pub fn chart(&self, metric: Metric, range: TelemetryRange, now: Duration) -> Chart {
        let (mut min, mut max) = metric.chart_span();
        if let Some(stats) = self.stats(metric, range, now) {
            min = min.min(stats.min / 10 * 10);
            max = max.max(stats.max.div_ceil(10) * 10);
        }
        let span = range.duration().as_secs_f32();
        let mut segments = Vec::new();
        let mut segment = Vec::new();
        for (at, readings) in self.in_range(range, now) {
            match readings.get(metric) {
                Some(value) => segment.push(ChartPoint {
                    x: 1.0 - now.saturating_sub(*at).as_secs_f32() / span,
                    y: (max - value) as f32 / (max - min) as f32,
                }),
                None if !segment.is_empty() => segments.push(std::mem::take(&mut segment)),
                None => {}
            }
        }
        if !segment.is_empty() {
            segments.push(segment);
        }
        Chart { min, max, segments }
    }

    fn in_range(
        &self,
        range: TelemetryRange,
        now: Duration,
    ) -> impl Iterator<Item = &(Duration, Readings)> {
        self.samples
            .iter()
            .filter(move |(at, _)| now.saturating_sub(*at) < range.duration())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        Chart, ChartPoint, Metric, RangeStats, Readings, TelemetryHistory, TelemetryRange,
    };

    fn temperature(celsius: Option<u32>) -> Readings {
        Readings {
            temperature_c: celsius,
            ..Readings::default()
        }
    }

    fn secs(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }

    #[test]
    fn range_statistics_cover_the_known_readings_in_the_range() {
        let mut history = TelemetryHistory::default();
        history.record(secs(1), temperature(Some(40)));
        history.record(secs(2), temperature(None));
        history.record(secs(3), temperature(Some(71)));
        history.record(secs(4), temperature(Some(45)));

        assert_eq!(
            history.stats(Metric::Temperature, TelemetryRange::OneMinute, secs(4)),
            Some(RangeStats {
                max: 71,
                avg: 52.0,
                min: 40,
            })
        );
    }

    #[test]
    fn range_statistics_leave_out_samples_received_before_the_range() {
        let mut history = TelemetryHistory::default();
        history.record(secs(0), temperature(Some(90)));
        history.record(secs(1), temperature(Some(50)));
        history.record(secs(60), temperature(Some(60)));

        let stats = history.stats(Metric::Temperature, TelemetryRange::OneMinute, secs(60));

        assert_eq!(
            stats,
            Some(RangeStats {
                max: 60,
                avg: 55.0,
                min: 50,
            })
        );
    }

    #[test]
    fn a_range_with_only_unknown_readings_has_no_statistics() {
        let mut history = TelemetryHistory::default();
        history.record(secs(0), temperature(Some(70)));
        history.record(secs(100), temperature(None));

        assert_eq!(
            history.stats(Metric::Temperature, TelemetryRange::OneMinute, secs(100)),
            None
        );
        assert_eq!(
            TelemetryHistory::default().stats(
                Metric::Temperature,
                TelemetryRange::OneHour,
                secs(0)
            ),
            None
        );
    }

    #[test]
    fn history_keeps_the_last_hour_of_one_hertz_samples() {
        let mut history = TelemetryHistory::default();
        history.record(secs(0), temperature(Some(99)));
        for second in 1..=3600 {
            history.record(secs(second), temperature(Some(40)));
        }

        assert_eq!(history.len(), 3600);
    }

    fn point(x: f32, y: f32) -> ChartPoint {
        ChartPoint { x, y }
    }

    #[test]
    fn a_chart_places_known_readings_by_age_and_breaks_at_unknown_ones() {
        let mut history = TelemetryHistory::default();
        history.record(secs(0), temperature(Some(90)));
        history.record(secs(15), temperature(Some(20)));
        history.record(secs(30), temperature(Some(60)));
        history.record(secs(45), temperature(None));
        history.record(secs(60), temperature(Some(100)));

        let chart = history.chart(Metric::Temperature, TelemetryRange::OneMinute, secs(60));

        // Temperature charts span 20–100 °C; the newest sample sits at the right edge.
        assert_eq!(
            chart,
            Chart {
                min: 20,
                max: 100,
                segments: vec![
                    vec![point(0.25, 1.0), point(0.5, 0.5)],
                    vec![point(1.0, 0.0)],
                ],
            }
        );
    }

    #[test]
    fn a_chart_widens_its_scale_to_fit_readings_outside_it() {
        let mut history = TelemetryHistory::default();
        history.record(secs(59), temperature(Some(14)));
        history.record(secs(60), temperature(Some(121)));

        let chart = history.chart(Metric::Temperature, TelemetryRange::OneMinute, secs(60));

        assert_eq!((chart.min, chart.max), (10, 130));
    }
}
