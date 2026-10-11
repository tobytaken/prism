//! Staged tuning: per-GPU drafts of unapplied tuning changes, their edit holds, and Apply and
//! Discard, through the core's input/effect interface.

use std::time::Duration;

use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_gpu::{
    GpuCommandError, GpuCommandErrorCode, GpuInventoryProvider, GpuTelemetryProvider,
    TuningOutcome, TuningOutcomeStatus,
};
use prism_protocol::{
    ApplyGpuProfileRequest, DaemonEvent, DaemonRequest, DaemonResponse, DaemonSnapshot,
    GpuTuningState, ProfileApplicationResponse, ProfileSetting, ProfileSettings, ProtocolVersion,
    ReportedTuningValues, SessionMessage, SetEditHoldsRequest, Welcome,
};
use prism_ui::telemetry::LimitKind;
use prism_ui::{
    AppearanceMode, CommandId, Content, Core, Effect, GpuWorkspace, Input, Intent, Key,
    SessionFailure, Shortcut, TelemetrySource, TrackKind,
};

const GPU: &str = "GPU-5a1b0000-0000-4000-8000-000000000001";
/// The tunable GPU's tuning-state revision in the shared snapshot.
const REVISION: &str = "4f1c2b9d0e8a7c35";

fn simulated(fixture: SimulatedFixture) -> SimulatedGpus {
    SimulatedGpus::new(fixture, || Duration::ZERO)
}

/// The snapshot from the shared contract, which describes the simulated tunable GPU.
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

/// Prism connected to prismd on `snapshot`, with one sample of `fixture` from prismd.
fn connected_to(fixture: SimulatedFixture, snapshot: DaemonSnapshot, can_tune: bool) -> Core {
    let mut core = Core::default();
    core.handle(Input::Launched {
        appearance: AppearanceMode::System,
    });
    core.handle(Input::PrismdConnected {
        welcome: welcome(can_tune),
        snapshot,
        socket: "/run/prism/prismd.sock".into(),
    });
    core.handle(Input::PrismdEvent {
        event: DaemonEvent::TelemetrySampled(simulated(fixture).performance()),
        received_at: Duration::from_secs(1),
    });
    core
}

/// Prism with tuning access to the tunable GPU, whose power limit is 575 W in a 400–600 W range.
fn connected() -> Core {
    connected_to(SimulatedFixture::TunableGpu, snapshot(), true)
}

/// Prism with tuning access to two identical GPUs.
fn connected_to_two() -> (Core, [String; 2]) {
    let gpus = simulated(SimulatedFixture::TwoIdenticalGpus);
    let mut snapshot = snapshot();
    snapshot.inventory = gpus.inventory();
    let template = snapshot.tuning_states[0].clone();
    snapshot.tuning_states = snapshot
        .inventory
        .gpus
        .iter()
        .map(|gpu| GpuTuningState {
            gpu_id: gpu.id.clone(),
            ..template.clone()
        })
        .collect();
    let ids = [
        snapshot.inventory.gpus[0].id.clone(),
        snapshot.inventory.gpus[1].id.clone(),
    ];
    (
        connected_to(SimulatedFixture::TwoIdenticalGpus, snapshot, true),
        ids,
    )
}

fn intent(core: &mut Core, intent: Intent) -> Vec<Effect> {
    core.handle(Input::Intent(intent))
}

fn workspace(core: &Core) -> GpuWorkspace {
    match core.content() {
        Content::Gpu(workspace) => *workspace,
        other => panic!("expected the GPU workspace, got {other:?}"),
    }
}

/// The power limit's stepper as the power track shows it.
fn power_limit(core: &Core) -> prism_ui::Stepper {
    workspace(core)
        .track(TrackKind::Power)
        .stepper
        .clone()
        .expect("the power track has the power limit's stepper")
}

/// The pending value next to the power limit, such as "→ 450 W".
fn pending(core: &Core) -> Option<String> {
    power_limit(core).pending
}

/// The edit holds sent to prismd among `effects`.
fn edit_holds(effects: &[Effect]) -> Vec<Vec<String>> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::SendToPrismd(DaemonRequest::SetEditHolds(SetEditHoldsRequest { gpu_ids })) => {
                Some(gpu_ids.clone())
            }
            _ => None,
        })
        .collect()
}

/// The Applies sent to prismd among `effects`, each with the command it answers.
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

