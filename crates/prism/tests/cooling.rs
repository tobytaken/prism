//! View tests for cooling in the staged loop: the Fans track's fan control mode, fixed speed and
//! fan curve editor, applied with the reviewed fans, and fan curves unavailable with the reason.

mod support;

use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use prism_gpu::GpuTelemetryProvider;
use prism_gpu::cooling::{CoolingConfiguration, FanConfiguration, FanSetting};
use prism_gpu::fan_curve::FanCurvePoint;
use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_protocol::{
    ApplyGpuProfileRequest, DaemonEvent, DaemonRequest, GpuCoolingControl, ProfileSettings,
};
use slint::platform::{Key, PointerEventButton};
use slint::{ComponentHandle, LogicalPosition};

use support::Harness;
use support::prismd::DaemonState;

use AccessibleRole::{Button, Groupbox, Slider, Spinbox, Text};

const GPU: &str = "GPU-5a1b0000-0000-4000-8000-000000000001";
/// The tunable GPU's tuning-state revision in the shared snapshot.
const REVISION: &str = "4f1c2b9d0e8a7c35";

fn reading() -> prism_gpu::GpuPerformanceResponse {
    SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO).performance()
}

/// Prism connected to prismd as `daemon` says, with one reading of the tunable GPU as `change`
/// makes it; its two fans run at 30–100% under automatic cooling.
fn connected_reading(
    daemon: DaemonState,
    change: impl FnOnce(&mut prism_gpu::CoolingInfo),
) -> Harness {
    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, daemon);
    let mut sample = reading();
    change(&mut sample.gpus[0].cooling);
    harness.prismd_tick(sample);
    harness
}

fn connected() -> Harness {
    connected_reading(DaemonState::Running, |_| {})
}

fn fan_mode(harness: &Harness) -> ElementHandle {
    harness.find(Groupbox, "Fan control mode")
}

/// What the fan control mode says, with the cooling pending next to it.
fn fan_mode_says(harness: &Harness) -> String {
    fan_mode(harness)
        .accessible_description()
        .map(String::from)
        .unwrap_or_default()
}

/// The fan control mode's segment for `mode`, such as "Fixed speed".
fn segment(harness: &Harness, mode: &str) -> ElementHandle {
    fan_mode(harness)
        .query_descendants()
        .match_accessible_role(Button)
        .find_all()
        .into_iter()
        .find(|segment| segment.accessible_label().as_deref() == Some(mode))
        .unwrap_or_else(|| panic!("no {mode} segment"))
}

fn changes(harness: &Harness) -> String {
    harness
        .app
        .window()
        .global::<prism::AppState>()
        .get_workspace()
        .changes
        .summary
        .into()
}

fn applies(harness: &Harness) -> Vec<ApplyGpuProfileRequest> {
    harness
        .prismd
        .requests()
        .into_iter()
        .filter_map(|request| match request {
            DaemonRequest::ApplyProfile(request) => Some(request),
            _ => None,
        })
        .collect()
}

#[test]
fn switching_to_fixed_speed_and_applying_sends_one_apply_with_cooling_and_the_reviewed_fans() {
    let harness = connected();
    assert_eq!(
        segment(&harness, "Automatic").accessible_checked(),
        Some(true)
    );
    assert_eq!(harness.count(Spinbox, "Fixed fan speed"), 0);

    segment(&harness, "Fixed speed").invoke_accessible_default_action();
    harness
        .find(Spinbox, "Fixed fan speed")
        .set_accessible_value("55");

    assert_eq!(
        segment(&harness, "Fixed speed").accessible_checked(),
        Some(true)
    );
    assert_eq!(fan_mode_says(&harness), "→ fixed");
    assert_eq!(changes(&harness), "1 unapplied change");
    let speed = harness
        .find(Spinbox, "Fixed fan speed")
        .accessible_description()
        .map(String::from)
        .unwrap_or_default();
    assert!(speed.ends_with("→ 55%"), "{speed}");

    harness
        .find(Button, "Apply")
        .invoke_accessible_default_action();

    assert_eq!(
        applies(&harness),
        [ApplyGpuProfileRequest {
            gpu_id: GPU.into(),
            settings: ProfileSettings {
                cooling: Some(CoolingConfiguration::Shared {
                    setting: FanSetting::Fixed { speed_percent: 55 },
                }),
                ..ProfileSettings::default()
            },
            expected_revision: REVISION.into(),
            fan_indexes: Some(vec![0, 1]),
        }]
    );
}

