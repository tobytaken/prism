//! View tests for the GPU workspace's monitoring: its five tracks, the range picker and the
//! hardware details.

mod support;

use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_gpu::{GpuPerformanceResponse, GpuTelemetryProvider};
use slint::ComponentHandle;

use support::prismd::DaemonState;
use support::{DEFAULT_SIZE, HALF_1080P, Harness, MINIMUM_SIZE};

use AccessibleRole::{Button, Groupbox, Spinbox, Text};

/// Each track, with the label of its readout and the metric its statistics name.
const TRACKS: [(&str, &str, &str); 5] = [
    ("Temperature", "Temperature reading", "temperature"),
    ("Utilization", "Utilization reading", "utilization"),
    ("Power", "Power draw reading", "power draw"),
    ("Clocks", "Graphics clock reading", "graphics clock"),
    ("Fans", "Fan speed reading", "fan speed"),
];

fn value(harness: &Harness, label: &str) -> String {
    harness
        .find(Text, label)
        .accessible_value()
        .map(String::from)
        .unwrap_or_default()
}

/// What the power limit's stepper says its value is.
fn power_limit(harness: &Harness) -> String {
    harness
        .find(Spinbox, "Power limit")
        .accessible_value()
        .map(String::from)
        .unwrap_or_default()
}

fn description(harness: &Harness, label: &str) -> String {
    harness
        .find(Text, label)
        .accessible_description()
        .map(String::from)
        .unwrap_or_default()
}

/// How many elements of the chart part `type_name` the window draws.
fn chart_parts(harness: &Harness, type_name: &str) -> usize {
    ElementHandle::find_by_element_type_name(harness.app.window(), type_name).count()
}

/// The tunable GPU as prismd would push it, `second` seconds into its simulation.
fn prismd_reading(second: u64) -> GpuPerformanceResponse {
    SimulatedGpus::new(SimulatedFixture::TunableGpu, move || {
        Duration::from_secs(second)
    })
    .performance()
}

/// Asserts that every track shows a known readout and max/avg/min, and that every chart draws.
fn assert_tracks_filled(harness: &Harness) {
    for (track, reading, metric) in TRACKS {
        harness.find(Groupbox, track);
        assert_ne!(value(harness, reading), "unknown", "{reading}");
        for statistic in ["Maximum", "Average", "Minimum"] {
            let label = format!("{statistic} {metric}");
            assert_ne!(value(harness, &label), "unknown", "{label}");
        }
    }
    // Temperature, utilization, power draw, graphics and memory clocks, and fan speed.
    assert_eq!(chart_parts(harness, "Series"), 6);
    assert_eq!(chart_parts(harness, "Area"), 1);
    // The configured and enforced power limits.
    assert_eq!(chart_parts(harness, "LimitLine"), 2);
}

#[test]
fn the_five_tracks_fill_from_in_process_telemetry() {
    let harness = Harness::new(SimulatedFixture::TunableGpu);
    // The tunable GPU's second and third seconds run at 4% and 3% load.
    harness.ticks(2);

    assert_tracks_filled(&harness);
    assert_eq!(value(&harness, "Power draw reading"), "38 W");
    assert_eq!(power_limit(&harness), "575 W");
    assert_eq!(value(&harness, "Enforced power limit"), "575 W");
    assert_eq!(value(&harness, "Graphics clock reading"), "287 MHz");
    assert_eq!(value(&harness, "Memory clock"), "812 MHz");
    assert_eq!(value(&harness, "Graphics clock offset"), "0 MHz");
    assert_eq!(value(&harness, "Memory clock offset"), "0 MHz");
    assert_eq!(value(&harness, "Fan speed reading"), "31%");
    assert_eq!(value(&harness, "Fan 0 speed"), "31%");
    assert_eq!(value(&harness, "Fan 1 speed"), "31%");
    assert_eq!(value(&harness, "Maximum power draw"), "43 W");
    assert_eq!(value(&harness, "Minimum power draw"), "38 W");
}

#[test]
fn the_five_tracks_fill_from_prismd_telemetry() {
    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, DaemonState::Running);

    harness.prismd_tick(prismd_reading(1));
    harness.prismd_tick(prismd_reading(2));

    assert_eq!(harness.in_process_reads(), 0);
    assert_tracks_filled(&harness);
    assert_eq!(value(&harness, "Power draw reading"), "38 W");
    assert_eq!(value(&harness, "Maximum power draw"), "43 W");
}

#[test]
fn the_range_picker_changes_the_charts_and_statistics() {
    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, DaemonState::Running);
    // Ten seconds of full load, then a minute and a half of idling.
    for second in 0..100 {
        let load_second = if second < 10 { 8 } else { 0 };
        harness.prismd_tick(prismd_reading(load_second));
    }
    let one_minute = harness.find(Button, "Show last 1 minute");
    let five_minutes = harness.find(Button, "Show last 5 minutes");
    assert_eq!(five_minutes.accessible_checkable(), Some(true));
    assert_eq!(five_minutes.accessible_checked(), Some(true));
    assert_eq!(one_minute.accessible_checked(), Some(false));
    assert_eq!(value(&harness, "Maximum utilization"), "100%");

    one_minute.invoke_accessible_default_action();

    assert_eq!(one_minute.accessible_checked(), Some(true));
    assert_eq!(five_minutes.accessible_checked(), Some(false));
    for (_, _, metric) in TRACKS {
        let maximum = format!("Maximum {metric}");
        let minimum = format!("Minimum {metric}");
        assert_eq!(
            value(&harness, &maximum),
            value(&harness, &minimum),
            "the last minute idled, so every {metric} reading in it is the same"
        );
    }
    assert_eq!(value(&harness, "Maximum utilization"), "2%");
}

