//! Cooling faults and external control (ADR 0006): a suspended fan curve shows inline in the Fans
//! track with how its fans fell back, as PAUSED in the sidebar after a fault and as a problem in
//! Activity, with Retry, or Resume after another tool took control, which applies the kept curve
//! again. A fault notifies while Prism's window doesn't have focus.

use std::time::Duration;

use prism_gpu::cooling::{
    CoolingApplication, CoolingConfiguration, CurveSuspension, FanOutcome, FanRestoration,
    FanSetting, RestorationStatus, SuspensionKind,
};
use prism_gpu::fan_curve::FanCurvePoint;
use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_gpu::{
    GpuCommandError, GpuCommandErrorCode, GpuTelemetryProvider, TuningOutcome, TuningOutcomeStatus,
};
use prism_protocol::{
    ApplyGpuProfileRequest, DaemonEvent, DaemonRequest, DaemonResponse, DaemonSnapshot,
    GpuCoolingControl, ProfileApplicationResponse, ProfileSettings, ProtocolVersion,
    SessionMessage, Welcome,
};
use prism_ui::{
    ActivityAction, ActivityItem, CommandId, Content, CoolingProblem, Core, Effect, GuiSettings,
    Input, Intent, Notification, Section,
};

const GPU: &str = "GPU-5a1b0000-0000-4000-8000-000000000001";
const LABEL: &str = "GPU 0 · RTX 5090 · 01:00.0";
/// The tunable GPU's tuning-state revision in the shared snapshot.
const REVISION: &str = "4f1c2b9d0e8a7c35";
const REASON: &str = "The GPU's temperature is unavailable, so Prism stopped following its curve.";

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

/// Prism connected to prismd with the tunable GPU and its two fans, with tuning access as
/// `can_tune` says and its window without focus.
fn connected_with(can_tune: bool) -> Core {
    let mut core = Core::default();
    core.handle(Input::Launched {
        settings: GuiSettings::default(),
        started_unix_ms: 0,
    });
    core.handle(Input::PrismdConnected {
        welcome: welcome(can_tune),
        snapshot: snapshot(),
        socket: "/run/prism/prismd.sock".into(),
    });
    core.handle(Input::PrismdEvent {
        event: DaemonEvent::TelemetrySampled(
            SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO).performance(),
        ),
        received_at: Duration::from_secs(1),
    });
    core
}

fn connected() -> Core {
    connected_with(true)
}

fn curve() -> CoolingConfiguration {
    CoolingConfiguration::Shared {
        setting: FanSetting::Curve {
            points: vec![FanCurvePoint::new(45, 30), FanCurvePoint::new(85, 100)],
        },
    }
}

fn restoration(fan_index: u32, status: RestorationStatus, detail: Option<&str>) -> FanRestoration {
    FanRestoration {
        fan_index,
        status,
        detail: detail.map(Into::into),
    }
}

/// prismd's push that the GPU's curve stopped as `kind` says, with each fan's fallback.
fn suspended(kind: SuspensionKind, restoration: Vec<FanRestoration>) -> GpuCoolingControl {
    GpuCoolingControl {
        gpu_id: GPU.into(),
        revision: 2,
        applied: Some(curve()),
        suspension: Some(CurveSuspension {
            kind,
            reason: REASON.into(),
            restoration,
        }),
        recovery: None,
        remembered_curve: Some(curve()),
    }
}

/// A fault whose fans both went back to automatic cooling.
fn fault() -> GpuCoolingControl {
    suspended(
        SuspensionKind::Fault,
        vec![
            restoration(0, RestorationStatus::Restored, None),
            restoration(1, RestorationStatus::Restored, None),
        ],
    )
}

/// The curve followed again, healthy.
fn healthy() -> GpuCoolingControl {
    GpuCoolingControl {
        revision: 3,
        suspension: None,
        ..fault()
    }
}

fn push(core: &mut Core, control: GpuCoolingControl) -> Vec<Effect> {
    core.handle(Input::PrismdEvent {
        event: DaemonEvent::CoolingControlChanged(control),
        received_at: Duration::from_secs(2),
    })
}

fn intent(core: &mut Core, intent: Intent) -> Vec<Effect> {
    core.handle(Input::Intent(intent))
}

/// The Fans track's cooling problem, if it shows one.
fn inline(core: &Core) -> Option<CoolingProblem> {
    let Content::Gpu(workspace) = core.content() else {
        panic!("expected the GPU workspace")
    };
    workspace
        .fan_mode()
        .expect("the Fans track has a fan control mode")
        .problem
        .clone()
}