#[test]
fn an_unknown_maximum_operating_temperature_disables_fan_curves_with_the_reason() {
    let reason = "This GPU or driver doesn't report a maximum operating temperature.";
    let harness = connected_reading(DaemonState::Running, |cooling| {
        cooling.max_operating_temperature_c = None;
        cooling.max_operating_temperature_reason = Some(reason.into());
    });

    let curve = segment(&harness, "Fan curve");
    assert_eq!(curve.accessible_enabled(), Some(false));
    let why = curve
        .accessible_description()
        .map(String::from)
        .unwrap_or_default();
    assert!(why.contains("maximum operating temperature"), "{why}");
    // It shows under the mode, too.
    assert_eq!(
        harness.find(Text, "Cooling").accessible_value().as_deref(),
        Some(why.as_str())
    );
    assert_eq!(
        segment(&harness, "Automatic").accessible_enabled(),
        Some(true)
    );

    curve.invoke_accessible_default_action();
    assert_eq!(changes(&harness), "No unapplied changes");
}

#[test]
fn without_tuning_access_the_fan_control_mode_is_read_only_with_the_reason() {
    let harness = connected_reading(DaemonState::ReadOnly, |_| {});

    assert_eq!(fan_mode_says(&harness), "bob isn't in group wheel");
    assert_eq!(
        fan_mode(&harness)
            .query_descendants()
            .match_accessible_role(Button)
            .find_all()
            .len(),
        0
    );
    assert_eq!(fan_mode(&harness).accessible_read_only(), Some(true));
    // Read-only, the mode in effect still takes focus and says itself.
    let mode = harness.find(Text, "Fan control mode");
    assert_eq!(mode.accessible_value().as_deref(), Some("Automatic"));
    assert_eq!(mode.accessible_read_only(), Some(true));
    assert_eq!(
        mode.accessible_description().as_deref(),
        Some("bob isn't in group wheel")
    );
}

/// The fan curve editor's point `number`, counting from 1.
fn curve_point(harness: &Harness, number: usize) -> ElementHandle {
    harness.find(Slider, &format!("Fan curve point {number}"))
}

/// What each of the editor's points says, such as "65% at 65 °C".
fn curve_points(harness: &Harness) -> Vec<String> {
    let editor = harness.find(Groupbox, "Fan curve");
    editor
        .query_descendants()
        .match_accessible_role(Slider)
        .find_all()
        .iter()
        .filter_map(ElementHandle::accessible_value)
        .map(String::from)
        .collect()
}

/// The tunable GPU's starter fan curve: T is 85 °C, 5 °C below its 90 °C maximum operating
/// temperature, and its fans run at 30–100%.
const STARTER: [&str; 4] = [
    "30% at 45 °C",
    "65% at 65 °C",
    "83% at 75 °C",
    "100% at 85 °C",
];

fn curve(points: &[(i64, i64)]) -> CoolingConfiguration {
    CoolingConfiguration::Shared {
        setting: FanSetting::Curve {
            points: points
                .iter()
                .map(|&(temperature, speed)| FanCurvePoint::new(temperature, speed))
                .collect(),
        },
    }
}

/// Prism connected to prismd with fan curve mode picked, which proposes the starter curve.
fn with_starter_curve() -> Harness {
    let harness = connected();
    segment(&harness, "Fan curve").invoke_accessible_default_action();
    harness
}

