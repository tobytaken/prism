//! View tests for switching profiles from the workspace header: the profile buttons with the
//! current one picked and the startup one's dot, rendered from prismd's events; one click applying
//! a profile; the question before discarding unapplied changes; and the one correction review.

mod support;

use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_gpu::{GpuTelemetryProvider, TuningOutcome, TuningOutcomeStatus};
use prism_protocol::{
    CorrectAndApplyProfileRequest, CurrentProfile, DaemonEvent, DaemonRequest, DaemonResponse,
    NumericRangeCorrection, ProfileApplicationResponse, ProfileIdRequest, ProfileRangeCorrection,
    ProfileSettingKey, ProfilesState, RangeBound,
};

use support::Harness;
use support::prismd::DaemonState;

use AccessibleRole::{Button, Groupbox, Spinbox, Text};

const GPU: &str = "GPU-5a1b0000-0000-4000-8000-000000000001";

/// The caller's Quiet and Boost profiles of the tunable GPU, with Quiet as its startup profile and
/// Boost current.
fn profiles() -> ProfilesState {
    serde_json::from_str(include_str!(
        "../../prism-protocol/tests/contracts/profiles-state.json"
    ))
    .unwrap()
}

/// Prism connected to prismd as `daemon` says, with one reading of the tunable GPU and its
/// profiles pushed.
fn connected_as(daemon: DaemonState) -> Harness {
    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, daemon);
    harness.prismd_tick(
        SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO).performance(),
    );
    push(&harness, profiles());
    harness
}

fn connected() -> Harness {
    connected_as(DaemonState::Running)
}

fn push(harness: &Harness, profiles: ProfilesState) {
    harness.prismd.push(DaemonEvent::ProfilesChanged(profiles));
    harness.app.receive_posted();
}

fn answer(harness: &Harness, response: DaemonResponse) {
    harness.prismd.answer(response);
    harness.app.receive_posted();
}

fn button(harness: &Harness, name: &str) -> ElementHandle {
    harness.find(Button, &format!("Apply profile {name}"))
}

fn description(element: &ElementHandle) -> String {
    element
        .accessible_description()
        .map(String::from)
        .unwrap_or_default()
}

/// The profile requests Prism sent to prismd, oldest first.
fn profile_requests(harness: &Harness) -> Vec<DaemonRequest> {
    harness
        .prismd
        .requests()
        .into_iter()
        .filter(|request| {
            matches!(
                request,
                DaemonRequest::ApplyProfileById(_) | DaemonRequest::CorrectAndApplyProfile(_)
            )
        })
        .collect()
}

fn apply_by_id(profile_id: &str) -> DaemonRequest {
    DaemonRequest::ApplyProfileById(ProfileIdRequest {
        profile_id: profile_id.into(),
    })
}

/// The labels of the buttons in the dialog titled `title`, in order.
fn actions(harness: &Harness, title: &str) -> Vec<String> {
    harness
        .find(Groupbox, title)
        .query_descendants()
        .match_accessible_role(Button)
        .find_all()
        .iter()
        .filter_map(|button| button.accessible_label().map(String::from))
        .collect()
}

#[test]
fn the_current_profile_and_the_startup_dot_render_from_events() {
    let harness = connected();

    assert_eq!(harness.count(Groupbox, "Profiles"), 1);
    assert_eq!(button(&harness, "Boost").accessible_checked(), Some(true));
    assert_eq!(button(&harness, "Quiet").accessible_checked(), Some(false));
    assert_eq!(description(&button(&harness, "Quiet")), "Startup profile");
    assert_eq!(description(&button(&harness, "Boost")), "");
    // A profile of a GPU that isn't here gets no button.
    assert_eq!(harness.count(Button, "Apply profile Old card"), 0);

    let mut changed = profiles();
    changed.current_profiles.insert(
        GPU.into(),
        CurrentProfile {
            id: "quiet".into(),
            name: "Quiet".into(),
        },
    );
    changed.startup_profiles.clear();
    push(&harness, changed);

    assert_eq!(button(&harness, "Quiet").accessible_checked(), Some(true));
    assert_eq!(button(&harness, "Boost").accessible_checked(), Some(false));
    assert_eq!(description(&button(&harness, "Quiet")), "");

    push(&harness, ProfilesState::default());
    assert_eq!(harness.count(Groupbox, "Profiles"), 0);
}

