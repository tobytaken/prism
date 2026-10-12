//! Automatic switching status and control (ADRs 0003 and 0004): the status bar says what drives
//! automatic switching on the selected GPU, or why it's waiting, paused, overridden, blocked or
//! failed, and the workspace says so for its GPU with Pause, Resume and, on a failure pause,
//! Retry. A failure pause also shows as PAUSED in the sidebar, as a problem in Activity and in a
//! desktop notification. What's shown changes only with prismd's pushes.

use std::collections::BTreeMap;
use std::time::Duration;

use prism_gpu::cooling::{CoolingConfiguration, CurveSuspension, FanSetting, SuspensionKind};
use prism_gpu::fan_curve::FanCurvePoint;
use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_gpu::{GpuCommandError, GpuCommandErrorCode, GpuTelemetryProvider};
use prism_protocol::{
    AutomaticSwitchingStatus, CurrentProfile, DaemonCommandResponse, DaemonEvent, DaemonRequest,
    DaemonResponse, DaemonSnapshot, GameEntry, GameId, GamesState, GpuCoolingControl,
    GpuSwitchingState, GpuSwitchingStatus, ProtocolVersion, RunningGame, RunningGamesState,
    SessionMessage, SetGpuSwitchingPausedRequest, StartupGpuRequest, SwitchingCondition, Welcome,
};
use prism_ui::{
    ActivityAction, ActivityItem, AppearanceMode, CommandId, Content, Core, Effect, Input, Intent,
    Notification, Section, SessionFailure, SwitchingStatus,
};

const GPU: &str = "GPU-5a1b0000-0000-4000-8000-000000000001";
const LABEL: &str = "GPU 0 · RTX 5090 · 01:00.0";
const CYBERPUNK: GameId = GameId::Steam { app_id: 1_091_500 };

fn snapshot() -> DaemonSnapshot {
    let message: SessionMessage = serde_json::from_str(include_str!(
        "../../prism-protocol/tests/contracts/daemon-snapshot.json"
    ))
    .unwrap();
    let SessionMessage::Snapshot { snapshot } = message else {
        panic!("the contract holds a snapshot")
    };
    snapshot
}

fn welcome(can_tune: bool) -> Welcome {
    Welcome {
        protocol: ProtocolVersion::CURRENT,
        prismd_version: "0.1.0".into(),
        can_tune,
        reason: (!can_tune).then(|| "bob isn't in group wheel".into()),
        authorization: None,
    }
}

/// Prism connected to prismd from `snapshot`, with tuning access as `can_tune` says and its window
/// without focus, showing the tunable GPU.
fn connected_from(snapshot: DaemonSnapshot, can_tune: bool) -> (Core, Vec<Effect>) {
    let mut core = Core::default();
    core.handle(Input::Launched {
        appearance: AppearanceMode::System,
        started_unix_ms: 0,
    });
    let effects = core.handle(Input::PrismdConnected {
        welcome: welcome(can_tune),
        snapshot,
        socket: "/run/prism/prismd.sock".into(),
    });
    core.handle(Input::PrismdEvent {
        event: DaemonEvent::TelemetrySampled(
            SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO).performance(),
        ),
        received_at: Duration::from_secs(1),
    });
    (core, effects)
}

fn connected_with(can_tune: bool) -> Core {
    connected_from(snapshot(), can_tune).0
}

fn connected() -> Core {
    connected_with(true)
}

fn profile(name: &str) -> CurrentProfile {
    CurrentProfile {
        id: name.to_lowercase(),
        name: name.into(),
    }
}

fn ready(gpus: Vec<GpuSwitchingStatus>) -> AutomaticSwitchingStatus {
    AutomaticSwitchingStatus {
        condition: SwitchingCondition::Ready {},
        gpus,
    }
}

fn gpu(state: GpuSwitchingState) -> GpuSwitchingStatus {
    GpuSwitchingStatus {
        gpu_id: GPU.into(),
        state,
        application: None,
    }
}

/// The GPU in `state`, with the caller's automatic switching on.
fn on_gpu(state: GpuSwitchingState) -> AutomaticSwitchingStatus {
    ready(vec![gpu(state)])
}

fn active() -> GpuSwitchingState {
    GpuSwitchingState::Active {
        game: CYBERPUNK,
        game_name: "Cyberpunk 2077".into(),
        profile: profile("Quiet"),
    }
}

