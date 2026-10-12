//! View tests for Activity and the problem indicator: the indicator counts current problems and
//! opens Activity, which lists them first, then startup outcomes and recent events from prismd's
//! snapshot and events, each with its action; Escape closes it.

mod support;

use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use prism_gpu::GpuTelemetryProvider;
use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_protocol::{
    ActivityEntry, DaemonEvent, DaemonRequest, DaemonSnapshot, StartupGpuRequest, StartupState,
    TuningStoreHealth, TuningStoreProblem,
};

use support::Harness;
use support::prismd::{DaemonState, snapshot};

use AccessibleRole::{Button, Groupbox, ListItem};

const GPU: &str = "GPU-5a1b0000-0000-4000-8000-000000000001";

/// What prismd did this boot: startup applied Quiet in part, then Boost was applied, Dota 2
/// started with Quiet, and its pre-game values were restored.
fn entries() -> Vec<ActivityEntry> {
    serde_json::from_str(include_str!(
        "../../prism-protocol/tests/contracts/activity-entries.json"
    ))
    .unwrap()
}

/// This boot's startup outcome on the tunable GPU: Quiet applied in part.
fn startup() -> StartupState {
    let mut state: StartupState = serde_json::from_str(include_str!(
        "../../prism-protocol/tests/contracts/startup-state.json"
    ))
    .unwrap();
    for outcome in &mut state.outcomes {
        outcome.gpu_id = GPU.into();
    }
    state
}

/// Prism connected to a prismd whose session starts from `snapshot`, with one reading of the
/// tunable GPU.
fn connected_from(snapshot: DaemonSnapshot) -> Harness {
    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, DaemonState::NotRunning);
    harness.prismd.set_snapshot(snapshot);
    harness.prismd.start_daemon();
    harness.app.receive_posted();
    harness.prismd_tick(
        SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO).performance(),
    );
    harness
}

fn push(harness: &Harness, event: DaemonEvent) {
    harness.prismd.push(event);
    harness.app.receive_posted();
}

/// The status bar's problem indicator, whatever it says.
fn indicator(harness: &Harness) -> ElementHandle {
    ["NO PROBLEMS", "1 PROBLEM", "2 PROBLEMS"]
        .into_iter()
        .flat_map(|label| ElementHandle::find_by_accessible_label(harness.app.window(), label))
        .find(|element| element.accessible_role() == Some(Button))
        .expect("the status bar has a problem indicator")
}

fn activity_open(harness: &Harness) -> bool {
    harness.count(Groupbox, "Activity") == 1
}

#[test]
fn the_indicator_counts_problems_and_opens_activity_which_escape_closes() {
    let harness = connected_from(snapshot());
    assert_eq!(
        indicator(&harness).accessible_label().unwrap(),
        "NO PROBLEMS"
    );

    push(
        &harness,
        DaemonEvent::StoreHealthChanged(TuningStoreHealth::Unhealthy {
            problem: TuningStoreProblem::Unreadable,
            path: "/var/lib/prism/tuning-store.json".into(),
            detail: "expected value at line 1 column 1".into(),
        }),
    );
    let indicator = indicator(&harness);
    assert_eq!(indicator.accessible_label().unwrap(), "1 PROBLEM");

    indicator.invoke_accessible_default_action();
    assert!(activity_open(&harness));
    let problem = harness.find(ListItem, "Prism's tuning store can't be used");
    assert!(
        problem
            .accessible_description()
            .unwrap()
            .contains("Profile applications and live tuning changes keep working.")
    );

    harness.press(&[], "\u{1b}");
    assert!(!activity_open(&harness));

    harness.ctrl_shift("a");
    assert!(activity_open(&harness));
    harness.ctrl_shift("a");
    assert!(!activity_open(&harness));
}

#[test]
fn activity_lists_what_prismd_did_while_prism_was_closed() {
    let harness = connected_from(DaemonSnapshot {
        activity: entries(),
        ..snapshot()
    });
    harness.ctrl_shift("a");

    let restored = harness.find(ListItem, "Pre-game values restored");
    assert_eq!(
        restored.accessible_description().unwrap(),
        "1 min ago · GPU 0 · RTX 5090 · 01:00.0 · Power limit restored"
    );
    assert_eq!(harness.count(ListItem, "Dota 2 started · Quiet applied"), 1);
    assert_eq!(harness.count(ListItem, "Boost applied"), 1);

    harness
        .find(Button, "Open · Boost applied")
        .invoke_accessible_default_action();
    assert!(!activity_open(&harness));
}

#[test]
fn a_startup_outcome_shows_per_setting_and_is_dismissed_through_prismd() {
    let harness = connected_from(DaemonSnapshot {
        startup: startup(),
        activity: entries()[..1].to_vec(),
        ..snapshot()
    });
    assert_eq!(indicator(&harness).accessible_label().unwrap(), "1 PROBLEM");
    harness.ctrl_shift("a");
    assert_eq!(harness.count(ListItem, "Startup applied Quiet in part"), 2);

    harness
        .find(Button, "Dismiss · Startup applied Quiet in part")
        .invoke_accessible_default_action();
    assert_eq!(
        harness.prismd.requests().last(),
        Some(&DaemonRequest::DismissStartupOutcome(StartupGpuRequest {
            gpu_id: GPU.into()
        }))
    );

    push(
        &harness,
        DaemonEvent::StartupStateChanged(StartupState {
            outcomes: vec![],
            ..startup()
        }),
    );
    assert_eq!(
        indicator(&harness).accessible_label().unwrap(),
        "NO PROBLEMS"
    );
    // What happened stays among recent events.
    assert_eq!(harness.count(ListItem, "Startup applied Quiet in part"), 1);
}
