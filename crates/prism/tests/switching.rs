//! View tests for automatic switching status and control: every switching state prismd pushes is
//! explained in the status bar and the workspace, Pause, Resume and Retry send their requests with
//! what's shown changing only with prismd's next push, and a failure pause shows as PAUSED in the
//! sidebar and as a problem in Activity, and notifies while the window doesn't have focus.

mod support;

use std::time::Duration;

use i_slint_backend_testing::AccessibleRole;
use prism_gpu::GpuTelemetryProvider;
use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_protocol::{
    AutomaticSwitchingStatus, DaemonCommandResponse, DaemonEvent, DaemonRequest, DaemonResponse,
    GpuSwitchingState, GpuSwitchingStatus, SetGpuSwitchingPausedRequest, StartupGpuRequest,
    SwitchingCondition,
};
use slint::ComponentHandle;

use support::Harness;
use support::prismd::DaemonState;

use AccessibleRole::{Button, Groupbox, List, ListItem, Text};

const GPU: &str = "GPU-5a1b0000-0000-4000-8000-000000000001";
const LABEL: &str = "GPU 0 · RTX 5090 · 01:00.0";

/// Prism connected to prismd, with tuning access as `daemon` gives it, and one reading of the
/// tunable GPU.
fn connected_to(daemon: DaemonState) -> Harness {
    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, daemon);
    harness.prismd_tick(
        SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO).performance(),
    );
    harness
}

fn connected() -> Harness {
    connected_to(DaemonState::Running)
}

/// The statuses in the shared contract, each about this GPU.
fn contract_statuses() -> Vec<AutomaticSwitchingStatus> {
    let statuses: Vec<AutomaticSwitchingStatus> = serde_json::from_str(include_str!(
        "../../prism-protocol/tests/contracts/automatic-switching-status.json"
    ))
    .unwrap();
    statuses
        .into_iter()
        .map(|mut status| {
            for gpu in &mut status.gpus {
                gpu.gpu_id = GPU.into();
            }
            status
        })
        .collect()
}

fn on_gpu(state: GpuSwitchingState) -> AutomaticSwitchingStatus {
    AutomaticSwitchingStatus {
        condition: SwitchingCondition::Ready {},
        gpus: vec![GpuSwitchingStatus {
            gpu_id: GPU.into(),
            state,
            application: None,
        }],
    }
}

fn active() -> GpuSwitchingState {
    GpuSwitchingState::Active {
        game: prism_protocol::GameId::Steam { app_id: 1_091_500 },
        game_name: "Cyberpunk 2077".into(),
        profile: prism_protocol::CurrentProfile {
            id: "quiet".into(),
            name: "Quiet".into(),
        },
    }
}

fn failed() -> GpuSwitchingState {
    GpuSwitchingState::Failed {
        message: "Power limit rejected.".into(),
    }
}

fn push(harness: &Harness, status: AutomaticSwitchingStatus) {
    harness
        .prismd
        .push(DaemonEvent::AutomaticSwitchingChanged(status));
    harness.app.receive_posted();
}

/// What the status bar says about automatic switching, if anything.
fn status_bar(harness: &Harness) -> Option<String> {
    (harness.count(Text, "Automatic switching") == 1).then(|| {
        harness
            .find(Text, "Automatic switching")
            .accessible_value()
            .unwrap()
            .into()
    })
}

/// What the workspace says about automatic switching, in short.
fn summary(harness: &Harness) -> String {
    harness
        .find(Text, "Automatic switching status")
        .accessible_value()
        .unwrap()
        .into()
}

/// What the workspace says about automatic switching in full.
fn explained(harness: &Harness) -> String {
    harness
        .find(Groupbox, "Automatic switching")
        .accessible_description()
        .unwrap()
        .into()
}

fn switching_requests(harness: &Harness) -> Vec<DaemonRequest> {
    harness
        .prismd
        .requests()
        .into_iter()
        .filter(|request| {
            matches!(
                request,
                DaemonRequest::SetGpuSwitchingPaused(_) | DaemonRequest::RetryAutomaticSwitching(_)
            )
        })
        .collect()
}

