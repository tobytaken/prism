//! View tests for cooling faults and external control: a suspended fan curve shows across the
//! Fans track, as PAUSED in the sidebar after a fault and as a problem in Activity; Retry, or
//! Resume, applies the kept curve again, and what's shown changes only with prismd's next push. A
//! fault notifies while the window doesn't have focus, and the click opens the GPU's Fans track.

mod support;

use std::time::Duration;

use i_slint_backend_testing::AccessibleRole;
use prism_gpu::GpuTelemetryProvider;
use prism_gpu::cooling::{
    CoolingConfiguration, CurveSuspension, FanRestoration, FanSetting, RestorationStatus,
    SuspensionKind,
};
use prism_gpu::fan_curve::FanCurvePoint;
use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_gpu::{GpuCommandError, GpuCommandErrorCode};
use prism_protocol::{
    DaemonEvent, DaemonRequest, DaemonResponse, GpuCoolingControl, RetryCoolingRequest,
};
use slint::ComponentHandle;

use support::Harness;
use support::prismd::DaemonState;

use AccessibleRole::{Button, Groupbox, ListItem, Text};

const GPU: &str = "GPU-5a1b0000-0000-4000-8000-000000000001";
const LABEL: &str = "GPU 0 · RTX 5090 · 01:00.0";
/// The tunable GPU's tuning-state revision in the shared snapshot.
const REVISION: &str = "4f1c2b9d0e8a7c35";
const REASON: &str = "The GPU's temperature is unavailable, so Prism stopped following its curve.";

/// Prism connected to prismd with one reading of the tunable GPU and its two fans.
fn connected() -> Harness {
    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, DaemonState::Running);
    harness.prismd_tick(
        SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO).performance(),
    );
    harness
}

fn curve() -> CoolingConfiguration {
    CoolingConfiguration::Shared {
        setting: FanSetting::Curve {
            points: vec![FanCurvePoint::new(45, 30), FanCurvePoint::new(85, 100)],
        },
    }
}

fn suspended(kind: SuspensionKind) -> GpuCoolingControl {
    GpuCoolingControl {
        gpu_id: GPU.into(),
        revision: 2,
        applied: Some(curve()),
        suspension: Some(CurveSuspension {
            kind,
            reason: REASON.into(),
            restoration: [0, 1]
                .map(|fan_index| FanRestoration {
                    fan_index,
                    status: RestorationStatus::Restored,
                    detail: None,
                })
                .into(),
        }),
        recovery: None,
        remembered_curve: Some(curve()),
    }
}

fn push(harness: &Harness, control: GpuCoolingControl) {
    harness
        .prismd
        .push(DaemonEvent::CoolingControlChanged(control));
    harness.app.receive_posted();
}

fn retries(harness: &Harness) -> Vec<RetryCoolingRequest> {
    harness
        .prismd
        .requests()
        .into_iter()
        .filter_map(|request| match request {
            DaemonRequest::RetryCooling(request) => Some(request),
            _ => None,
        })
        .collect()
}

fn retry_request() -> RetryCoolingRequest {
    RetryCoolingRequest {
        gpu_id: GPU.into(),
        expected_revision: REVISION.into(),
        fan_indexes: vec![0, 1],
    }
}

/// Whether the GPU's sidebar block says it's paused.
fn paused(harness: &Harness) -> bool {
    let block = harness.find(ListItem, LABEL);
    let paused = harness.count(Text, "PAUSED") == 1;
    let says = block
        .accessible_description()
        .is_some_and(|vitals| vitals.ends_with("Fan curve paused"));
    assert_eq!(paused, says, "the block says what it shows");
    paused
}

#[test]
fn a_cooling_fault_shows_across_the_fans_track_in_the_sidebar_and_in_activity() {
    let harness = connected();
    assert!(!paused(&harness));
    assert_eq!(harness.count(Groupbox, "Fan curve paused"), 0);

    push(&harness, suspended(SuspensionKind::Fault));

    let problem = harness.find(Groupbox, "Fan curve paused");
    let detail = problem.accessible_description().unwrap();
    assert!(detail.starts_with(REASON), "{detail}");
    assert!(
        detail.contains("Fan 0: automatic cooling restored."),
        "{detail}"
    );
    assert!(paused(&harness));
    assert_eq!(harness.count(Button, "1 PROBLEM"), 1);

    harness
        .find(Button, "1 PROBLEM")
        .invoke_accessible_default_action();

    assert_eq!(harness.count(ListItem, "Fan curve paused"), 1);
    // Activity's Retry and the Fans track's.
    assert_eq!(harness.count(Button, "Retry · Fan curve paused"), 2);
}

