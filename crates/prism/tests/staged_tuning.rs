//! View tests for staged tuning: staging a power limit by dragging its line or with its stepper,
//! the change strip with Apply and Discard, and EDIT HOLD.

mod support;

use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_gpu::{GpuInventoryProvider, GpuTelemetryProvider, TuningOutcome, TuningOutcomeStatus};
use prism_protocol::{
    ApplyGpuProfileRequest, DaemonEvent, DaemonRequest, DaemonResponse, ProfileApplicationResponse,
    ProfileSetting, ProfileSettings, SetEditHoldsRequest,
};
use prism_ui::{Content, TrackKind};
use slint::platform::{Key, PointerEventButton};
use slint::{ComponentHandle, LogicalPosition};

use support::Harness;
use support::prismd::DaemonState;

use AccessibleRole::{Button, Groupbox, ListItem, Spinbox};

const GPU: &str = "GPU-5a1b0000-0000-4000-8000-000000000001";
/// The tunable GPU's tuning-state revision in the shared snapshot.
const REVISION: &str = "4f1c2b9d0e8a7c35";

fn reading(fixture: SimulatedFixture) -> prism_gpu::GpuPerformanceResponse {
    SimulatedGpus::new(fixture, || Duration::ZERO).performance()
}

/// Prism connected to prismd as `daemon` says, with one reading of the tunable GPU, whose power
/// limit is 575 W in a 400–600 W range.
fn connected_as(daemon: DaemonState) -> Harness {
    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, daemon);
    harness.prismd_tick(reading(SimulatedFixture::TunableGpu));
    harness
}

fn connected() -> Harness {
    connected_as(DaemonState::Running)
}

fn power_limit(harness: &Harness) -> ElementHandle {
    harness.find(Spinbox, "Power limit")
}

/// What the power limit's stepper says, with the pending value next to it.
fn stepper_says(harness: &Harness) -> String {
    power_limit(harness)
        .accessible_description()
        .map(String::from)
        .unwrap_or_default()
}

/// The change strip's summary, as shown.
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

fn edit_hold(harness: &Harness) -> bool {
    harness.shows("EDIT HOLD")
}

/// The power limit line's handle, which a drag moves.
fn limit_handle(harness: &Harness) -> Option<ElementHandle> {
    ElementHandle::find_by_element_id(harness.app.window(), "ChartPane::limit-handle").next()
}

/// The power chart's top edge and height in the window, from its grid.
fn power_chart_bounds(harness: &Harness, handle: &ElementHandle) -> (f32, f32) {
    let center = handle.absolute_position().y + handle.size().height / 2.0;
    ElementHandle::find_by_element_type_name(harness.app.window(), "Grid")
        .map(|grid| (grid.absolute_position().y, grid.size().height))
        .find(|(top, height)| (*top..=top + height).contains(&center))
        .expect("the handle lies on a chart")
}

/// The Applies Prism sent to prismd.
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

/// The edit holds Prism sent to prismd, oldest first.
fn edit_holds(harness: &Harness) -> Vec<Vec<String>> {
    harness
        .prismd
        .requests()
        .into_iter()
        .filter_map(|request| match request {
            DaemonRequest::SetEditHolds(SetEditHoldsRequest { gpu_ids }) => Some(gpu_ids),
            _ => None,
        })
        .collect()
}

fn watts(value: i64) -> ProfileSettings {
    ProfileSettings {
        power_limit_watts: Some(ProfileSetting::Value { value }),
        ..ProfileSettings::default()
    }
}

fn power_applied(status: TuningOutcomeStatus, error: Option<&str>) -> DaemonResponse {
    DaemonResponse::ProfileApplication(ProfileApplicationResponse {
        rollback: None,
        cooling: None,
        power: Some(TuningOutcome {
            status,
            error: error.map(Into::into),
        }),
        graphics: None,
        memory: None,
        corrections: Vec::new(),
    })
}

#[test]
fn dragging_the_limit_line_stages_a_pending_value_on_the_control_and_the_chart() {
    let harness = connected();
    assert_eq!(changes(&harness), "No unapplied changes");
    assert!(!edit_hold(&harness));
    let handle = limit_handle(&harness).expect("the power limit's line can be dragged");
    let (top, height) = power_chart_bounds(&harness, &handle);
    let x = handle.absolute_position().x + handle.size().width / 2.0;

    // Past the bottom of the chart, the limit stops at the lowest the GPU allows.
    handle.mock_drag(
        LogicalPosition::new(x, top + 2.0 * height),
        PointerEventButton::Left,
    );

    assert_eq!(stepper_says(&harness), "In effect 575 W, → 400 W");
    assert_eq!(changes(&harness), "1 unapplied change");
    assert!(edit_hold(&harness));
    assert_eq!(edit_holds(&harness), [vec![GPU.to_owned()]]);
    // The configured, enforced and pending limits.
    assert_eq!(
        ElementHandle::find_by_element_type_name(harness.app.window(), "LimitLine").count(),
        3
    );

    // Let go where 500 W is drawn, the limit lands there, to the pixel.
    let chart = match harness.app.core().content() {
        Content::Gpu(workspace) => workspace.track(TrackKind::Power).charts[0].chart.clone(),
        other => panic!("expected the GPU workspace, got {other:?}"),
    };
    let handle = limit_handle(&harness).unwrap();
    handle.mock_drag(
        LogicalPosition::new(x, top + height * chart.y(500)),
        PointerEventButton::Left,
    );
    let watts_per_pixel = (chart.max - chart.min) as f32 / height;
    let staged: f32 = power_limit(&harness)
        .accessible_value()
        .unwrap()
        .trim_end_matches(" W")
        .parse()
        .unwrap();
    assert!(
        (staged - 500.0).abs() <= watts_per_pixel.ceil(),
        "staged {staged} W"
    );
}