fn pause_request(paused: bool) -> DaemonRequest {
    DaemonRequest::SetGpuSwitchingPaused(SetGpuSwitchingPausedRequest {
        gpu_id: GPU.into(),
        paused,
    })
}

fn retry_request() -> DaemonRequest {
    DaemonRequest::RetryAutomaticSwitching(StartupGpuRequest { gpu_id: GPU.into() })
}

/// Whether the GPU's sidebar block says it's paused.
fn paused(harness: &Harness) -> bool {
    let block = harness.find(ListItem, LABEL);
    let paused = harness.count(Text, "PAUSED") == 1;
    let says = block
        .accessible_description()
        .is_some_and(|vitals| vitals.ends_with("Automatic switching failed"));
    assert_eq!(paused, says, "the block says what it shows");
    paused
}

/// The palette's commands matching `query`, by title.
fn palette(harness: &Harness, query: &str) -> Vec<String> {
    harness.ctrl("k");
    harness.type_text(query);
    let titles = harness
        .find(List, "Commands")
        .query_descendants()
        .match_accessible_role(ListItem)
        .find_all()
        .iter()
        .filter_map(|row| row.accessible_label().map(String::from))
        .collect();
    harness.press(&[], slint::platform::Key::Escape);
    titles
}

#[test]
fn every_switching_state_is_explained_in_the_status_bar_and_the_workspace() {
    let expected = [
        "Paused",
        "Overridden",
        "Overridden",
        "Dota 2 → Gaming",
        "Another user's game",
        "Failed",
        "Blocked",
        "game.exe → Quiet",
        "Needs recovery",
        "Waiting for edits → Gaming",
        "Needs recovery",
    ];
    let harness = connected();
    let statuses = contract_statuses();
    assert_eq!(statuses.len(), expected.len());

    for (status, expected) in statuses.into_iter().zip(expected) {
        push(&harness, status.clone());

        assert_eq!(summary(&harness), expected, "{status:?}");
        assert_eq!(
            status_bar(&harness),
            Some(format!("AUTO · {}", expected.to_uppercase())),
            "{status:?}"
        );
        let explained = explained(&harness);
        assert!(
            explained.starts_with(&format!("{expected} · "))
                && explained.len() > expected.len() + 3,
            "{explained}"
        );
    }
}

#[test]
fn without_a_session_automatic_switching_isnt_shown() {
    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, DaemonState::NotRunning);
    harness.tick();

    assert_eq!(status_bar(&harness), None);
    assert_eq!(harness.count(Groupbox, "Automatic switching"), 0);
}

#[test]
fn pause_and_resume_send_their_requests_and_the_state_changes_only_with_prismds_push() {
    let harness = connected();
    push(&harness, on_gpu(active()));

    harness
        .find(Button, "Pause automatic switching")
        .invoke_accessible_default_action();

    assert_eq!(switching_requests(&harness), [pause_request(true)]);
    let pause = harness.find(Button, "Pause automatic switching");
    assert_eq!(pause.accessible_enabled(), Some(false));
    assert_eq!(pause.accessible_description().as_deref(), Some("Pausing…"));
    harness
        .prismd
        .answer(DaemonResponse::Command(DaemonCommandResponse {
            success: true,
            error: None,
        }));
    harness.app.receive_posted();
    assert_eq!(summary(&harness), "Cyberpunk 2077 → Quiet");

    push(&harness, on_gpu(GpuSwitchingState::Paused {}));

    assert_eq!(summary(&harness), "Paused");
    assert_eq!(status_bar(&harness).as_deref(), Some("AUTO · PAUSED"));
    harness
        .find(Button, "Resume automatic switching")
        .invoke_accessible_default_action();
    assert_eq!(
        switching_requests(&harness),
        [pause_request(true), pause_request(false)]
    );
}

