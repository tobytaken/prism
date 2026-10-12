//! Activity and the problem indicator: current problems first, such as an unhealthy tuning store
//! or a startup outcome that didn't apply in full, then this boot's undismissed startup outcomes,
//! then recent events from prismd's Activity and Prism's own, each with its action. Opened from
//! the status bar's problem indicator, Ctrl+Shift+A or the palette, and closed with Escape.

use std::time::Duration;

use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_gpu::{GpuCommandError, GpuCommandErrorCode, GpuTelemetryProvider};
use prism_protocol::{
    ActivityEntry, DaemonEvent, DaemonRequest, DaemonResponse, DaemonSnapshot, ProtocolVersion,
    SessionMessage, StartupGpuRequest, StartupOutcome, StartupState, TuningStoreHealth,
    TuningStoreProblem, Welcome,
};
use prism_ui::{
    ActivityAction, ActivityItem, ActivityPanel, CommandId, Core, Effect, GuiSettings, Input,
    Intent, Key, Section, SessionFailure, Shortcut,
};

const GPU: &str = "GPU-5a1b0000-0000-4000-8000-000000000001";
const LABEL: &str = "GPU 0 · RTX 5090 · 01:00.0";
/// When Prism started, a minute after prismd's latest Activity entry.
const STARTED_UNIX_MS: u64 = 1_791_804_860_000;

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

/// What prismd did this boot: startup failed to apply Quiet in full, then Boost was applied, Dota
/// 2 started with Quiet, and its pre-game values were restored.
fn entries() -> Vec<ActivityEntry> {
    serde_json::from_str(include_str!(
        "../../prism-protocol/tests/contracts/activity-entries.json"
    ))
    .unwrap()
}

/// This boot's startup outcome: Quiet applied in part.
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

fn welcome(can_tune: bool) -> Welcome {
    Welcome {
        protocol: ProtocolVersion::CURRENT,
        prismd_version: "0.1.0".into(),
        can_tune,
        reason: (!can_tune).then(|| "bob isn't in group wheel".into()),
        authorization: None,
    }
}

fn launched() -> Core {
    let mut core = Core::default();
    core.handle(Input::Launched {
        settings: GuiSettings::default(),
        started_unix_ms: STARTED_UNIX_MS,
    });
    core
}

/// Prism connected to prismd with the tunable GPU, one second after it started, from a snapshot
/// `change` adjusts.
fn connected_with(can_tune: bool, change: impl FnOnce(&mut DaemonSnapshot)) -> Core {
    let mut core = launched();
    let mut snapshot = snapshot();
    change(&mut snapshot);
    core.handle(Input::PrismdConnected {
        welcome: welcome(can_tune),
        snapshot,
        socket: "/run/prism/prismd.sock".into(),
    });
    sample(&mut core, Duration::from_secs(1));
    core
}

fn connected() -> Core {
    connected_with(true, |_| {})
}

/// prismd's telemetry tick, which reached Prism `at` after it started.
fn sample(core: &mut Core, at: Duration) {
    core.handle(Input::PrismdEvent {
        event: DaemonEvent::TelemetrySampled(
            SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO).performance(),
        ),
        received_at: at,
    });
}

fn push(core: &mut Core, event: DaemonEvent) {
    core.handle(Input::PrismdEvent {
        event,
        received_at: Duration::from_secs(2),
    });
}

fn intent(core: &mut Core, intent: Intent) -> Vec<Effect> {
    core.handle(Input::Intent(intent))
}

fn press(core: &mut Core, key: Key) -> Vec<Effect> {
    let shortcut = Shortcut::for_key(&key).expect("the key is a shortcut");
    intent(core, Intent::Press(shortcut))
}

fn ctrl_shift_a() -> Key {
    Key {
        text: "A".into(),
        control: true,
        shift: true,
        ..Key::default()
    }
}

fn escape() -> Key {
    Key {
        text: "\u{1b}".into(),
        ..Key::default()
    }
}

fn panel(core: &Core) -> ActivityPanel {
    core.activity().expect("Activity is open")
}

fn titles(items: &[ActivityItem]) -> Vec<&str> {
    items.iter().map(|item| item.title.as_str()).collect()
}