#[test]
fn apply_sends_one_apply_profile_with_the_drafts_revision_and_clears_the_draft() {
    let harness = connected();
    power_limit(&harness).invoke_accessible_decrement_action();
    assert_eq!(changes(&harness), "1 unapplied change");

    harness
        .find(Button, "Apply")
        .invoke_accessible_default_action();

    assert_eq!(
        applies(&harness),
        [ApplyGpuProfileRequest {
            gpu_id: GPU.into(),
            settings: watts(574),
            expected_revision: REVISION.into(),
            fan_indexes: None,
        }]
    );
    harness
        .prismd
        .answer(power_applied(TuningOutcomeStatus::Succeeded, None));
    harness.app.receive_posted();
    assert_eq!(changes(&harness), "No unapplied changes");
    assert!(!edit_hold(&harness));
    assert_eq!(edit_holds(&harness), [vec![GPU.to_owned()], vec![]]);
}

#[test]
fn ctrl_enter_applies_and_a_failed_setting_stays_pending_with_why() {
    let harness = connected();
    power_limit(&harness).set_accessible_value("450");

    harness.press(&[Key::Control], "\n");

    assert_eq!(applies(&harness).len(), 1);
    harness.prismd.answer(power_applied(
        TuningOutcomeStatus::Failed,
        Some("Insufficient permissions"),
    ));
    harness.app.receive_posted();
    assert_eq!(changes(&harness), "1 unapplied change");
    assert_eq!(stepper_says(&harness), "In effect 575 W, → 450 W");
    assert_eq!(
        harness
            .find(Groupbox, "Unapplied changes")
            .accessible_description()
            .as_deref(),
        Some("Power limit: Insufficient permissions")
    );
}

#[test]
fn discard_clears_the_draft_and_releases_its_edit_hold() {
    let harness = connected();
    power_limit(&harness).set_accessible_value("450");
    assert!(edit_hold(&harness));

    harness.ctrl_shift("\u{8}");

    assert_eq!(changes(&harness), "No unapplied changes");
    assert_eq!(stepper_says(&harness), "");
    assert!(!edit_hold(&harness));
    assert_eq!(edit_holds(&harness), [vec![GPU.to_owned()], vec![]]);
    assert!(applies(&harness).is_empty());
}

#[test]
fn drafts_survive_switching_gpus_and_edit_hold_shows_while_any_exists() {
    let harness = connected();
    let two = SimulatedGpus::new(SimulatedFixture::TwoIdenticalGpus, || Duration::ZERO);
    harness
        .prismd
        .push(DaemonEvent::InventoryChanged(two.inventory()));
    harness.prismd_tick(two.performance());
    let gpus: Vec<_> = harness
        .app
        .core()
        .sidebar()
        .gpus
        .iter()
        .map(|gpu| gpu.label.clone())
        .collect();
    harness
        .find(ListItem, &gpus[0])
        .invoke_accessible_default_action();
    // These GPUs' power limits are 450 W.
    power_limit(&harness).set_accessible_value("400");

    harness
        .find(ListItem, &gpus[1])
        .invoke_accessible_default_action();
    assert_eq!(changes(&harness), "No unapplied changes");
    assert!(edit_hold(&harness));
    harness
        .find(ListItem, "Games")
        .invoke_accessible_default_action();
    assert!(edit_hold(&harness));

    harness
        .find(ListItem, &gpus[0])
        .invoke_accessible_default_action();
    assert_eq!(changes(&harness), "1 unapplied change");
    assert_eq!(stepper_says(&harness), "In effect 450 W, → 400 W");
}

#[test]
fn without_tuning_access_the_power_limit_is_read_only_with_the_reason() {
    let harness = connected_as(DaemonState::ReadOnly);

    let stepper = power_limit(&harness);
    assert_eq!(stepper.accessible_read_only(), Some(true));
    assert_eq!(
        stepper.accessible_description().as_deref(),
        Some("bob isn't in group wheel")
    );
    assert!(limit_handle(&harness).is_none());
    stepper.invoke_accessible_decrement_action();
    assert_eq!(changes(&harness), "No unapplied changes");
    assert!(edit_holds(&harness).is_empty());
}
