//! Chart geometry: turns the core's chart data into Slint `Path` commands.

use std::fmt::Write as _;

use prism_ui::telemetry::{Chart, ChartPoint};

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

/// Path commands for a line through `points`, such as a fan curve's, in a chart `width` by
/// `height` pixels.
pub(crate) fn line_commands(points: &[ChartPoint], width: f32, height: f32) -> String {
    points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let verb = if index == 0 { 'M' } else { 'L' };
            format!("{verb}{:.1} {:.1}", point.x * width, point.y * height)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Path commands for the area under each run of a chart's readings, down to the bottom of a
/// chart `width` by `height` pixels; a run of one reading has no area.
pub(crate) fn area_commands(chart: &Chart, width: f32, height: f32) -> String {
    let mut commands = Vec::new();
    for segment in chart.segments.iter().filter(|segment| segment.len() > 1) {
        let (first, last) = (segment[0], segment[segment.len() - 1]);
        commands.push(format!("M{:.1} {height:.1}", first.x * width));
        for point in segment {
            commands.push(format!("L{:.1} {:.1}", point.x * width, point.y * height));
        }
        commands.push(format!("L{:.1} {height:.1} Z", last.x * width));
    }
    commands.join(" ")
}

/// Path commands for a limit across a chart `width` by `height` pixels at `y`, from the top (0) to
/// the bottom (1): one line, or dashes `(dash, gap)` long from the left edge.
pub(crate) fn limit_commands(
    y: f32,
    width: f32,
    height: f32,
    dashes: Option<(f32, f32)>,
) -> String {
    let y = y * height;
    let line = |from: f32, to: f32| format!("M{from:.1} {y:.1} L{to:.1} {y:.1}");
    match dashes {
        Some((dash, gap)) if dash > 0.0 && gap >= 0.0 => {
            let mut commands = Vec::new();
            let mut from = 0.0;
            while from < width {
                commands.push(line(from, (from + dash).min(width)));
                from += dash + gap;
            }
            commands.join(" ")
        }
        _ => line(0.0, width),
    }
}

#[cfg(test)]
mod tests {
    use prism_ui::telemetry::{Chart, ChartPoint};

    use super::{area_commands, limit_commands, line_commands, series_commands};

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
    fn a_line_runs_through_every_point() {
        assert_eq!(
            line_commands(
                &[point(0.0, 1.0), point(0.5, 0.25), point(1.0, 0.0)],
                200.0,
                100.0
            ),
            "M0.0 100.0 L100.0 25.0 L200.0 0.0"
        );
        assert_eq!(line_commands(&[], 200.0, 100.0), "");
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

    #[test]
    fn an_area_fills_under_each_run_down_to_the_bottom() {
        let chart = Chart {
            min: 0,
            max: 100,
            segments: vec![
                vec![point(0.25, 1.0), point(0.5, 0.5)],
                vec![point(0.75, 0.0)],
            ],
        };

        assert_eq!(
            area_commands(&chart, 200.0, 100.0),
            "M50.0 100.0 L50.0 100.0 L100.0 50.0 L100.0 100.0 Z"
        );
    }

    #[test]
    fn a_solid_limit_crosses_the_whole_chart() {
        assert_eq!(
            limit_commands(0.25, 200.0, 100.0, None),
            "M0.0 25.0 L200.0 25.0"
        );
    }

    #[test]
    fn a_dashed_limit_is_built_from_dashes_cut_off_at_the_edge() {
        assert_eq!(
            limit_commands(0.5, 20.0, 100.0, Some((4.0, 3.0))),
            "M0.0 50.0 L4.0 50.0 M7.0 50.0 L11.0 50.0 M14.0 50.0 L18.0 50.0"
        );
        assert_eq!(
            limit_commands(0.5, 16.0, 100.0, Some((4.0, 3.0))),
            "M0.0 50.0 L4.0 50.0 M7.0 50.0 L11.0 50.0 M14.0 50.0 L16.0 50.0"
        );
    }

    #[test]
    fn a_dashed_limit_without_dashes_to_draw_is_solid() {
        assert_eq!(
            limit_commands(0.5, 20.0, 100.0, Some((0.0, 3.0))),
            "M0.0 50.0 L20.0 50.0"
        );
    }
}
