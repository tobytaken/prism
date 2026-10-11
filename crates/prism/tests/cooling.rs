//! View tests for cooling in the staged loop: the Fans track's fan control mode and fixed speed,
//! applied with the reviewed fans, and fan curves unavailable with the reason.

mod support;

use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use prism_gpu::GpuTelemetryProvider;
use prism_gpu::cooling::{CoolingConfiguration, FanSetting};
use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_protocol::{ApplyGpuProfileRequest, DaemonRequest, ProfileSettings};
use slint::ComponentHandle;

use support::Harness;
use support::prismd::DaemonState;

use AccessibleRole::{Button, Groupbox, Spinbox, Text};

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