fn watts(value: i64) -> ProfileSettings {
    ProfileSettings {
        power_limit_watts: Some(ProfileSetting::Value { value }),
        ..ProfileSettings::default()
    }
}

/// Stages `watts` and applies it, returning the one Apply sent.
fn stage_and_apply(core: &mut Core, watts: i64) -> (CommandId, ApplyGpuProfileRequest) {
    intent(core, Intent::StagePowerLimit(watts));
    let sent = applies(&intent(core, Intent::ApplyChanges));
    assert_eq!(sent.len(), 1, "{sent:?}");
    sent.into_iter().next().unwrap()
}

/// prismd's answer that the power limit was applied as `status`, with `error`.
fn power_outcome(status: TuningOutcomeStatus, error: Option<&str>) -> DaemonResponse {
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

/// prismd pushes the GPU's tuning state at `revision` with its power limit at `watts`.
fn tuning_changed(core: &mut Core, gpu_id: &str, revision: &str, watts: i64) {
    core.handle(Input::PrismdEvent {
        event: DaemonEvent::TuningStateChanged(GpuTuningState {
            gpu_id: gpu_id.into(),
            revision: revision.into(),
            reported: ReportedTuningValues {
                cooling: None,
                power_limit_watts: Some(watts),
                graphics_offset_mhz: Some(0),
                memory_offset_mhz: Some(0),
            },
            current_profile: None,
        }),
        received_at: Duration::from_secs(2),
    });
}

#[test]
fn a_staged_power_limit_is_pending_on_its_control_and_its_chart_and_holds_automatic_changes() {
    let mut core = connected();
    assert_eq!(workspace(&core).changes.summary, "No unapplied changes");
    assert!(!core.status_bar().edit_hold);

    let effects = intent(&mut core, Intent::StagePowerLimit(450));

    assert_eq!(edit_holds(&effects), [vec![GPU.to_owned()]]);
    assert_eq!(pending(&core).as_deref(), Some("→ 450 W"));
    assert_eq!(power_limit(&core).value, 450);
    let changes = workspace(&core).changes;
    assert!(changes.pending);
    assert_eq!(changes.summary, "1 unapplied change");
    let power = workspace(&core).track(TrackKind::Power).clone();
    let chart = &power.charts[0];
    let pending_line = chart
        .limits
        .iter()
        .find(|limit| limit.kind == LimitKind::Pending)
        .expect("the pending limit is drawn");
    assert_eq!(pending_line.y, chart.chart.y(450));
    assert!(core.status_bar().edit_hold);
}

#[test]
fn a_staged_power_limit_stays_within_the_allowed_range() {
    let mut core = connected();

    intent(&mut core, Intent::StagePowerLimit(9_999));
    assert_eq!(pending(&core).as_deref(), Some("→ 600 W"));

    intent(&mut core, Intent::StagePowerLimit(0));
    assert_eq!(pending(&core).as_deref(), Some("→ 400 W"));
    let stepper = power_limit(&core);
    assert_eq!((stepper.min, stepper.max), (400, 600));
}

#[test]
fn dragging_the_limit_line_stages_the_value_where_it_is_let_go() {
    let mut core = connected();
    let chart = workspace(&core).track(TrackKind::Power).charts[0]
        .chart
        .clone();

    intent(&mut core, Intent::DragPowerLimit(chart.y(450)));

    assert_eq!(pending(&core).as_deref(), Some("→ 450 W"));
}

#[test]
fn staging_the_value_in_effect_leaves_nothing_to_apply_and_releases_the_edit_hold() {
    let mut core = connected();
    intent(&mut core, Intent::StagePowerLimit(450));

    let effects = intent(&mut core, Intent::StagePowerLimit(575));

    assert_eq!(pending(&core), None);
    assert!(!workspace(&core).changes.pending);
    assert_eq!(edit_holds(&effects), [Vec::<String>::new()]);
    assert!(!core.status_bar().edit_hold);
}

#[test]
fn apply_sends_one_apply_profile_made_against_the_revision_the_draft_started_from() {
    let mut core = connected();
    intent(&mut core, Intent::StagePowerLimit(450));
    // Changed outside Prism while the draft was open.
    tuning_changed(&mut core, GPU, "changed", 500);

    let effects = intent(&mut core, Intent::ApplyChanges);

    let sent = applies(&effects);
    assert_eq!(sent.len(), 1, "{effects:?}");
    assert_eq!(
        sent[0].1,
        ApplyGpuProfileRequest {
            gpu_id: GPU.into(),
            settings: watts(450),
            expected_revision: REVISION.into(),
            fan_indexes: None,
        }
    );
    // A second Apply waits for the first's answer.
    assert!(applies(&intent(&mut core, Intent::ApplyChanges)).is_empty());
}

#[test]
fn an_applied_draft_is_gone_with_its_edit_hold_and_the_values_come_from_prismd() {
    let mut core = connected();
    let (id, _) = stage_and_apply(&mut core, 450);
    tuning_changed(&mut core, GPU, "applied", 450);

    let effects = core.handle(Input::PrismdAnswered {
        id,
        response: power_outcome(TuningOutcomeStatus::Succeeded, None),
    });

    assert_eq!(pending(&core), None);
    assert!(!workspace(&core).changes.pending);
    assert_eq!(edit_holds(&effects), [Vec::<String>::new()]);
    assert!(!core.status_bar().edit_hold);
}

#[test]
fn a_failed_setting_stays_pending_with_why_for_a_retry_against_the_new_state() {
    let mut core = connected();
    let (id, _) = stage_and_apply(&mut core, 450);

    let effects = core.handle(Input::PrismdAnswered {
        id,
        response: power_outcome(
            TuningOutcomeStatus::Failed,
            Some("Insufficient permissions"),
        ),
    });

    assert!(edit_holds(&effects).is_empty());
    assert_eq!(pending(&core).as_deref(), Some("→ 450 W"));
    let changes = workspace(&core).changes;
    assert!(changes.pending);
    assert_eq!(
        changes.problem.as_deref(),
        Some("Power limit: Insufficient permissions")
    );
    let retry = applies(&intent(&mut core, Intent::ApplyChanges));
    assert_eq!(retry[0].1.settings, watts(450));
}

#[test]
fn a_stale_apply_keeps_the_draft_for_review_and_bases_the_next_apply_on_what_changed() {
    let mut core = connected();
    let (id, _) = stage_and_apply(&mut core, 450);
    tuning_changed(&mut core, GPU, "changed", 500);

    core.handle(Input::PrismdAnswered {
        id,
        response: DaemonResponse::Error(GpuCommandError::new(
            GpuCommandErrorCode::StaleRevision,
            "The GPU's tuning values changed since these changes were made. Review them, then apply again.",
        )),
    });

    assert_eq!(pending(&core).as_deref(), Some("→ 450 W"));
    assert!(
        workspace(&core)
            .changes
            .problem
            .is_some_and(|problem| problem.contains("Review them"))
    );
    let retry = applies(&intent(&mut core, Intent::ApplyChanges));
    assert_eq!(retry[0].1.expected_revision, "changed");
}

#[test]
fn discard_drops_the_draft_and_releases_its_edit_hold() {
    let mut core = connected();
    intent(&mut core, Intent::StagePowerLimit(450));

    let effects = intent(&mut core, Intent::DiscardChanges);

    assert_eq!(pending(&core), None);
    assert_eq!(edit_holds(&effects), [Vec::<String>::new()]);
    assert!(!core.status_bar().edit_hold);
}

#[test]
fn drafts_and_their_edit_holds_survive_switching_gpus() {
    let (mut core, [first, second]) = connected_to_two();
    intent(&mut core, Intent::SelectGpu(first.clone()));
    intent(&mut core, Intent::StagePowerLimit(450));

    let effects = intent(&mut core, Intent::SelectGpu(second.clone()));
    assert!(edit_holds(&effects).is_empty());
    assert_eq!(pending(&core), None);
    assert!(core.status_bar().edit_hold);
    let effects = intent(&mut core, Intent::StagePowerLimit(500));
    assert_eq!(edit_holds(&effects), [vec![first.clone(), second.clone()]]);

    intent(&mut core, Intent::SelectGpu(first.clone()));
    assert_eq!(pending(&core).as_deref(), Some("→ 450 W"));
    let effects = intent(&mut core, Intent::DiscardChanges);
    assert_eq!(edit_holds(&effects), [vec![second]]);
    assert!(core.status_bar().edit_hold);
}

#[test]
fn edit_holds_are_sent_again_when_prismd_comes_back() {
    let mut core = connected();
    intent(&mut core, Intent::StagePowerLimit(450));
    core.handle(Input::PrismdNotConnected(SessionFailure::Unavailable {
        message: "The session with prismd ended".into(),
    }));

    let effects = core.handle(Input::PrismdConnected {
        welcome: welcome(true),
        snapshot: snapshot(),
        socket: "/run/prism/prismd.sock".into(),
    });

    assert_eq!(edit_holds(&effects), [vec![GPU.to_owned()]]);
    assert_eq!(pending(&core).as_deref(), Some("→ 450 W"));
}

#[test]
fn without_tuning_access_the_power_limit_is_read_only_with_the_reason() {
    let mut core = connected_to(SimulatedFixture::TunableGpu, snapshot(), false);

    let effects = intent(&mut core, Intent::StagePowerLimit(450));

    assert!(effects.is_empty());
    let stepper = power_limit(&core);
    assert_eq!(stepper.value, 575);
    assert_eq!(
        stepper.read_only.as_deref(),
        Some("bob isn't in group wheel")
    );
    assert_eq!(pending(&core), None);

    let mut core = Core::default();
    core.handle(Input::Launched {
        appearance: AppearanceMode::System,
    });
    core.handle(Input::PrismdNotConnected(SessionFailure::Unavailable {
        message: "No such file or directory (os error 2)".into(),
    }));
    let gpus = simulated(SimulatedFixture::TunableGpu);
    core.handle(Input::InventoryRead(gpus.inventory()));
    core.handle(Input::TelemetrySampled {
        gpus: gpus.performance(),
        source: TelemetrySource::InProcess,
        received_at: Duration::from_secs(1),
    });
    assert_eq!(power_limit(&core).read_only.as_deref(), Some("not running"));
    assert!(intent(&mut core, Intent::StagePowerLimit(450)).is_empty());
}

#[test]
fn ctrl_enter_applies_and_ctrl_shift_backspace_discards() {
    let key = |text: &str, shift| Key {
        text: text.into(),
        control: true,
        shift,
        ..Key::default()
    };
    assert_eq!(Shortcut::for_key(&key("\n", false)), Some(Shortcut::Apply));
    assert_eq!(
        Shortcut::for_key(&key("\u{8}", true)),
        Some(Shortcut::Discard)
    );
    assert!(Shortcut::Discard.clashes_with_text_editing());
    let mut core = connected();
    intent(&mut core, Intent::StagePowerLimit(450));

    assert_eq!(
        applies(&intent(&mut core, Intent::Press(Shortcut::Apply))).len(),
        1
    );

    let mut core = connected();
    intent(&mut core, Intent::StagePowerLimit(450));
    intent(&mut core, Intent::Press(Shortcut::Discard));
    assert_eq!(pending(&core), None);
}

#[test]
fn the_palette_applies_and_discards_and_says_why_it_cant() {
    let mut core = connected();
    intent(&mut core, Intent::OpenPalette);
    let entry = |core: &Core, title: &str| {
        core.palette()
            .unwrap()
            .entries
            .into_iter()
            .find(|entry| entry.title == title)
            .unwrap_or_else(|| panic!("no entry is titled {title:?}"))
    };
    let apply = entry(&core, "Apply unapplied changes");
    assert_eq!(apply.unavailable.as_deref(), Some("No unapplied changes"));
    assert_eq!(apply.shortcut(), "Ctrl+Enter");
    assert_eq!(
        entry(&core, "Discard unapplied changes").shortcut(),
        "Ctrl+Shift+Backspace"
    );
    intent(&mut core, Intent::Dismiss);

    intent(&mut core, Intent::StagePowerLimit(450));
    intent(&mut core, Intent::OpenPalette);
    intent(&mut core, Intent::FilterPalette("apply unapplied".into()));
    let effects = intent(&mut core, Intent::RunHighlighted);
    assert_eq!(applies(&effects).len(), 1);

    core.handle(Input::PrismdNotConnected(SessionFailure::Unavailable {
        message: "The session with prismd ended".into(),
    }));
    intent(&mut core, Intent::OpenPalette);
    assert_eq!(
        entry(&core, "Apply unapplied changes")
            .unavailable
            .as_deref(),
        Some("Needs tuning access · not running")
    );
    assert_eq!(entry(&core, "Discard unapplied changes").unavailable, None);
}