#[test]
fn retry_sends_the_apply_and_the_fault_clears_only_with_the_next_event() {
    let harness = connected();
    push(&harness, suspended(SuspensionKind::Fault));

    harness
        .find(Button, "Retry · Fan curve paused")
        .invoke_accessible_default_action();

    assert_eq!(retries(&harness), [retry_request()]);
    let retry = harness.find(Button, "Retry · Fan curve paused");
    assert_eq!(retry.accessible_enabled(), Some(false));
    assert_eq!(retry.accessible_description().as_deref(), Some("Retrying…"));

    harness
        .prismd
        .answer(DaemonResponse::Error(GpuCommandError::new(
            GpuCommandErrorCode::StaleRevision,
            "The GPU's tuning values changed since these changes were made.",
        )));
    harness.app.receive_posted();
    assert!(paused(&harness));
    assert_eq!(
        harness.find(Text, "Outcome").accessible_value().as_deref(),
        Some("Not retried: The GPU's tuning values changed since these changes were made.")
    );
    assert_eq!(
        harness
            .find(Button, "Retry · Fan curve paused")
            .accessible_enabled(),
        Some(true)
    );

    push(
        &harness,
        GpuCoolingControl {
            revision: 3,
            suspension: None,
            ..suspended(SuspensionKind::Fault)
        },
    );

    assert!(!paused(&harness));
    assert_eq!(harness.count(Groupbox, "Fan curve paused"), 0);
    assert_eq!(harness.count(Button, "NO PROBLEMS"), 1);
}

#[test]
fn an_external_control_change_shows_curve_control_suspended_with_resume() {
    let harness = connected();

    push(&harness, suspended(SuspensionKind::ExternalControl));

    let detail = harness
        .find(Groupbox, "Fan curve suspended")
        .accessible_description()
        .unwrap();
    assert!(detail.contains("Resume when you want Prism"), "{detail}");
    assert!(!paused(&harness));

    harness
        .find(Button, "Resume · Fan curve suspended")
        .invoke_accessible_default_action();

    assert_eq!(retries(&harness), [retry_request()]);
}

#[test]
fn a_cooling_fault_notifies_while_unfocused_and_the_click_opens_the_fans_track() {
    let harness = connected();
    harness.activation.set_focused(false);
    harness.ctrl_shift("g");
    assert_eq!(harness.count(Groupbox, "Fan curve paused"), 0);
    let window = harness.app.window().window();
    window.set_minimized(true);

    push(&harness, suspended(SuspensionKind::Fault));

    let [notification] = harness
        .notifier
        .sent()
        .try_into()
        .expect("one notification");
    assert_eq!(notification.summary, format!("Fan curve paused on {LABEL}"));

    harness
        .notifier
        .click(notification.id, Some("token-from-the-server"));
    harness.tick();

    assert!(!window.is_minimized());
    assert_eq!(
        harness.activation.tokens(),
        [Some("token-from-the-server".to_owned())]
    );
    assert_eq!(harness.count(Groupbox, "Fans"), 1);
    assert_eq!(harness.count(Groupbox, "Fan curve paused"), 1);
}

#[test]
fn the_click_scrolls_the_fans_track_into_view_in_a_short_window() {
    let harness = connected();
    harness.resize(support::MINIMUM_SIZE);
    harness.activation.set_focused(false);
    harness.ctrl_shift("g");
    push(&harness, suspended(SuspensionKind::Fault));
    let [notification] = harness.notifier.sent().try_into().unwrap();

    harness.notifier.click(notification.id, None);
    harness.tick();
    // The workspace shown anew scrolls once it's laid out, as drawing it does.
    assert_eq!(harness.count(Groupbox, "Fans"), 1);
    harness.tick();

    let retry = harness.find(Button, "Retry · Fan curve paused");
    let status_bar = harness.find(Button, "1 PROBLEM");
    let bottom = retry.absolute_position().y + retry.size().height;
    assert!(
        bottom <= status_bar.absolute_position().y,
        "Retry ends at {bottom}, under the status bar at {}",
        status_bar.absolute_position().y
    );
}