fn failed() -> GpuSwitchingState {
    GpuSwitchingState::Failed {
        message: "Power limit rejected.\nMemory offset rejected.".into(),
    }
}

fn push_event(core: &mut Core, event: DaemonEvent) -> Vec<Effect> {
    core.handle(Input::PrismdEvent {
        event,
        received_at: Duration::from_secs(2),
    })
}

fn push(core: &mut Core, status: AutomaticSwitchingStatus) -> Vec<Effect> {
    push_event(core, DaemonEvent::AutomaticSwitchingChanged(status))
}

fn intent(core: &mut Core, intent: Intent) -> Vec<Effect> {
    core.handle(Input::Intent(intent))
}

/// The workspace's automatic switching status, if it shows one.
fn status(core: &Core) -> Option<SwitchingStatus> {
    let Content::Gpu(workspace) = core.content() else {
        panic!("expected the GPU workspace")
    };
    workspace.switching
}

fn shown(core: &Core) -> SwitchingStatus {
    status(core).expect("the workspace shows automatic switching")
}

fn status_bar(core: &Core) -> Option<String> {
    core.status_bar().switching
}

fn paused(core: &Core) -> bool {
    core.sidebar().gpus[0].paused()
}

/// The problems Activity lists.
fn problems(core: &mut Core) -> Vec<ActivityItem> {
    intent(core, Intent::OpenActivity);
    let problems = core.activity().expect("Activity is open").problems;
    intent(core, Intent::Dismiss);
    problems
}

/// The requests among `effects` sent for an answer.
fn commands(effects: &[Effect]) -> Vec<(CommandId, DaemonRequest)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::CommandPrismd { id, request } => Some((*id, request.clone())),
            _ => None,
        })
        .collect()
}