#[test]
fn the_starter_curve_appears_for_a_gpu_with_no_curve() {
    let harness = connected();
    assert_eq!(harness.count(Groupbox, "Fan curve"), 0);
    assert_eq!(harness.count(Button, "Fan curve editor"), 0);

    segment(&harness, "Fan curve").invoke_accessible_default_action();

    assert_eq!(curve_points(&harness), STARTER);
    assert_eq!(fan_mode_says(&harness), "→ curve");
    assert_eq!(changes(&harness), "1 unapplied change");
    // Every curve point stays within the speeds every fan supports.
    let point = curve_point(&harness, 1);
    assert_eq!(point.accessible_value_minimum(), Some(30.0));
    assert_eq!(point.accessible_value_maximum(), Some(100.0));
}

#[test]
fn dragging_a_point_edits_the_draft_and_applying_sends_the_complete_curve_in_one_apply() {
    let harness = with_starter_curve();
    let point = curve_point(&harness, 3);
    let (from, to) = (
        point.absolute_position(),
        LogicalPosition::new(
            point.absolute_position().x - 30.0,
            point.absolute_position().y + 20.0,
        ),
    );
    let center = |position: LogicalPosition| {
        LogicalPosition::new(
            position.x + point.size().width / 2.0,
            position.y + point.size().height / 2.0,
        )
    };

    point.mock_drag(center(to), PointerEventButton::Left);

    let moved = curve_point(&harness, 3);
    let at = moved.absolute_position();
    // The point lands where it's let go, to within a degree or a percent across the editor.
    assert!(
        (at.x - to.x).abs() <= 3.0 && (at.y - to.y).abs() <= 3.0,
        "dragged from {from:?} to {to:?}, landed at {at:?}"
    );
    let value = String::from(moved.accessible_value().unwrap());
    assert_ne!(value, STARTER[2]);
    assert_eq!(&curve_points(&harness)[..2], &STARTER[..2]);

    harness
        .find(Button, "Apply")
        .invoke_accessible_default_action();

    let sent = applies(&harness);
    assert_eq!(sent.len(), 1, "{sent:?}");
    let Some(CoolingConfiguration::Shared {
        setting: FanSetting::Curve { points },
    }) = &sent[0].settings.cooling
    else {
        panic!("Apply sends the curve: {sent:?}")
    };
    assert_eq!(points.len(), 4, "the complete curve");
    let moved = points[2];
    assert_eq!(
        format!("{}% at {} °C", moved.speed_percent, moved.temperature_c),
        value
    );
    assert!(
        moved.temperature_c < 75 && moved.speed_percent < 83,
        "{moved:?}"
    );
    assert_eq!(sent[0].fan_indexes, Some(vec![0, 1]));
}

#[test]
fn the_arrow_keys_move_the_focused_point_and_tab_moves_to_the_next() {
    let harness = with_starter_curve();
    // A click gives a point the keyboard.
    curve_point(&harness, 2).mock_single_click(PointerEventButton::Left);

    harness.press(&[], Key::RightArrow);
    harness.press(&[], Key::DownArrow);
    harness.press(&[Key::Shift], Key::DownArrow);

    assert_eq!(
        curve_point(&harness, 2).accessible_value().as_deref(),
        Some("59% at 66 °C")
    );

    harness.press(&[], Key::Tab);
    harness.press(&[], Key::LeftArrow);
    harness.press(&[], Key::UpArrow);

    assert_eq!(
        curve_points(&harness),
        [
            "30% at 45 °C",
            "59% at 66 °C",
            "84% at 74 °C",
            "100% at 85 °C"
        ]
    );
    assert_eq!(changes(&harness), "1 unapplied change");
}