#[test]
fn clicking_a_profile_applies_it_at_once() {
    let harness = connected();

    button(&harness, "Quiet").invoke_accessible_default_action();

    assert_eq!(profile_requests(&harness), [apply_by_id("quiet")]);
    assert_eq!(description(&button(&harness, "Boost")), "Applying…");
    answer(
        &harness,
        DaemonResponse::ProfileApplication(ProfileApplicationResponse {
            rollback: None,
            cooling: None,
            power: Some(TuningOutcome {
                status: TuningOutcomeStatus::Succeeded,
                error: None,
            }),
            graphics: None,
            memory: None,
            corrections: Vec::new(),
        }),
    );
    assert_eq!(
        description(&harness.find(Groupbox, "Unapplied changes")),
        "Quiet applied"
    );
}

#[test]
fn clicking_a_profile_over_unapplied_changes_asks_first() {
    let harness = connected();
    harness
        .find(Spinbox, "Power limit")
        .set_accessible_value("450");
    let question = "Discard 1 unapplied change and apply Quiet?";

    button(&harness, "Quiet").invoke_accessible_default_action();

    assert_eq!(profile_requests(&harness), []);
    assert_eq!(
        actions(&harness, question),
        ["Discard and apply", "Keep editing"]
    );
    harness
        .find(Button, "Keep editing")
        .invoke_accessible_default_action();
    assert_eq!(harness.count(Groupbox, question), 0);
    assert_eq!(profile_requests(&harness), []);

    button(&harness, "Quiet").invoke_accessible_default_action();
    harness
        .find(Button, "Discard and apply")
        .invoke_accessible_default_action();
    assert_eq!(harness.count(Groupbox, question), 0);
    assert_eq!(profile_requests(&harness), [apply_by_id("quiet")]);
    assert!(harness.shows("No unapplied changes"));
}

#[test]
fn out_of_range_values_open_one_review_and_only_adjust_and_apply_sends_the_corrections() {
    let harness = connected();
    let corrections = vec![ProfileRangeCorrection::Numeric(NumericRangeCorrection {
        setting: ProfileSettingKey::PowerLimitWatts,
        saved_value: 650,
        proposed_value: 600,
        bound: RangeBound::Maximum,
    })];
    let needs_corrections = || {
        let unattempted = Some(TuningOutcome {
            status: TuningOutcomeStatus::Unattempted,
            error: Some("out of range".into()),
        });
        DaemonResponse::ProfileApplication(ProfileApplicationResponse {
            rollback: None,
            cooling: None,
            power: unattempted.clone(),
            graphics: unattempted.clone(),
            memory: unattempted,
            corrections: corrections.clone(),
        })
    };
    let review = "Adjust Quiet to apply it?";

    button(&harness, "Quiet").invoke_accessible_default_action();
    answer(&harness, needs_corrections());

    assert_eq!(
        actions(&harness, review),
        ["Cancel", "Adjust profile and apply"]
    );
    let rows: Vec<_> = harness
        .find(Groupbox, review)
        .query_descendants()
        .match_accessible_role(Text)
        .find_all()
        .iter()
        .filter_map(|row| {
            Some((
                row.accessible_label()?.to_string(),
                row.accessible_value()?.to_string(),
            ))
        })
        .collect();
    assert_eq!(
        rows,
        [("Power limit".to_owned(), "650 W → 600 W".to_owned())]
    );
    harness
        .find(Button, "Cancel")
        .invoke_accessible_default_action();
    assert_eq!(harness.count(Groupbox, review), 0);
    assert_eq!(profile_requests(&harness), [apply_by_id("quiet")]);

    button(&harness, "Quiet").invoke_accessible_default_action();
    answer(&harness, needs_corrections());
    harness
        .find(Button, "Adjust profile and apply")
        .invoke_accessible_default_action();
    assert_eq!(harness.count(Groupbox, review), 0);
    assert_eq!(
        profile_requests(&harness),
        [
            apply_by_id("quiet"),
            apply_by_id("quiet"),
            DaemonRequest::CorrectAndApplyProfile(CorrectAndApplyProfileRequest {
                profile_id: "quiet".into(),
                corrections,
            }),
        ]
    );
}

#[test]
fn without_tuning_access_the_header_shows_the_current_profile_read_only() {
    let harness = connected_as(DaemonState::ReadOnly);

    let profiles = harness.find(Groupbox, "Profiles");
    assert_eq!(profiles.accessible_read_only(), Some(true));
    assert_eq!(harness.count(Button, "Apply profile Quiet"), 0);
    let current = harness.find(Text, "Profiles");
    assert_eq!(current.accessible_value().as_deref(), Some("Boost"));
    assert_eq!(description(&current), "bob isn't in group wheel");
}