fn requests(effects: &[Effect]) -> Vec<DaemonRequest> {
    commands(effects)
        .into_iter()
        .map(|(_, request)| request)
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

fn set_paused(core: &mut Core, paused: bool) -> Vec<Effect> {
    intent(
        core,
        Intent::SetSwitchingPaused {
            gpu_id: GPU.into(),
            paused,
        },
    )
}

fn retry(core: &mut Core) -> Vec<Effect> {
    intent(core, Intent::RetrySwitching(GPU.into()))
}

/// Every palette title, with why it can't run, while the palette is open.
fn palette(core: &mut Core) -> Vec<(String, Option<String>)> {
    intent(core, Intent::OpenPalette);
    let entries = core
        .palette()
        .expect("the palette is open")
        .entries
        .into_iter()
        .map(|entry| (entry.title, entry.unavailable))
        .collect();
    intent(core, Intent::Dismiss);
    entries
}

/// The palette's automatic switching commands for the GPU, with why they can't run.
fn switching_commands(core: &mut Core) -> Vec<(String, Option<String>)> {
    palette(core)
        .into_iter()
        .filter(|(title, _)| {
            title.ends_with(LABEL)
                && (title.contains("automatic switching") || title.starts_with("Retry"))
        })
        .collect()
}

fn notifications(effects: &[Effect]) -> Vec<Notification> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Notify(notification) => Some(notification.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn every_gpu_state_explains_itself_in_the_workspace_and_the_status_bar() {
    let cases = [
        (
            active(),
            "Cyberpunk 2077 → Quiet",
            "Cyberpunk 2077 is running, so automatic switching applied Quiet. The pre-game \
             tuning values come back when it ends.",
        ),
        (
            GpuSwitchingState::WaitingForEdits {
                profile: profile("Quiet"),
            },
            "Waiting for edits → Quiet",
            "A game linked to Quiet is running. Automatic switching applies it once this GPU's \
             unapplied tuning changes are applied or discarded.",
        ),
        (
            GpuSwitchingState::Paused {},
            "Paused",
            "Automatic switching is paused on this GPU, keeping its current tuning values, \
             until you resume it.",
        ),
        (
            GpuSwitchingState::AnotherUser {},
            "Another user's game",
            "Another user's running game drives this GPU.",
        ),
        (
            GpuSwitchingState::Restored {},
            "Pre-game values restored",
            "The game ended, and automatic switching restored this GPU's pre-game tuning values.",
        ),
        (
            GpuSwitchingState::ManualOverride {
                changed_outside_prism: false,
            },
            "Overridden",
            "Your own tuning change paused automatic switching on this GPU until its linked \
             games end. Resume to switch again now.",
        ),
        (
            GpuSwitchingState::ManualOverride {
                changed_outside_prism: true,
            },
            "Overridden",
            "A change made outside Prism paused automatic switching on this GPU until its \
             linked games end. Resume to switch again now.",
        ),
        (
            failed(),
            "Failed",
            "Power limit rejected. Memory offset rejected. Automatic switching won't change \
             this GPU again until you retry.",
        ),
        (
            GpuSwitchingState::RecoveryRequired {
                message: "The pre-game tuning values cannot be read".into(),
            },
            "Needs recovery",
            "The pre-game tuning values cannot be read. Automatic switching won't change this \
             GPU again until you retry.",
        ),
    ];
    for (state, summary, detail) in cases {
        let mut core = connected();
        push(&mut core, on_gpu(state.clone()));

        let shown = shown(&core);
        assert_eq!(shown.summary, summary, "{state:?}");
        assert_eq!(shown.detail.join(" "), detail, "{state:?}");
        assert_eq!(
            status_bar(&core).as_deref(),
            Some(format!("AUTO · {}", summary.to_uppercase()).as_str()),
            "{state:?}"
        );
    }
}

#[test]
fn without_a_state_for_the_gpu_the_condition_explains_it() {
    let cases = [
        (
            SwitchingCondition::Ready {},
            "No linked game running",
            "Automatic switching applies a game's linked profile while the game runs.",
        ),
        (
            SwitchingCondition::Off {},
            "Off",
            "Automatic switching is off. Turn it on in Games.",
        ),
        (
            SwitchingCondition::Blocked {
                reason: "Automatic switching is suspended because Prism's tuning store can't be \
                         used."
                    .into(),
            },
            "Blocked",
            "Automatic switching is suspended because Prism's tuning store can't be used.",
        ),
        (
            SwitchingCondition::RecognitionUnavailable {
                reason: "A complete host process scan could not be read".into(),
            },
            "Can't recognize games",
            "A complete host process scan could not be read. Current tuning values are held \
             until recognition works again.",
        ),
    ];
    for (condition, summary, detail) in cases {
        let mut core = connected();
        push(
            &mut core,
            AutomaticSwitchingStatus {
                condition: condition.clone(),
                gpus: Vec::new(),
            },
        );

        let shown = shown(&core);
        assert_eq!(shown.summary, summary, "{condition:?}");
        assert_eq!(shown.detail.join(" "), detail, "{condition:?}");
        assert!(!shown.failed);
    }
}

#[test]
fn the_snapshot_says_how_automatic_switching_starts() {
    let mut snapshot = snapshot();
    snapshot.automatic_switching = on_gpu(active());

    let (core, _) = connected_from(snapshot, true);

    assert_eq!(shown(&core).summary, "Cyberpunk 2077 → Quiet");
    assert_eq!(
        status_bar(&core).as_deref(),
        Some("AUTO · CYBERPUNK 2077 → QUIET")
    );
}

#[test]
fn a_manual_override_names_the_running_games_it_waits_for() {
    let mut core = connected();
    push_event(
        &mut core,
        DaemonEvent::GamesChanged(GamesState {
            automatic_switching: true,
            games: vec![GameEntry {
                game: CYBERPUNK,
                name: "Cyberpunk 2077".into(),
                installed: true,
                profiles: BTreeMap::from([(GPU.to_owned(), "quiet".to_owned())]),
            }],
        }),
    );
    push_event(
        &mut core,
        DaemonEvent::RunningGamesChanged(RunningGamesState::Available {
            games: vec![RunningGame {
                game: CYBERPUNK,
                pid: 4242,
                start_time_ticks: 100,
            }],
        }),
    );

    push(
        &mut core,
        on_gpu(GpuSwitchingState::ManualOverride {
            changed_outside_prism: false,
        }),
    );

    let shown = shown(&core);
    assert_eq!(shown.summary, "Overridden until Cyberpunk 2077 ends");
    assert_eq!(
        shown.detail.join(" "),
        "Your own tuning change paused automatic switching on this GPU until Cyberpunk 2077 \
         ends. Resume to switch again now."
    );
    assert!(shown.paused, "Resume, not Pause");
}

#[test]
fn without_a_session_automatic_switching_isnt_shown() {
    let mut core = connected();
    push(&mut core, on_gpu(active()));

    core.handle(Input::PrismdNotConnected(SessionFailure::Unavailable {
        message: "gone".into(),
    }));

    assert_eq!(status(&core), None);
    assert_eq!(status_bar(&core), None);
    assert_eq!(switching_commands(&mut core), []);
}

#[test]
fn outside_a_gpu_the_status_bar_shows_the_condition() {
    let mut core = connected();
    push(&mut core, on_gpu(active()));

    intent(&mut core, Intent::GoTo(Section::Games));

    assert_eq!(status_bar(&core).as_deref(), Some("AUTO · ON"));

    push(
        &mut core,
        AutomaticSwitchingStatus {
            condition: SwitchingCondition::Off {},
            gpus: Vec::new(),
        },
    );
    assert_eq!(status_bar(&core).as_deref(), Some("AUTO · OFF"));
}

#[test]
fn pause_asks_prismd_and_the_gpu_shows_paused_only_once_prismd_says_so() {
    let mut core = connected();
    push(&mut core, on_gpu(active()));
    let shown_before = shown(&core);
    assert!(!shown_before.paused);
    assert_eq!(shown_before.pause_unavailable, None);

    let effects = set_paused(&mut core, true);

    let [(command, request)] = commands(&effects).try_into().expect("one request");
    assert_eq!(request, pause_request(true));
    let waiting = shown(&core);
    assert_eq!(waiting.summary, "Cyberpunk 2077 → Quiet");
    assert_eq!(waiting.pause_unavailable.as_deref(), Some("Pausing…"));
    assert_eq!(set_paused(&mut core, true), []);

    core.handle(Input::PrismdAnswered {
        id: command,
        response: DaemonResponse::Command(DaemonCommandResponse {
            success: true,
            error: None,
        }),
    });
    assert_eq!(shown(&core).summary, "Cyberpunk 2077 → Quiet");

    push(&mut core, on_gpu(GpuSwitchingState::Paused {}));

    let paused = shown(&core);
    assert_eq!(paused.summary, "Paused");
    assert!(paused.paused);
    assert_eq!(paused.pause_unavailable, None);
}

#[test]
fn resume_asks_prismd_to_end_a_pause_or_a_manual_override() {
    for state in [
        GpuSwitchingState::Paused {},
        GpuSwitchingState::ManualOverride {
            changed_outside_prism: true,
        },
    ] {
        let mut core = connected();
        push(&mut core, on_gpu(state));

        let effects = set_paused(&mut core, false);

        assert_eq!(requests(&effects), [pause_request(false)]);
        assert_eq!(shown(&core).pause_unavailable.as_deref(), Some("Resuming…"));
    }
}

#[test]
fn a_refused_pause_says_why_until_the_next_change() {
    let mut core = connected();
    push(&mut core, on_gpu(active()));
    let [(command, _)] = commands(&set_paused(&mut core, true)).try_into().unwrap();

    core.handle(Input::PrismdAnswered {
        id: command,
        response: DaemonResponse::Error(GpuCommandError::new(
            GpuCommandErrorCode::Io,
            "The tuning store can't be written",
        )),
    });

    let shown_now = shown(&core);
    assert_eq!(shown_now.pause_unavailable, None);
    assert_eq!(
        shown_now.outcome.as_deref(),
        Some("Not paused: The tuning store can't be written.")
    );

    // prismd's next push for the GPU says what holds now.
    push(&mut core, on_gpu(GpuSwitchingState::Restored {}));
    assert_eq!(shown(&core).outcome, None);
}

#[test]
fn pause_and_resume_need_tuning_access() {
    let mut core = connected_with(false);
    push(&mut core, on_gpu(active()));

    assert_eq!(
        shown(&core).pause_unavailable.as_deref(),
        Some("Needs tuning access · bob isn't in group wheel")
    );
    assert_eq!(set_paused(&mut core, true), []);
}

#[test]
fn a_failure_pause_shows_with_retry_in_the_workspace_sidebar_and_activity() {
    let mut core = connected();
    assert!(!paused(&core));
    assert_eq!(problems(&mut core), []);

    push(&mut core, on_gpu(failed()));

    let shown = shown(&core);
    assert!(shown.failed);
    assert_eq!(shown.retry_unavailable, None);
    assert!(paused(&core));
    assert!(
        core.sidebar().gpus[0]
            .vitals()
            .ends_with("Automatic switching failed"),
        "{}",
        core.sidebar().gpus[0].vitals()
    );
    assert_eq!(core.status_bar().problem_indicator, "1 PROBLEM");
    let [problem] = problems(&mut core).try_into().expect("one problem");
    assert_eq!(problem.title, "Automatic switching failed");
    assert_eq!(
        problem.detail,
        [
            LABEL,
            "Power limit rejected.",
            "Memory offset rejected.",
            "Automatic switching won't change this GPU again until you retry.",
        ]
    );
    let button = problem.action.expect("it has Retry");
    assert_eq!(button.title, "Retry");
    assert_eq!(button.action, ActivityAction::RetrySwitching(GPU.into()));
    assert_eq!(button.unavailable, None);

    let effects = intent(&mut core, Intent::RunActivityAction(button.action));

    assert_eq!(requests(&effects), [retry_request()]);
}

#[test]
fn retry_asks_prismd_and_the_failure_clears_only_with_its_next_push() {
    let mut core = connected();
    push(&mut core, on_gpu(failed()));

    let effects = retry(&mut core);

    let [(command, request)] = commands(&effects).try_into().expect("one request");
    assert_eq!(request, retry_request());
    assert_eq!(shown(&core).retry_unavailable.as_deref(), Some("Retrying…"));
    assert_eq!(retry(&mut core), []);

    // Its answer is a status, but only events change what's shown.
    core.handle(Input::PrismdAnswered {
        id: command,
        response: DaemonResponse::AutomaticSwitchingStatus(on_gpu(active())),
    });
    assert!(shown(&core).failed);
    assert!(paused(&core));

    push(&mut core, on_gpu(active()));

    assert!(!shown(&core).failed);
    assert!(!paused(&core));
    assert_eq!(problems(&mut core), []);
}

#[test]
fn a_refused_retry_says_why() {
    let mut core = connected();
    push(&mut core, on_gpu(failed()));
    let [(command, _)] = commands(&retry(&mut core)).try_into().unwrap();

    core.handle(Input::PrismdAnswered {
        id: command,
        response: DaemonResponse::Error(GpuCommandError::new(
            GpuCommandErrorCode::NoPermission,
            "bob isn't in group wheel",
        )),
    });

    let outcome = Some("Not retried: bob isn't in group wheel.".to_owned());
    assert_eq!(shown(&core).outcome, outcome);
    assert_eq!(problems(&mut core)[0].detail.last(), outcome.as_ref());
}

#[test]
fn retry_is_only_for_a_failure_pause() {
    let mut core = connected();
    push(&mut core, on_gpu(active()));

    assert!(!shown(&core).failed);
    assert_eq!(retry(&mut core), []);
}

#[test]
fn a_failure_pause_notifies_while_unfocused_and_clicking_opens_the_gpu() {
    let mut core = connected();
    intent(&mut core, Intent::GoTo(Section::Games));

    let effects = push(&mut core, on_gpu(failed()));

    let [notification] = notifications(&effects)
        .try_into()
        .expect("one notification");
    assert_eq!(
        notification.summary,
        format!("Automatic switching failed on {LABEL}")
    );
    assert_eq!(
        notification.body,
        "Power limit rejected. Memory offset rejected. Automatic switching won't change this \
         GPU again until you retry."
    );

    let effects = core.handle(Input::NotificationClicked {
        id: notification.id,
        activation_token: Some("token".into()),
    });

    assert!(effects.contains(&Effect::RaiseWindow {
        activation_token: Some("token".into())
    }));
    let Content::Gpu(workspace) = core.content() else {
        panic!("expected the GPU workspace")
    };
    assert_eq!(workspace.id, GPU);
    assert_eq!(
        workspace.reveal_fans, 0,
        "the Fans track has nothing to say"
    );
}

#[test]
fn a_failure_pause_notifies_once_and_its_notification_goes_when_it_ends() {
    let mut core = connected();
    let [notification] = notifications(&push(&mut core, on_gpu(failed())))
        .try_into()
        .unwrap();

    // Still failed, such as after a Retry that failed again.
    assert_eq!(notifications(&push(&mut core, on_gpu(failed()))), []);

    let effects = push(&mut core, on_gpu(active()));

    assert_eq!(effects, [Effect::WithdrawNotification(notification.id)]);
}

#[test]
fn no_notification_while_focused_or_for_a_failure_already_there_on_connecting() {
    let mut core = connected();
    core.handle(Input::WindowFocusChanged(true));
    assert_eq!(notifications(&push(&mut core, on_gpu(failed()))), []);

    let mut snapshot = snapshot();
    snapshot.automatic_switching = on_gpu(failed());
    let (core, effects) = connected_from(snapshot, true);
    assert_eq!(notifications(&effects), []);
    assert!(shown(&core).failed);
}

#[test]
fn a_cooling_fault_ending_leaves_a_newer_switching_notification_alone() {
    let mut core = connected();
    let curve = CoolingConfiguration::Shared {
        setting: FanSetting::Curve {
            points: vec![FanCurvePoint::new(45, 30), FanCurvePoint::new(85, 100)],
        },
    };
    let fault = GpuCoolingControl {
        gpu_id: GPU.into(),
        revision: 2,
        applied: Some(curve),
        suspension: Some(CurveSuspension {
            kind: SuspensionKind::Fault,
            reason: "The GPU's temperature is unavailable.".into(),
            restoration: Vec::new(),
        }),
        recovery: None,
        remembered_curve: None,
    };
    let [cooling] = notifications(&push_event(
        &mut core,
        DaemonEvent::CoolingControlChanged(fault.clone()),
    ))
    .try_into()
    .unwrap();

    let [switching] = notifications(&push(&mut core, on_gpu(failed())))
        .try_into()
        .unwrap();
    // At most one live notification per GPU.
    assert_eq!(switching.replaces, Some(cooling.id));

    let effects = push_event(
        &mut core,
        DaemonEvent::CoolingControlChanged(GpuCoolingControl {
            revision: 3,
            suspension: None,
            ..fault
        }),
    );
    assert_eq!(effects, []);
}

#[test]
fn the_palette_offers_pause_resume_and_retry_only_when_they_apply() {
    let mut core = connected();
    let pause = format!("Pause automatic switching on {LABEL}");
    let resume = format!("Resume automatic switching on {LABEL}");
    let retry_title = format!("Retry {LABEL}");

    push(&mut core, on_gpu(active()));
    assert_eq!(switching_commands(&mut core), [(pause.clone(), None)]);

    push(&mut core, on_gpu(GpuSwitchingState::Paused {}));
    assert_eq!(switching_commands(&mut core), [(resume.clone(), None)]);

    push(
        &mut core,
        on_gpu(GpuSwitchingState::ManualOverride {
            changed_outside_prism: false,
        }),
    );
    assert_eq!(
        switching_commands(&mut core),
        [(pause.clone(), None), (resume, None)]
    );

    push(&mut core, on_gpu(failed()));
    assert_eq!(
        switching_commands(&mut core),
        [(pause.clone(), None), (retry_title.clone(), None)]
    );

    // From anywhere, not only the GPU's workspace.
    intent(&mut core, Intent::GoTo(Section::Games));
    intent(&mut core, Intent::OpenPalette);
    intent(&mut core, Intent::FilterPalette(retry_title));
    let effects = intent(&mut core, Intent::RunHighlighted);
    assert_eq!(requests(&effects), [retry_request()]);

    intent(&mut core, Intent::OpenPalette);
    intent(&mut core, Intent::FilterPalette(pause));
    let effects = intent(&mut core, Intent::RunHighlighted);
    assert_eq!(requests(&effects), [pause_request(true)]);
}

#[test]
fn palette_commands_say_they_need_tuning_access() {
    let mut core = connected_with(false);
    push(&mut core, on_gpu(failed()));

    let reason = Some("Needs tuning access · bob isn't in group wheel".to_owned());
    assert_eq!(
        switching_commands(&mut core),
        [
            (
                format!("Pause automatic switching on {LABEL}"),
                reason.clone()
            ),
            (format!("Retry {LABEL}"), reason),
        ]
    );
}

#[test]
fn retry_tells_switching_from_a_cooling_fault_on_the_same_gpu() {
    let mut core = connected();
    push_event(
        &mut core,
        DaemonEvent::CoolingControlChanged(GpuCoolingControl {
            gpu_id: GPU.into(),
            revision: 2,
            applied: None,
            suspension: Some(CurveSuspension {
                kind: SuspensionKind::Fault,
                reason: "The GPU's temperature is unavailable.".into(),
                restoration: Vec::new(),
            }),
            recovery: None,
            remembered_curve: None,
        }),
    );
    push(&mut core, on_gpu(failed()));

    let titles: Vec<_> = palette(&mut core)
        .into_iter()
        .map(|(title, _)| title)
        .filter(|title| title.starts_with("Retry"))
        .collect();

    assert_eq!(
        titles,
        [
            format!("Retry {LABEL}"),
            format!("Retry automatic switching on {LABEL}"),
        ]
    );
}