#[test]
fn the_live_marker_follows_telemetry() {
    let harness = with_starter_curve();
    let marker_says = |harness: &Harness| {
        harness
            .find(Text, "Temperature and fan speed now")
            .accessible_value()
            .map(String::from)
            .unwrap_or_default()
    };
    let mut sample = reading();
    sample.gpus[0].telemetry.temperature_c = Some(50);
    sample.gpus[0].telemetry.fan_speed_percent = Some(40);
    harness.prismd_tick(sample.clone());
    assert_eq!(marker_says(&harness), "Now 50 °C, fans at 40%");
    let marker = || {
        ElementHandle::find_by_element_id(harness.app.window(), "FanCurve::marker")
            .next()
            .expect("the live marker is drawn")
            .absolute_position()
    };
    let before = marker();

    sample.gpus[0].telemetry.temperature_c = Some(70);
    sample.gpus[0].telemetry.fan_speed_percent = Some(80);
    harness.prismd_tick(sample);

    assert_eq!(marker_says(&harness), "Now 70 °C, fans at 80%");
    let after = marker();
    assert!(
        after.x > before.x && after.y < before.y,
        "{before:?} → {after:?}"
    );
}

#[test]
fn a_curve_that_reaches_full_speed_too_late_cant_be_applied_and_says_why() {
    let harness = with_starter_curve();
    curve_point(&harness, 4).mock_single_click(PointerEventButton::Left);

    harness.press(&[], Key::RightArrow);

    let problems = harness
        .find(Text, "Fan curve problems")
        .accessible_value()
        .map(String::from)
        .unwrap_or_default();
    assert!(problems.contains("must reach 100% by 85°C"), "{problems}");
    let apply = harness.find(Button, "Apply");
    assert_eq!(apply.accessible_enabled(), Some(false));
    apply.invoke_accessible_default_action();
    assert_eq!(applies(&harness), Vec::new());
}

#[test]
fn without_tuning_access_the_curve_in_effect_shows_read_only() {
    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, DaemonState::ReadOnly);
    let mut state = support::prismd::snapshot().tuning_states[0].clone();
    state.reported.cooling = Some(curve(&[(45, 30), (85, 100)]));
    state.revision = "after".into();
    harness.prismd.push(DaemonEvent::TuningStateChanged(state));
    harness.prismd_tick(reading());

    let point = curve_point(&harness, 1);
    assert_eq!(point.accessible_read_only(), Some(true));
    assert_eq!(
        point.accessible_description().as_deref(),
        Some("bob isn't in group wheel")
    );
    point.invoke_accessible_increment_action();
    assert_eq!(point.accessible_value().as_deref(), Some("30% at 45 °C"));
}

#[test]
fn the_curve_editor_collapses_to_the_fan_speed_history_and_opens_again() {
    let harness = with_starter_curve();
    let toggle = || harness.find(Button, "Fan curve editor");
    assert_eq!(toggle().accessible_expanded(), Some(true));
    harness.find(Groupbox, "Fan curve");

    toggle().invoke_accessible_default_action();

    assert_eq!(toggle().accessible_expanded(), Some(false));
    assert_eq!(harness.count(Groupbox, "Fan curve"), 0);

    toggle().invoke_accessible_default_action();
    harness.find(Groupbox, "Fan curve");
}

/// The segment labeled `label` in the row of segments labeled `row`, such as "Per fan" in "Fan
/// arrangement".
fn segment_in(harness: &Harness, row: &str, label: &str) -> ElementHandle {
    harness
        .find(Groupbox, row)
        .query_descendants()
        .match_accessible_role(Button)
        .find_all()
        .into_iter()
        .find(|segment| segment.accessible_label().as_deref() == Some(label))
        .unwrap_or_else(|| panic!("no {label} segment in {row}"))
}