fn paused(core: &Core) -> bool {
    core.sidebar().gpus[0].paused()
}

/// The problems Activity lists.
fn problems(core: &mut Core) -> Vec<ActivityItem> {
    intent(core, Intent::OpenActivity);
    core.activity().expect("Activity is open").problems
}

/// The Applies among `effects`.
fn applies(effects: &[Effect]) -> Vec<(CommandId, ApplyGpuProfileRequest)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::CommandPrismd {
                id,
                request: DaemonRequest::ApplyProfile(request),
            } => Some((*id, request.clone())),
            _ => None,
        })
        .collect()
}

/// The Apply that brings the suspended curve back.
fn retry_request() -> ApplyGpuProfileRequest {
    ApplyGpuProfileRequest {
        gpu_id: GPU.into(),
        settings: ProfileSettings {
            cooling: Some(curve()),
            ..ProfileSettings::default()
        },
        expected_revision: REVISION.into(),
        fan_indexes: Some(vec![0, 1]),
    }
}

fn retry(core: &mut Core) -> Vec<Effect> {
    intent(core, Intent::RetryCooling(GPU.into()))
}

/// The palette entry titled `title`, with why it can't run, if it's listed.
fn palette_entry(core: &mut Core, title: &str) -> Option<Option<String>> {
    intent(core, Intent::OpenPalette);
    intent(core, Intent::FilterPalette(String::new()));
    let entry = core
        .palette()
        .expect("the palette is open")
        .entries
        .into_iter()
        .find(|entry| entry.title == title)
        .map(|entry| entry.unavailable);
    intent(core, Intent::Dismiss);
    entry
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
fn a_cooling_fault_shows_inline_with_each_fans_fallback_and_retry() {
    let mut core = connected();
    assert_eq!(inline(&core), None);

    push(
        &mut core,
        suspended(
            SuspensionKind::Fault,
            vec![
                restoration(0, RestorationStatus::Restored, None),
                restoration(1, RestorationStatus::Failed, Some("NVML: Not Supported")),
            ],
        ),
    );

    let problem = inline(&core).expect("the fault shows inline");
    assert_eq!(problem.title, "Fan curve paused");
    assert_eq!(
        problem.detail,
        [
            REASON,
            "Fan 0: automatic cooling restored.",
            "Fan 1: automatic cooling not restored: NVML: Not Supported.",
            "Prism kept the curve, but won't follow it again until you retry.",
        ]
    );
    assert_eq!(problem.action, "Retry");
    assert_eq!(problem.unavailable, None);
    // Said once, where its Retry is.
    let Content::Gpu(workspace) = core.content() else {
        panic!("expected the GPU workspace")
    };
    assert_eq!(workspace.fan_mode().unwrap().note, None);
}

#[test]
fn a_cooling_fault_marks_the_gpu_paused_in_the_sidebar() {
    let mut core = connected();
    assert!(!paused(&core));

    push(&mut core, fault());

    assert!(paused(&core));
    assert!(
        core.sidebar().gpus[0]
            .vitals()
            .ends_with("Fan curve paused"),
        "{}",
        core.sidebar().gpus[0].vitals()
    );
}

#[test]
fn a_cooling_fault_is_a_problem_in_activity_with_retry() {
    let mut core = connected();
    assert_eq!(core.status_bar().problem_indicator, "NO PROBLEMS");

    push(&mut core, fault());

    assert_eq!(core.status_bar().problem_indicator, "1 PROBLEM");
    let [problem] = problems(&mut core).try_into().expect("one problem");
    assert_eq!(problem.title, "Fan curve paused");
    assert_eq!(problem.detail[0], LABEL);
    assert_eq!(problem.detail[1], REASON);
    let button = problem.action.expect("it has Retry");
    assert_eq!(button.title, "Retry");
    assert_eq!(button.action, ActivityAction::RetryCooling(GPU.into()));
    assert_eq!(button.unavailable, None);

    let effects = intent(&mut core, Intent::RunActivityAction(button.action));

    assert_eq!(
        applies(&effects)
            .into_iter()
            .map(|(_, request)| request)
            .collect::<Vec<_>>(),
        [retry_request()]
    );
}

#[test]
fn retry_applies_the_kept_curve_again_and_the_fault_clears_only_with_the_next_event() {
    let mut core = connected();
    push(&mut core, fault());

    let effects = retry(&mut core);

    let [(command, request)] = applies(&effects).try_into().expect("one Apply");
    assert_eq!(request, retry_request());
    // Nothing changes until prismd says so, and Retry waits for its answer.
    assert!(paused(&core));
    let problem = inline(&core).expect("still shown");
    assert_eq!(problem.unavailable.as_deref(), Some("Retrying…"));
    assert_eq!(retry(&mut core), []);

    core.handle(Input::PrismdAnswered {
        id: command,
        response: DaemonResponse::ProfileApplication(ProfileApplicationResponse {
            rollback: None,
            cooling: Some(CoolingApplication {
                revision: 3,
                outcome: TuningOutcome {
                    status: TuningOutcomeStatus::Succeeded,
                    error: None,
                },
                fans: vec![FanOutcome {
                    fan_index: 0,
                    outcome: TuningOutcome {
                        status: TuningOutcomeStatus::Succeeded,
                        error: None,
                    },
                }],
            }),
            power: None,
            graphics: None,
            memory: None,
            corrections: Vec::new(),
        }),
    });
    assert!(paused(&core));
    assert!(inline(&core).is_some());
    assert_eq!(problems(&mut core).len(), 1);

    push(&mut core, healthy());

    assert!(!paused(&core));
    assert_eq!(inline(&core), None);
    assert_eq!(problems(&mut core), []);
    assert_eq!(core.status_bar().problem_indicator, "NO PROBLEMS");
}

#[test]
fn a_refused_retry_says_why_until_the_next_one() {
    let mut core = connected();
    push(&mut core, fault());
    let [(command, _)] = applies(&retry(&mut core)).try_into().unwrap();

    core.handle(Input::PrismdAnswered {
        id: command,
        response: DaemonResponse::Error(GpuCommandError::new(
            GpuCommandErrorCode::StaleRevision,
            "The GPU's tuning values changed since these changes were made.",
        )),
    });

    let problem = inline(&core).unwrap();
    assert_eq!(problem.unavailable, None);
    assert_eq!(
        problem.outcome.as_deref(),
        Some("Not retried: The GPU's tuning values changed since these changes were made.")
    );
    assert_eq!(
        problems(&mut core)[0].detail.last().map(String::as_str),
        problem.outcome.as_deref()
    );

    retry(&mut core);
    assert_eq!(inline(&core).unwrap().outcome, None);
}

#[test]
fn the_palette_offers_retry_for_a_gpu_with_a_cooling_fault() {
    let mut core = connected();
    let title = format!("Retry {LABEL}");
    assert_eq!(palette_entry(&mut core, &title), None);
    push(&mut core, fault());
    // From anywhere, not only the GPU's workspace.
    intent(&mut core, Intent::GoTo(Section::Games));
    assert_eq!(palette_entry(&mut core, &title), Some(None));

    intent(&mut core, Intent::OpenPalette);
    intent(&mut core, Intent::FilterPalette(title));
    let effects = intent(&mut core, Intent::RunHighlighted);

    assert_eq!(
        applies(&effects)
            .into_iter()
            .map(|(_, request)| request)
            .collect::<Vec<_>>(),
        [retry_request()]
    );
}

#[test]
fn retry_is_unavailable_while_prism_has_no_configuration_to_apply_again() {
    let mut core = connected();
    push(
        &mut core,
        GpuCoolingControl {
            applied: None,
            ..fault()
        },
    );

    assert_eq!(
        inline(&core).unwrap().unavailable.as_deref(),
        Some("Prism has no cooling configuration to apply again")
    );
    assert_eq!(retry(&mut core), []);
}

#[test]
fn retry_needs_tuning_access() {
    let mut core = connected_with(false);
    push(&mut core, fault());

    let problem = inline(&core).unwrap();
    assert_eq!(
        problem.unavailable.as_deref(),
        Some("Needs tuning access · bob isn't in group wheel")
    );
    assert_eq!(retry(&mut core), []);
    assert_eq!(
        palette_entry(&mut core, &format!("Retry {LABEL}")),
        Some(Some(
            "Needs tuning access · bob isn't in group wheel".into()
        ))
    );
}

#[test]
fn an_external_control_change_shows_curve_control_suspended_with_resume() {
    let mut core = connected();

    push(
        &mut core,
        suspended(SuspensionKind::ExternalControl, Vec::new()),
    );

    let problem = inline(&core).expect("the suspension shows inline");
    assert_eq!(problem.title, "Fan curve suspended");
    assert_eq!(
        problem.detail,
        [
            REASON,
            "Prism left the other tool's fan control alone. Resume when you want Prism to \
             follow the curve again.",
        ]
    );
    assert_eq!(problem.action, "Resume");
    // It isn't a failure pause.
    assert!(!paused(&core));
    let [item] = problems(&mut core).try_into().expect("one problem");
    assert_eq!(item.title, "Fan curve suspended");
    assert_eq!(item.action.unwrap().title, "Resume");
    assert_eq!(
        palette_entry(&mut core, &format!("Resume {LABEL}")),
        Some(None)
    );

    let effects = retry(&mut core);

    assert_eq!(
        applies(&effects)
            .into_iter()
            .map(|(_, request)| request)
            .collect::<Vec<_>>(),
        [retry_request()]
    );
}

#[test]
fn a_cooling_fault_notifies_while_unfocused_and_clicking_opens_the_gpu() {
    let mut core = connected();
    intent(&mut core, Intent::GoTo(Section::Games));

    let effects = push(&mut core, fault());

    let [notification] = notifications(&effects)
        .try_into()
        .expect("one notification");
    assert_eq!(notification.summary, format!("Fan curve paused on {LABEL}"));
    assert_eq!(
        notification.body,
        format!("{REASON} Prism returned its fans to automatic cooling.")
    );

    let before = match core.content() {
        Content::Gpu(workspace) => workspace.reveal_fans,
        _ => 0,
    };
    let effects = core.handle(Input::NotificationClicked {
        id: notification.id,
        activation_token: Some("token".into()),
    });

    assert!(effects.contains(&Effect::RaiseWindow {
        activation_token: Some("token".into())
    }));
    assert!(inline(&core).is_some(), "Prism shows the GPU's Fans track");
    let Content::Gpu(workspace) = core.content() else {
        panic!("expected the GPU workspace")
    };
    assert_eq!(
        workspace.reveal_fans,
        before + 1,
        "and scrolls it into view"
    );
}

#[test]
fn a_fault_that_left_a_fan_unrestored_says_so_in_its_notification() {
    let mut core = connected();

    let effects = push(
        &mut core,
        suspended(
            SuspensionKind::Fault,
            vec![
                restoration(0, RestorationStatus::Restored, None),
                restoration(1, RestorationStatus::Unverified, None),
            ],
        ),
    );

    let [notification] = notifications(&effects).try_into().unwrap();
    assert_eq!(
        notification.body,
        format!("{REASON} Prism couldn't confirm every fan returned to automatic cooling.")
    );
}

#[test]
fn a_fault_notifies_once_and_its_notification_goes_when_it_clears() {
    let mut core = connected();
    let [notification] = notifications(&push(&mut core, fault())).try_into().unwrap();

    // The same fault pushed again, such as when the remembered curve changed.
    assert_eq!(
        notifications(&push(
            &mut core,
            GpuCoolingControl {
                revision: 3,
                remembered_curve: None,
                ..fault()
            }
        )),
        []
    );

    let effects = push(&mut core, healthy());

    assert_eq!(effects, [Effect::WithdrawNotification(notification.id)]);
}

#[test]
fn no_notification_while_focused_or_for_external_control() {
    let mut core = connected();
    core.handle(Input::WindowFocusChanged(true));
    assert_eq!(notifications(&push(&mut core, fault())), []);

    let mut core = connected();
    assert_eq!(
        notifications(&push(
            &mut core,
            suspended(SuspensionKind::ExternalControl, Vec::new())
        )),
        []
    );
}

#[test]
fn no_notification_with_desktop_notifications_off() {
    let mut core = connected();
    core.handle(Input::Intent(Intent::SetDesktopNotifications(false)));

    assert_eq!(notifications(&push(&mut core, fault())), []);
}

#[test]
fn a_fault_already_there_when_prism_connects_isnt_notified_but_is_shown() {
    let mut core = Core::default();
    core.handle(Input::Launched {
        settings: GuiSettings::default(),
        started_unix_ms: 0,
    });
    let mut snapshot = snapshot();
    snapshot.cooling_controls = vec![fault()];

    let effects = core.handle(Input::PrismdConnected {
        welcome: welcome(true),
        snapshot,
        socket: "/run/prism/prismd.sock".into(),
    });

    assert_eq!(notifications(&effects), []);
    assert!(paused(&core));
}