#[test]
fn the_palette_shows_a_range() {
    let harness = Harness::new(SimulatedFixture::TunableGpu);
    harness.tick();

    harness.ctrl("k");
    harness.type_text("last hour");
    harness.press(&[], slint::platform::Key::Return);

    assert_eq!(
        harness
            .find(Button, "Show last 1 hour")
            .accessible_checked(),
        Some(true)
    );
}

#[test]
fn hardware_details_are_collapsed_until_opened_and_list_inventory_and_capabilities() {
    let harness = Harness::new(SimulatedFixture::TunableGpu);
    // Tall enough to show every fact without scrolling.
    harness.resize((1280, 1100));
    harness.tick();
    let details = harness.find(Button, "Hardware details");
    assert_eq!(details.accessible_expandable(), Some(true));
    assert_eq!(details.accessible_expanded(), Some(false));
    assert_eq!(harness.count(Text, "VBIOS"), 0);

    details.invoke_accessible_default_action();
    assert_eq!(details.accessible_expanded(), Some(true));
    for (fact, expected) in [
        ("Model", "NVIDIA GeForce RTX 5090"),
        ("VBIOS", "98.02.2E.00.01"),
        ("Memory", "32607 MiB"),
        ("Power limit", "400–600 W · default 575 W"),
        ("Graphics clock lock", "210–3090 MHz"),
        ("Fan control", "2 fans · 30–100%"),
    ] {
        let facts: Vec<_> = ElementHandle::find_by_accessible_label(harness.app.window(), fact)
            .filter(|element| element.accessible_value().as_deref() == Some(expected))
            .collect();
        assert_eq!(facts.len(), 1, "{fact}: {expected}");
    }

    details.invoke_accessible_default_action();
    assert_eq!(harness.count(Text, "VBIOS"), 0);
}

#[test]
fn every_track_shows_unknown_readings_as_unknown() {
    let harness = Harness::without_readings();
    harness.tick();

    for (_, reading, metric) in TRACKS {
        assert_eq!(value(&harness, reading), "unknown", "{reading}");
        assert_eq!(
            description(&harness, reading),
            "Not reported by this GPU",
            "{reading}"
        );
        for statistic in ["Maximum", "Average", "Minimum"] {
            let label = format!("{statistic} {metric}");
            assert_eq!(value(&harness, &label), "unknown", "{label}");
        }
    }
    assert_eq!(power_limit(&harness), "unknown");
    for detail in [
        "Enforced power limit",
        "Memory clock",
        "Graphics clock offset",
        "Memory clock offset",
        "Fan 0 speed",
        "Fan 1 speed",
    ] {
        assert_eq!(value(&harness, detail), "unknown", "{detail}");
        assert_eq!(
            description(&harness, detail),
            "Not reported by this GPU",
            "{detail}"
        );
    }
    // Nothing is drawn as if it were 0.
    assert_eq!(chart_parts(&harness, "Series"), 0);
    assert_eq!(chart_parts(&harness, "LimitLine"), 0);
    assert!(!harness.shows("0 W"));
}

#[test]
fn a_narrow_window_drops_max_avg_min_and_reserves_the_fan_curve_in_place_of_fan_speed() {
    let harness = Harness::new(SimulatedFixture::TunableGpu);
    harness.ticks(2);
    assert_eq!(harness.count(Groupbox, "Fan curve"), 0);

    for size in [HALF_1080P, MINIMUM_SIZE] {
        harness.resize(size);

        for (track, reading, metric) in TRACKS {
            harness.find(Groupbox, track);
            assert_ne!(value(&harness, reading), "unknown", "{reading}");
            for statistic in ["Maximum", "Average", "Minimum"] {
                let label = format!("{statistic} {metric}");
                assert_eq!(harness.count(Text, &label), 0, "{label} at {size:?}");
            }
        }
        harness.find(Groupbox, "Fan curve");
        // Every chart but fan speed's.
        assert_eq!(chart_parts(&harness, "Series"), 5, "at {size:?}");
        assert_eq!(value(&harness, "Fan 0 speed"), "31%");
    }

    harness.resize(DEFAULT_SIZE);
    assert_tracks_filled(&harness);
    assert_eq!(harness.count(Groupbox, "Fan curve"), 0);
}

#[test]
fn at_the_minimum_size_every_track_fits_without_scrolling() {
    // Without tuning access, so the "Tuning needs prismd" line takes room too.
    let harness = Harness::new(SimulatedFixture::TunableGpu);
    harness.ticks(2);

    harness.resize(MINIMUM_SIZE);

    let status_bar = harness.find(Button, "PRISMD · NOT RUNNING");
    let details = harness.find(Button, "Hardware details");
    let bottom = details.absolute_position().y + details.size().height;
    assert!(
        bottom <= status_bar.absolute_position().y,
        "the hardware details end at {bottom}, below the workspace"
    );
}

#[test]
fn the_window_asks_the_desktop_not_to_resize_it_below_its_minimum() {
    let harness = Harness::new(SimulatedFixture::TunableGpu);
    harness.ticks(2);

    for size in [DEFAULT_SIZE, HALF_1080P, MINIMUM_SIZE] {
        harness.resize(size);
        assert_eq!(
            support::minimum_size(harness.app.window().window()),
            slint::LogicalSize::new(MINIMUM_SIZE.0 as f32, MINIMUM_SIZE.1 as f32),
            "at {size:?}"
        );
    }
}