#[test]
fn per_fan_settings_stage_and_apply_with_every_reviewed_fan() {
    let harness = connected();
    assert_eq!(
        segment_in(&harness, "Fan arrangement", "Shared").accessible_checked(),
        Some(true)
    );

    segment_in(&harness, "Fan arrangement", "Fan 1").invoke_accessible_default_action();
    segment(&harness, "Fixed speed").invoke_accessible_default_action();
    harness
        .find(Spinbox, "Fan 1 fixed speed")
        .set_accessible_value("60");

    assert_eq!(
        harness
            .find(Groupbox, "Fan arrangement")
            .accessible_description()
            .as_deref(),
        Some("→ per fan")
    );
    assert_eq!(changes(&harness), "1 unapplied change");
    // Fan 0 keeps the automatic cooling they shared.
    segment_in(&harness, "Fan arrangement", "Fan 0").invoke_accessible_default_action();
    assert_eq!(
        segment(&harness, "Automatic").accessible_checked(),
        Some(true)
    );

    harness
        .find(Button, "Apply")
        .invoke_accessible_default_action();

    assert_eq!(
        applies(&harness),
        [ApplyGpuProfileRequest {
            gpu_id: GPU.into(),
            settings: ProfileSettings {
                cooling: Some(CoolingConfiguration::PerFan {
                    fans: vec![
                        FanConfiguration {
                            fan_index: 0,
                            setting: FanSetting::Automatic {},
                        },
                        FanConfiguration {
                            fan_index: 1,
                            setting: FanSetting::Fixed { speed_percent: 60 },
                        },
                    ],
                }),
                ..ProfileSettings::default()
            },
            expected_revision: REVISION.into(),
            fan_indexes: Some(vec![0, 1]),
        }]
    );
}

#[test]
fn recalling_the_remembered_curve_creates_a_draft_that_reaches_prismd_only_with_apply() {
    let harness = connected();
    assert_eq!(harness.count(Button, "Recall remembered fan curve"), 0);
    let remembered = curve(&[(45, 30), (85, 100)]);
    harness
        .prismd
        .push(DaemonEvent::CoolingControlChanged(GpuCoolingControl {
            gpu_id: GPU.into(),
            revision: 2,
            remembered_curve: Some(remembered.clone()),
            ..GpuCoolingControl::default()
        }));
    harness.prismd_tick(reading());

    harness
        .find(Button, "Recall remembered fan curve")
        .invoke_accessible_default_action();

    assert_eq!(curve_points(&harness), ["30% at 45 °C", "100% at 85 °C"]);
    assert_eq!(fan_mode_says(&harness), "→ curve");
    assert_eq!(changes(&harness), "1 unapplied change");
    assert_eq!(applies(&harness), Vec::new());
    assert_eq!(harness.count(Button, "Recall remembered fan curve"), 0);

    harness
        .find(Button, "Apply")
        .invoke_accessible_default_action();
    let sent = applies(&harness);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].settings.cooling, Some(remembered));
}

#[test]
fn a_cooling_change_from_another_session_needs_review_before_apply_replaces_it() {
    let harness = connected();
    segment(&harness, "Fixed speed").invoke_accessible_default_action();
    harness
        .find(Spinbox, "Fixed fan speed")
        .set_accessible_value("55");

    let mut state = support::prismd::snapshot().tuning_states[0].clone();
    state.revision = "elsewhere".into();
    state.reported.cooling = Some(CoolingConfiguration::Shared {
        setting: FanSetting::Fixed { speed_percent: 70 },
    });
    harness.prismd.push(DaemonEvent::TuningStateChanged(state));
    harness.prismd_tick(reading());

    // The draft stays, and Apply becomes Review.
    assert_eq!(changes(&harness), "1 unapplied change");
    assert_eq!(harness.count(Button, "Apply"), 0);
    let strip = harness
        .find(Groupbox, "Unapplied changes")
        .accessible_description()
        .map(String::from)
        .unwrap_or_default();
    assert!(strip.contains("cooling changed"), "{strip}");

    harness
        .find(Button, "Review")
        .invoke_accessible_default_action();

    assert_eq!(applies(&harness), Vec::new());
    let change = harness.find(Text, "Cooling");
    assert_eq!(
        change.accessible_value().as_deref(),
        Some("unknown → fixed 70%")
    );
    assert_eq!(
        change.accessible_description().as_deref(),
        Some("Yours: fixed 55%")
    );

    harness
        .find(Button, "Apply")
        .invoke_accessible_default_action();

    let sent = applies(&harness);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].expected_revision, "elsewhere");
    assert_eq!(
        sent[0].settings.cooling,
        Some(CoolingConfiguration::Shared {
            setting: FanSetting::Fixed { speed_percent: 55 },
        })
    );
}