fn sent_once(effects: &[Effect]) -> (CommandId, DaemonRequest) {
    let sent: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::CommandPrismd { id, request } => Some((*id, request.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(sent.len(), 1, "{effects:?}");
    sent[0].clone()
}

#[test]
fn the_problem_indicator_opens_activity_and_escape_closes_it() {
    let mut core = connected();
    assert!(core.activity().is_none());
    assert_eq!(core.status_bar().problems, 0);
    assert_eq!(core.status_bar().problem_indicator, "NO PROBLEMS");

    intent(&mut core, Intent::OpenActivity);
    assert!(core.activity().is_some());

    press(&mut core, escape());
    assert!(core.activity().is_none());
}

#[test]
fn ctrl_shift_a_toggles_activity_and_the_palette_opens_it() {
    let mut core = connected();
    press(&mut core, ctrl_shift_a());
    assert!(core.activity().is_some());
    press(&mut core, ctrl_shift_a());
    assert!(core.activity().is_none());

    intent(&mut core, Intent::OpenPalette);
    intent(&mut core, Intent::FilterPalette("open activity".into()));
    let entry = &core.palette().unwrap().entries[0];
    assert_eq!(entry.title, "Open Activity");
    assert_eq!(entry.keys, ["Ctrl", "Shift", "A"]);
    intent(&mut core, Intent::RunHighlighted);
    assert!(core.activity().is_some());
    assert!(core.palette().is_none());

    // Open, it's listed as such.
    intent(&mut core, Intent::OpenPalette);
    intent(&mut core, Intent::FilterPalette("open activity".into()));
    assert_eq!(
        core.palette().unwrap().entries[0].unavailable.as_deref(),
        Some("Already open")
    );
    // Escape closes the palette over Activity first.
    press(&mut core, escape());
    assert!(core.palette().is_none());
    assert!(core.activity().is_some());
}

#[test]
fn the_panel_lists_what_prismd_did_while_prism_was_closed_newest_first() {
    let mut core = connected_with(true, |snapshot| snapshot.activity = entries());
    intent(&mut core, Intent::OpenActivity);

    let recent = panel(&core).recent;
    assert_eq!(
        titles(&recent),
        [
            "Pre-game values restored",
            "Dota 2 started · Quiet applied",
            "Boost applied",
            "Startup applied Quiet in part",
        ]
    );
    assert_eq!(recent[0].detail, [LABEL, "Power limit restored"]);
    assert_eq!(recent[0].when.as_deref(), Some("1 min ago"));
    assert_eq!(recent[3].when.as_deref(), Some("1 h ago"));
    assert_eq!(
        recent[3].detail,
        [
            LABEL,
            "Power limit applied",
            "Graphics clock offset not applied: Driver rejected the offset.",
        ]
    );
    // Each event opens its GPU.
    let open = recent[1].action.as_ref().expect("an action");
    assert_eq!(open.title, "Open");
    assert_eq!(open.action, ActivityAction::OpenGpu(GPU.into()));
    intent(&mut core, Intent::GoTo(Section::Games));
    intent(&mut core, Intent::OpenActivity);
    intent(&mut core, Intent::RunActivityAction(open.action.clone()));
    assert!(core.activity().is_none());
    assert!(core.sidebar().gpus[0].selected);
}

#[test]
fn events_pushed_later_join_the_top_once_each() {
    let mut core = connected_with(true, |snapshot| snapshot.activity = entries()[..2].to_vec());
    intent(&mut core, Intent::OpenActivity);
    let later = entries()[2].clone();
    push(&mut core, DaemonEvent::ActivityRecorded(later.clone()));
    // The snapshot and a push can overlap; an entry shows once.
    push(&mut core, DaemonEvent::ActivityRecorded(later));
    assert_eq!(
        titles(&panel(&core).recent),
        [
            "Dota 2 started · Quiet applied",
            "Boost applied",
            "Startup applied Quiet in part",
        ]
    );
}

#[test]
fn startup_outcomes_show_per_setting_until_dismissed() {
    let mut core = connected_with(true, |snapshot| {
        snapshot.startup = startup();
        snapshot.activity = entries()[..1].to_vec();
    });
    // Quiet applied only in part, which is a problem.
    assert_eq!(core.status_bar().problems, 1);
    assert_eq!(core.status_bar().problem_indicator, "1 PROBLEM");
    intent(&mut core, Intent::OpenActivity);
    let problem = &panel(&core).problems[0];
    assert_eq!(problem.title, "Startup applied Quiet in part");
    assert_eq!(
        problem.detail,
        [
            LABEL,
            "Power limit applied",
            "Graphics clock offset not applied: Driver rejected the offset.",
            "Memory clock offset not applied: An earlier profile setting failed.",
            "Cooling not applied: An earlier profile setting failed. Retry or discard this \
             instruction.",
        ]
    );
    let dismiss = problem.action.clone().expect("an action");
    assert_eq!(dismiss.title, "Dismiss");
    assert_eq!(dismiss.unavailable, None);

    let effects = intent(&mut core, Intent::RunActivityAction(dismiss.action));
    let (_, request) = sent_once(&effects);
    assert_eq!(
        request,
        DaemonRequest::DismissStartupOutcome(StartupGpuRequest { gpu_id: GPU.into() })
    );
    // It goes only once prismd says so.
    assert_eq!(panel(&core).problems.len(), 1);
    push(
        &mut core,
        DaemonEvent::StartupStateChanged(StartupState {
            outcomes: vec![],
            ..startup()
        }),
    );
    assert!(panel(&core).problems.is_empty());
    assert_eq!(core.status_bar().problems, 0);
    // It stays in recent events as what happened.
    assert_eq!(
        titles(&panel(&core).recent),
        ["Startup applied Quiet in part"]
    );
}

fn applied_outcome() -> StartupOutcome {
    let mut outcome = startup().outcomes.remove(0);
    outcome.status = prism_protocol::StartupOutcomeStatus::Applied;
    outcome.reason = None;
    outcome.application = Some(
        serde_json::from_value(serde_json::json!({
            "power": { "status": "succeeded", "error": null },
            "graphics": null, "memory": null
        }))
        .unwrap(),
    );
    outcome
}

#[test]
fn a_startup_outcome_applied_in_full_shows_under_startup_without_counting_as_a_problem() {
    let outcome = applied_outcome();
    let mut core = connected_with(true, |snapshot| {
        snapshot.startup = StartupState {
            outcomes: vec![outcome.clone()],
            ..startup()
        };
        snapshot.activity = vec![ActivityEntry {
            event: prism_protocol::ActivityEvent::StartupRan { outcome },
            ..entries().remove(0)
        }];
    });
    assert_eq!(core.status_bar().problems, 0);
    intent(&mut core, Intent::OpenActivity);
    let panel = panel(&core);
    assert!(panel.problems.is_empty());
    assert_eq!(titles(&panel.startup), ["Startup applied Quiet"]);
    assert_eq!(panel.startup[0].detail, [LABEL, "Power limit applied"]);
    assert_eq!(panel.startup[0].action.as_ref().unwrap().title, "Dismiss");
}

#[test]
fn a_startup_outcome_with_nothing_to_report_isnt_shown() {
    // prismd records only outcomes worth reporting in Activity, such as none for a GPU without a
    // startup profile.
    let mut core = connected_with(true, |snapshot| snapshot.startup = startup());
    intent(&mut core, Intent::OpenActivity);
    assert!(panel(&core).problems.is_empty());
    assert!(panel(&core).startup.is_empty());
}

#[test]
fn dismissing_needs_tuning_access_and_a_refusal_is_explained() {
    let mut core = connected_with(false, |snapshot| {
        snapshot.startup = startup();
        snapshot.activity = entries()[..1].to_vec();
    });
    intent(&mut core, Intent::OpenActivity);
    let dismiss = panel(&core).problems[0].action.clone().unwrap();
    assert_eq!(
        dismiss.unavailable.as_deref(),
        Some("Needs tuning access · bob isn't in group wheel")
    );
    assert!(intent(&mut core, Intent::RunActivityAction(dismiss.action)).is_empty());

    let mut core = connected_with(true, |snapshot| {
        snapshot.startup = startup();
        snapshot.activity = entries()[..1].to_vec();
    });
    intent(&mut core, Intent::OpenActivity);
    let dismiss = panel(&core).problems[0].action.clone().unwrap();
    let (id, _) = sent_once(&intent(
        &mut core,
        Intent::RunActivityAction(dismiss.action),
    ));
    core.handle(Input::PrismdAnswered {
        id,
        response: DaemonResponse::Error(GpuCommandError::new(
            GpuCommandErrorCode::Io,
            "Startup is suspended because Prism's boot state can't be used: disk full",
        )),
    });
    assert_eq!(
        panel(&core).outcome.as_deref(),
        Some(
            "Not dismissed: Startup is suspended because Prism's boot state can't be used: disk \
             full"
        )
    );
}

fn unhealthy() -> TuningStoreHealth {
    TuningStoreHealth::Unhealthy {
        problem: TuningStoreProblem::NewerVersion,
        path: "/var/lib/prism/tuning-store.json".into(),
        detail: "schema version 9 is newer than this prismd understands".into(),
    }
}

#[test]
fn an_unhealthy_tuning_store_is_a_problem_with_its_explanation() {
    let mut core = connected_with(true, |snapshot| snapshot.store_health = unhealthy());
    assert_eq!(core.status_bar().problems, 1);
    intent(&mut core, Intent::OpenActivity);
    let problem = &panel(&core).problems[0];
    assert_eq!(problem.title, "Prism's tuning store can't be used");
    assert_eq!(
        problem.detail,
        [
            "It was written by a newer version of Prism: schema version 9 is newer than this \
             prismd understands.",
            "Startup behavior and automatic switching are suspended. Profile applications and \
             live tuning changes keep working.",
            "Prism leaves /var/lib/prism/tuning-store.json untouched.",
        ]
    );
    assert_eq!(problem.action, None);

    push(
        &mut core,
        DaemonEvent::StoreHealthChanged(TuningStoreHealth::Healthy {}),
    );
    assert_eq!(core.status_bar().problems, 0);

    push(&mut core, DaemonEvent::StoreHealthChanged(unhealthy()));
    assert_eq!(core.status_bar().problems, 1);
    // Without a session, prismd's problems are unknown rather than gone, and not shown.
    core.handle(Input::PrismdNotConnected(SessionFailure::Unavailable {
        message: "connection refused".into(),
    }));
    assert_eq!(core.status_bar().problems, 0);
}

#[test]
fn a_short_prismd_connection_loss_is_prisms_own_entry_and_prismds_events_stay() {
    let mut core = connected_with(true, |snapshot| snapshot.activity = entries()[3..].to_vec());
    sample(&mut core, Duration::from_secs(60));
    core.handle(Input::PrismdNotConnected(SessionFailure::Unavailable {
        message: "connection refused".into(),
    }));
    core.handle(Input::TelemetrySampled {
        gpus: SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO).performance(),
        source: prism_ui::TelemetrySource::InProcess,
        received_at: Duration::from_secs(65),
    });
    intent(&mut core, Intent::OpenActivity);
    // prismd's events stay what happened.
    assert_eq!(titles(&panel(&core).recent), ["Pre-game values restored"]);

    core.handle(Input::PrismdConnected {
        welcome: welcome(true),
        snapshot: DaemonSnapshot {
            activity: entries()[3..].to_vec(),
            ..snapshot()
        },
        socket: "/run/prism/prismd.sock".into(),
    });
    sample(&mut core, Duration::from_secs(66));
    let recent = panel(&core).recent;
    assert_eq!(
        titles(&recent),
        [
            "Lost the prismd connection for 5 s",
            "Pre-game values restored"
        ]
    );
    assert_eq!(
        recent[0].detail,
        ["Tuning was read-only until Prism reconnected."]
    );
    assert_eq!(recent[0].when.as_deref(), Some("just now"));
    assert_eq!(recent[0].action, None);
}

#[test]
fn with_nothing_to_show_the_panel_says_so() {
    let mut core = connected();
    intent(&mut core, Intent::OpenActivity);
    let panel = panel(&core);
    assert!(panel.problems.is_empty() && panel.startup.is_empty() && panel.recent.is_empty());
    assert_eq!(
        panel.empty.as_deref(),
        Some("Nothing has happened yet this boot.")
    );
}
