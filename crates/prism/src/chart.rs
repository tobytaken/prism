//! Chart geometry: turns the core's chart data into Slint `Path` commands.

use std::fmt::Write as _;

use prism_ui::telemetry::Chart;

/// Path commands for a line through each run of a chart's readings, in a chart `width` by
/// `height` pixels; a run of one reading has no line.
pub(crate) fn series_commands(chart: &Chart, width: f32, height: f32) -> String {
    let mut commands = String::new();
    for segment in chart.segments.iter().filter(|segment| segment.len() > 1) {
        for (index, point) in segment.iter().enumerate() {
            if !commands.is_empty() {
                commands.push(' ');
            }
            let verb = if index == 0 { 'M' } else { 'L' };
            let _ = write!(
                commands,
                "{verb}{:.1} {:.1}",
                point.x * width,
                point.y * height
            );
        }
    }
    commands
}

#[cfg(test)]
mod tests {
    use prism_ui::telemetry::{Chart, ChartPoint};

    use super::series_commands;

    fn point(x: f32, y: f32) -> ChartPoint {
        ChartPoint { x, y }
    }

    #[test]
    fn a_series_is_a_line_through_each_run_of_known_readings() {
        let chart = Chart {
            min: 20,
            max: 100,
            segments: vec![
                vec![point(0.25, 1.0), point(0.5, 0.5)],
                vec![point(0.75, 0.0), point(1.0, 0.25)],
            ],
        };

        assert_eq!(
            series_commands(&chart, 200.0, 100.0),
            "M50.0 100.0 L100.0 50.0 M150.0 0.0 L200.0 25.0"
        );
    }

    #[test]
    fn a_lone_reading_has_no_line_to_draw() {
        let chart = Chart {
            min: 20,
            max: 100,
            segments: vec![vec![point(1.0, 0.5)]],
        };

        assert_eq!(series_commands(&chart, 200.0, 100.0), "");
    }
}