#[test]
fn pause_is_read_only_without_tuning_access() {
    let harness = connected_to(DaemonState::ReadOnly);
    push(&harness, on_gpu(active()));

    let pause = harness.find(Button, "Pause automatic switching");
    assert_eq!(pause.accessible_enabled(), Some(false));
    assert!(
        pause
            .accessible_description()
            .is_some_and(|reason| reason.starts_with("Needs tuning access")),
        "{:?}",
        pause.accessible_description()
    );
    pause.invoke_accessible_default_action();
    assert_eq!(switching_requests(&harness), []);
}

#[test]
fn a_failure_pause_shows_inline_in_the_sidebar_and_in_activity_with_retry() {
    let harness = connected();
    assert!(!paused(&harness));

    push(&harness, on_gpu(failed()));

    assert_eq!(summary(&harness), "Failed");
    assert!(paused(&harness));
    assert_eq!(harness.count(Button, "1 PROBLEM"), 1);
    harness
        .find(Button, "1 PROBLEM")
        .invoke_accessible_default_action();
    assert_eq!(harness.count(ListItem, "Automatic switching failed"), 1);

    harness
        .find(Button, "Retry · Automatic switching failed")
        .invoke_accessible_default_action();

    assert_eq!(switching_requests(&harness), [retry_request()]);
    let retry = harness.find(Button, "Retry automatic switching");
    assert_eq!(retry.accessible_enabled(), Some(false));
    assert_eq!(retry.accessible_description().as_deref(), Some("Retrying…"));
    // Its answer is a status, but only prismd's push changes what's shown.
    harness
        .prismd
        .answer(DaemonResponse::AutomaticSwitchingStatus(on_gpu(active())));
    harness.app.receive_posted();
    assert!(paused(&harness));

    push(&harness, on_gpu(active()));

    assert!(!paused(&harness));
    assert_eq!(harness.count(Button, "Retry automatic switching"), 0);
    assert_eq!(harness.count(Button, "NO PROBLEMS"), 1);
}

#[test]
fn the_workspace_retry_sends_the_retry() {
    let harness = connected();
    push(&harness, on_gpu(failed()));

    harness
        .find(Button, "Retry automatic switching")
        .invoke_accessible_default_action();

    assert_eq!(switching_requests(&harness), [retry_request()]);
}

#[test]
fn a_failure_pause_notifies_while_unfocused_and_the_click_opens_the_gpu() {
    let harness = connected();
    harness.activation.set_focused(false);
    harness.ctrl_shift("g");
    assert_eq!(harness.count(Groupbox, "Automatic switching"), 0);
    let window = harness.app.window().window();
    window.set_minimized(true);

    push(&harness, on_gpu(failed()));

    let [notification] = harness
        .notifier
        .sent()
        .try_into()
        .expect("one notification");
    assert_eq!(
        notification.summary,
        format!("Automatic switching failed on {LABEL}")
    );

    harness
        .notifier
        .click(notification.id, Some("token-from-the-server"));
    harness.tick();

    assert!(!window.is_minimized());
    assert_eq!(
        harness.activation.tokens(),
        [Some("token-from-the-server".to_owned())]
    );
    assert_eq!(summary(&harness), "Failed");
}

#[test]
fn the_palette_lists_pause_resume_and_retry_only_when_they_apply() {
    let harness = connected();
    let pause = format!("Pause automatic switching on {LABEL}");
    let resume = format!("Resume automatic switching on {LABEL}");
    let retry = format!("Retry {LABEL}");

    push(&harness, on_gpu(active()));
    assert_eq!(palette(&harness, "switching on gpu"), [pause.as_str()]);

    push(&harness, on_gpu(GpuSwitchingState::Paused {}));
    assert_eq!(palette(&harness, "switching on gpu"), [resume]);

    push(&harness, on_gpu(failed()));
    assert_eq!(palette(&harness, "switching on gpu"), [pause]);
    assert_eq!(palette(&harness, "retry"), [retry.as_str()]);

    harness.ctrl("k");
    harness.type_text(&retry);
    harness.press(&[], slint::platform::Key::Return);
    assert_eq!(switching_requests(&harness), [retry_request()]);
}
