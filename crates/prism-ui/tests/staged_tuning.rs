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
    GpuTuningState, ProfileApplicationResponse, ProfileSetting, ProfileSettingKey, ProfileSettings,
    ProtocolVersion, ReportedTuningValues, SessionMessage, SetEditHoldsRequest, Welcome,
};
use prism_ui::telemetry::LimitKind;
use prism_ui::{
    CommandId, Content, Core, Dialog, Effect, GpuWorkspace, GuiSettings, Input, Intent, Key,
    SessionFailure, Shortcut, TelemetrySource, TrackKind,
};

use ProfileSettingKey::{GraphicsOffsetMhz, MemoryOffsetMhz, PowerLimitWatts};

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
        settings: GuiSettings::default(),
        started_unix_ms: 0,
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

/// The stepper that tunes `setting`, as its track shows it.
fn stepper(core: &Core, setting: ProfileSettingKey) -> prism_ui::Stepper {
    workspace(core)
        .stepper(setting)
        .cloned()
        .unwrap_or_else(|| panic!("no track has a stepper for {setting:?}"))
}

/// The power limit's stepper as the power track shows it.
fn power_limit(core: &Core) -> prism_ui::Stepper {
    stepper(core, PowerLimitWatts)
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
    intent(core, Intent::Stage(PowerLimitWatts, watts));
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
            defaults: Vec::new(),
            reported: ReportedTuningValues {
                cooling: Some(prism_gpu::cooling::CoolingConfiguration::Shared {
                    setting: prism_gpu::cooling::FanSetting::Automatic {},
                }),
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

    let effects = intent(&mut core, Intent::Stage(PowerLimitWatts, 450));

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

    intent(&mut core, Intent::Stage(PowerLimitWatts, 9_999));
    assert_eq!(pending(&core).as_deref(), Some("→ 600 W"));

    intent(&mut core, Intent::Stage(PowerLimitWatts, 0));
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
    intent(&mut core, Intent::Stage(PowerLimitWatts, 450));

    let effects = intent(&mut core, Intent::Stage(PowerLimitWatts, 575));

    assert_eq!(pending(&core), None);
    assert!(!workspace(&core).changes.pending);
    assert_eq!(edit_holds(&effects), [Vec::<String>::new()]);
    assert!(!core.status_bar().edit_hold);
}

#[test]
fn apply_sends_one_apply_profile_made_against_the_revision_the_draft_started_from() {
    let mut core = connected();
    intent(&mut core, Intent::Stage(PowerLimitWatts, 450));
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
fn a_failed_setting_stays_pending_with_why_for_a_retry_against_the_same_state() {
    let mut core = connected();
    let (id, _) = stage_and_apply(&mut core, 450);
    // Changed outside Prism while the Apply was on its way: its answer can overtake this.
    tuning_changed(&mut core, GPU, "changed", 500);

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
        changes.outcome.as_deref(),
        Some("Power limit not applied: Insufficient permissions")
    );
    let retry = applies(&intent(&mut core, Intent::ApplyChanges));
    assert_eq!(retry[0].1.settings, watts(450));
    // Still made against the state the draft started from, so prismd refuses it for review.
    assert_eq!(retry[0].1.expected_revision, REVISION);
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
            .outcome
            .is_some_and(|outcome| outcome.contains("Review them"))
    );
    let retry = applies(&intent(&mut core, Intent::ApplyChanges));
    assert_eq!(retry[0].1.expected_revision, "changed");
}

#[test]
fn discard_drops_the_draft_and_releases_its_edit_hold() {
    let mut core = connected();
    intent(&mut core, Intent::Stage(PowerLimitWatts, 450));

    let effects = intent(&mut core, Intent::DiscardChanges);

    assert_eq!(pending(&core), None);
    assert_eq!(edit_holds(&effects), [Vec::<String>::new()]);
    assert!(!core.status_bar().edit_hold);
}

#[test]
fn drafts_and_their_edit_holds_survive_switching_gpus() {
    let (mut core, [first, second]) = connected_to_two();
    intent(&mut core, Intent::SelectGpu(first.clone()));
    intent(&mut core, Intent::Stage(PowerLimitWatts, 450));

    let effects = intent(&mut core, Intent::SelectGpu(second.clone()));
    assert!(edit_holds(&effects).is_empty());
    assert_eq!(pending(&core), None);
    assert!(core.status_bar().edit_hold);
    let effects = intent(&mut core, Intent::Stage(PowerLimitWatts, 500));
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
    intent(&mut core, Intent::Stage(PowerLimitWatts, 450));
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

    let effects = intent(&mut core, Intent::Stage(PowerLimitWatts, 450));

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
        settings: GuiSettings::default(),
        started_unix_ms: 0,
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
    assert!(intent(&mut core, Intent::Stage(PowerLimitWatts, 450)).is_empty());
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
    intent(&mut core, Intent::Stage(PowerLimitWatts, 450));

    assert_eq!(
        applies(&intent(&mut core, Intent::Press(Shortcut::Apply))).len(),
        1
    );

    let mut core = connected();
    intent(&mut core, Intent::Stage(PowerLimitWatts, 450));
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

    intent(&mut core, Intent::Stage(PowerLimitWatts, 450));
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

/// prismd's answer to an Apply, with each setting's outcome.
fn outcomes(
    power: Option<TuningOutcome>,
    graphics: Option<TuningOutcome>,
    memory: Option<TuningOutcome>,
) -> DaemonResponse {
    DaemonResponse::ProfileApplication(ProfileApplicationResponse {
        rollback: None,
        cooling: None,
        power,
        graphics,
        memory,
        corrections: Vec::new(),
    })
}

fn outcome(status: TuningOutcomeStatus, error: Option<&str>) -> Option<TuningOutcome> {
    Some(TuningOutcome {
        status,
        error: error.map(Into::into),
    })
}

/// The one Apply among `effects`.
fn the_apply(effects: &[Effect]) -> (CommandId, ApplyGpuProfileRequest) {
    let sent = applies(effects);
    assert_eq!(sent.len(), 1, "{effects:?}");
    sent.into_iter().next().unwrap()
}

fn stale() -> DaemonResponse {
    DaemonResponse::Error(GpuCommandError::new(
        GpuCommandErrorCode::StaleRevision,
        "The GPU's tuning values changed since these changes were made. Review them, then apply again. No GPU settings were changed.",
    ))
}

#[test]
fn clock_offsets_stage_within_their_range_and_apply_with_the_power_limit_in_one_apply() {
    let mut core = connected();
    let clocks = workspace(&core).track(TrackKind::Clocks).clone();
    let graphics = stepper(&core, GraphicsOffsetMhz);
    assert!(
        clocks
            .details
            .iter()
            .any(|detail| detail.stepper.as_ref() == Some(&graphics)),
        "the Clocks track has the graphics clock offset's stepper"
    );
    assert_eq!((graphics.min, graphics.max), (-1_000, 1_000));
    assert_eq!(graphics.readout.text(), "0 MHz");
    assert_eq!(graphics.read_only, None);

    intent(&mut core, Intent::Stage(GraphicsOffsetMhz, 150));
    intent(&mut core, Intent::Stage(MemoryOffsetMhz, 9_000));
    intent(&mut core, Intent::Stage(PowerLimitWatts, 450));

    assert_eq!(
        stepper(&core, GraphicsOffsetMhz).pending.as_deref(),
        Some("→ +150 MHz")
    );
    assert_eq!(
        stepper(&core, MemoryOffsetMhz).pending.as_deref(),
        Some("→ +6000 MHz")
    );
    assert_eq!(workspace(&core).changes.summary, "3 unapplied changes");
    let (_, apply) = the_apply(&intent(&mut core, Intent::ApplyChanges));
    assert_eq!(
        apply.settings,
        ProfileSettings {
            power_limit_watts: Some(ProfileSetting::Value { value: 450 }),
            graphics_offset_mhz: Some(ProfileSetting::Value { value: 150 }),
            memory_offset_mhz: Some(ProfileSetting::Value { value: 6_000 }),
            cooling: None,
        }
    );
}

#[test]
fn use_default_stages_a_default_instruction_that_reads_as_default_and_applies_as_one() {
    let mut core = connected();
    assert!(stepper(&core, GraphicsOffsetMhz).can_use_default);
    intent(&mut core, Intent::Stage(GraphicsOffsetMhz, 150));

    let effects = intent(&mut core, Intent::UseDefault(GraphicsOffsetMhz));
    intent(&mut core, Intent::UseDefault(PowerLimitWatts));

    // Already the draft's edit hold: the draft only changed.
    assert!(edit_holds(&effects).is_empty());
    let graphics = stepper(&core, GraphicsOffsetMhz);
    assert_eq!(graphics.pending.as_deref(), Some("→ default"));
    assert!(!graphics.can_use_default);
    // An offset's default is no offset.
    assert_eq!(graphics.value, 0);
    // Even with the power limit at its default already, the choice is the default itself.
    let power = power_limit(&core);
    assert_eq!(power.pending.as_deref(), Some("→ default"));
    assert_eq!(power.value, 575);
    assert_eq!(workspace(&core).changes.summary, "2 unapplied changes");
    let (_, apply) = the_apply(&intent(&mut core, Intent::ApplyChanges));
    assert_eq!(
        apply.settings,
        ProfileSettings {
            power_limit_watts: Some(ProfileSetting::Default {}),
            graphics_offset_mhz: Some(ProfileSetting::Default {}),
            ..ProfileSettings::default()
        }
    );
}

#[test]
fn a_value_staged_after_use_default_replaces_the_default() {
    let mut core = connected();
    intent(&mut core, Intent::UseDefault(MemoryOffsetMhz));

    intent(&mut core, Intent::Stage(MemoryOffsetMhz, 500));

    let memory = stepper(&core, MemoryOffsetMhz);
    assert_eq!(memory.pending.as_deref(), Some("→ +500 MHz"));
    assert!(memory.can_use_default);
}

#[test]
fn without_tuning_access_use_default_does_nothing() {
    let mut core = connected_to(SimulatedFixture::TunableGpu, snapshot(), false);

    assert!(intent(&mut core, Intent::UseDefault(GraphicsOffsetMhz)).is_empty());

    let graphics = stepper(&core, GraphicsOffsetMhz);
    assert!(!graphics.can_use_default);
    assert_eq!(
        graphics.read_only.as_deref(),
        Some("bob isn't in group wheel")
    );
    assert!(!workspace(&core).changes.pending);
}

#[test]
fn a_partial_apply_marks_what_succeeded_and_failed_and_keeps_the_failed_edits_pending() {
    let mut core = connected();
    intent(&mut core, Intent::Stage(PowerLimitWatts, 450));
    intent(&mut core, Intent::Stage(GraphicsOffsetMhz, 150));
    intent(&mut core, Intent::Stage(MemoryOffsetMhz, 500));
    let (id, _) = the_apply(&intent(&mut core, Intent::ApplyChanges));

    let effects = core.handle(Input::PrismdAnswered {
        id,
        response: outcomes(
            outcome(TuningOutcomeStatus::Succeeded, None),
            outcome(
                TuningOutcomeStatus::Failed,
                Some("The driver refused the offset"),
            ),
            outcome(TuningOutcomeStatus::Unattempted, None),
        ),
    });

    // Never blanket success: the draft and its edit hold stay for what failed.
    assert!(edit_holds(&effects).is_empty());
    assert!(core.status_bar().edit_hold);
    let changes = workspace(&core).changes;
    assert_eq!(changes.summary, "2 unapplied changes");
    assert_eq!(
        changes.outcome.as_deref(),
        Some(
            "Power limit applied · Graphics clock offset not applied: The driver refused the \
             offset · Memory clock offset not applied"
        )
    );
    assert_eq!(pending(&core), None);
    assert_eq!(power_limit(&core).failed, None);
    let graphics = stepper(&core, GraphicsOffsetMhz);
    assert_eq!(graphics.pending.as_deref(), Some("→ +150 MHz"));
    assert_eq!(
        graphics.failed.as_deref(),
        Some("Not applied: The driver refused the offset")
    );
    assert_eq!(
        stepper(&core, MemoryOffsetMhz).failed.as_deref(),
        Some("Not applied")
    );

    // A retry sends only what's still pending.
    let (_, retry) = the_apply(&intent(&mut core, Intent::ApplyChanges));
    assert_eq!(
        retry.settings,
        ProfileSettings {
            graphics_offset_mhz: Some(ProfileSetting::Value { value: 150 }),
            memory_offset_mhz: Some(ProfileSetting::Value { value: 500 }),
            ..ProfileSettings::default()
        }
    );
}

#[test]
fn editing_a_failed_setting_clears_its_failure_and_the_last_outcome() {
    let mut core = connected();
    intent(&mut core, Intent::Stage(GraphicsOffsetMhz, 150));
    intent(&mut core, Intent::Stage(MemoryOffsetMhz, 500));
    let (id, _) = the_apply(&intent(&mut core, Intent::ApplyChanges));
    core.handle(Input::PrismdAnswered {
        id,
        response: outcomes(
            None,
            outcome(TuningOutcomeStatus::Failed, Some("Refused")),
            outcome(TuningOutcomeStatus::Failed, Some("Refused")),
        ),
    });

    intent(&mut core, Intent::Stage(GraphicsOffsetMhz, 100));

    assert_eq!(stepper(&core, GraphicsOffsetMhz).failed, None);
    // The other edit still failed last time.
    assert_eq!(
        stepper(&core, MemoryOffsetMhz).failed.as_deref(),
        Some("Not applied: Refused")
    );
    assert_eq!(workspace(&core).changes.outcome, None);
}

#[test]
fn a_stale_apply_opens_a_review_of_what_changed_and_applying_again_uses_the_new_revision() {
    let mut core = connected();
    let (id, _) = stage_and_apply(&mut core, 450);
    tuning_changed(&mut core, GPU, "changed", 500);

    core.handle(Input::PrismdAnswered {
        id,
        response: stale(),
    });

    let Some(Dialog::Review(review)) = core.dialog() else {
        panic!("expected the review, got {:?}", core.dialog())
    };
    assert_eq!(
        review.title,
        "GPU 0 · RTX 5090 · 01:00.0 changed since these changes were made"
    );
    assert_eq!(
        review.message,
        "Another Prism, automatic switching or a change outside Prism changed these values. \
         Apply your changes over them, or discard them."
    );
    assert_eq!(review.changes.len(), 1, "{review:?}");
    let change = &review.changes[0];
    assert_eq!(change.name, "Power limit");
    assert_eq!(change.change, "575 W → 500 W");
    assert_eq!(change.yours.as_deref(), Some("450 W"));
    assert_eq!(review.apply_unavailable, None);
    // The draft stays as it was.
    assert_eq!(pending(&core).as_deref(), Some("→ 450 W"));

    let (_, apply) = the_apply(&intent(&mut core, Intent::ApplyChanges));
    assert_eq!(apply.expected_revision, "changed");
    assert_eq!(apply.settings, watts(450));
    assert_eq!(core.dialog(), None);
}

#[test]
fn a_review_compares_with_the_values_the_draft_started_from() {
    let mut core = connected();
    intent(&mut core, Intent::Stage(GraphicsOffsetMhz, 150));
    // Changed by automatic switching, say, while the draft was open: offsets and power both.
    core.handle(Input::PrismdEvent {
        event: DaemonEvent::TuningStateChanged(GpuTuningState {
            gpu_id: GPU.into(),
            revision: "switched".into(),
            defaults: Vec::new(),
            reported: ReportedTuningValues {
                cooling: Some(prism_gpu::cooling::CoolingConfiguration::Shared {
                    setting: prism_gpu::cooling::FanSetting::Automatic {},
                }),
                power_limit_watts: Some(500),
                graphics_offset_mhz: Some(100),
                memory_offset_mhz: Some(0),
            },
            current_profile: None,
        }),
        received_at: Duration::from_secs(2),
    });
    let (id, _) = the_apply(&intent(&mut core, Intent::ApplyChanges));

    core.handle(Input::PrismdAnswered {
        id,
        response: stale(),
    });

    let Some(Dialog::Review(review)) = core.dialog() else {
        panic!("expected the review, got {:?}", core.dialog())
    };
    let changes: Vec<_> = review
        .changes
        .iter()
        .map(|change| {
            (
                change.name.as_str(),
                change.change.as_str(),
                change.yours.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        changes,
        [
            ("Power limit", "575 W → 500 W", None),
            (
                "Graphics clock offset",
                "0 MHz → +100 MHz",
                Some("+150 MHz")
            ),
        ]
    );
}

#[test]
fn discarding_from_the_review_drops_the_draft_and_closes_it() {
    let mut core = connected();
    let (id, _) = stage_and_apply(&mut core, 450);
    tuning_changed(&mut core, GPU, "changed", 500);
    core.handle(Input::PrismdAnswered {
        id,
        response: stale(),
    });

    let effects = intent(&mut core, Intent::DiscardChanges);

    assert_eq!(core.dialog(), None);
    assert_eq!(pending(&core), None);
    assert_eq!(edit_holds(&effects), [Vec::<String>::new()]);
}

#[test]
fn dismissing_the_review_keeps_editing_on_the_new_revision() {
    let mut core = connected();
    let (id, _) = stage_and_apply(&mut core, 450);
    tuning_changed(&mut core, GPU, "changed", 500);
    core.handle(Input::PrismdAnswered {
        id,
        response: stale(),
    });

    intent(&mut core, Intent::Press(Shortcut::Dismiss));

    assert_eq!(core.dialog(), None);
    assert_eq!(pending(&core).as_deref(), Some("→ 450 W"));
    let (_, apply) = the_apply(&intent(&mut core, Intent::ApplyChanges));
    assert_eq!(apply.expected_revision, "changed");
}

#[test]
fn a_review_acts_on_its_own_gpu_while_another_is_shown() {
    let (mut core, [first, second]) = connected_to_two();
    intent(&mut core, Intent::SelectGpu(first.clone()));
    let (id, _) = stage_and_apply(&mut core, 400);
    intent(&mut core, Intent::SelectGpu(second.clone()));
    tuning_changed(&mut core, &first, "changed", 425);

    core.handle(Input::PrismdAnswered {
        id,
        response: stale(),
    });
    assert!(matches!(core.dialog(), Some(Dialog::Review(_))));
    // Nothing else opens over it.
    intent(&mut core, Intent::OpenPalette);
    assert!(!core.palette_open());

    let (_, apply) = the_apply(&intent(&mut core, Intent::ApplyChanges));
    assert_eq!(apply.gpu_id, first);
    assert_eq!(apply.expected_revision, "changed");
}

#[test]
fn closing_with_unapplied_changes_asks_first_and_keep_editing_cancels_the_close() {
    let (mut core, [first, second]) = connected_to_two();
    intent(&mut core, Intent::SelectGpu(first));
    intent(&mut core, Intent::Stage(PowerLimitWatts, 400));
    intent(&mut core, Intent::SelectGpu(second));
    intent(&mut core, Intent::Stage(PowerLimitWatts, 400));
    intent(&mut core, Intent::Stage(GraphicsOffsetMhz, 100));

    let effects = intent(&mut core, Intent::CloseWindow);

    assert!(!effects.contains(&Effect::Quit), "{effects:?}");
    let Some(Dialog::ConfirmClose(confirm)) = core.dialog() else {
        panic!("expected the close confirmation, got {:?}", core.dialog())
    };
    // Every GPU's changes count.
    assert_eq!(confirm.summary, "3 unapplied changes");

    let effects = intent(&mut core, Intent::Dismiss);
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(core.dialog(), None);
    assert!(core.status_bar().edit_hold);

    // Ctrl+Q asks the same.
    let effects = intent(&mut core, Intent::Press(Shortcut::Quit));
    assert!(!effects.contains(&Effect::Quit), "{effects:?}");
    assert!(matches!(core.dialog(), Some(Dialog::ConfirmClose(_))));

    let effects = intent(&mut core, Intent::DiscardAndClose);
    assert_eq!(effects.last(), Some(&Effect::Quit));
}

#[test]
fn closing_without_unapplied_changes_quits_at_once() {
    let mut core = connected();

    assert_eq!(intent(&mut core, Intent::CloseWindow), [Effect::Quit]);
    assert_eq!(core.dialog(), None);
}

/// Stages a power limit and a graphics clock offset and applies them; prismd puts the power
/// limit into effect and refuses the offset.
fn partly_applied(core: &mut Core) -> CommandId {
    intent(core, Intent::Stage(PowerLimitWatts, 450));
    intent(core, Intent::Stage(GraphicsOffsetMhz, 150));
    the_apply(&intent(core, Intent::ApplyChanges)).0
}

fn power_applied_offset_refused() -> DaemonResponse {
    outcomes(
        outcome(TuningOutcomeStatus::Succeeded, None),
        outcome(TuningOutcomeStatus::Failed, Some("Refused")),
        None,
    )
}

#[test]
fn a_retry_after_a_partial_apply_is_made_against_the_state_its_own_write_pushed() {
    let mut core = connected();
    let id = partly_applied(&mut core);

    core.handle(Input::PrismdAnswered {
        id,
        response: power_applied_offset_refused(),
    });
    // The write's own event arrives after its answer.
    tuning_changed(&mut core, GPU, "applied", 450);

    assert_eq!(core.dialog(), None);
    let (_, retry) = the_apply(&intent(&mut core, Intent::ApplyChanges));
    assert_eq!(retry.expected_revision, "applied");
}

#[test]
fn a_retry_after_a_partial_apply_whose_event_came_first_is_made_against_that_state() {
    let mut core = connected();
    let id = partly_applied(&mut core);
    tuning_changed(&mut core, GPU, "applied", 450);

    core.handle(Input::PrismdAnswered {
        id,
        response: power_applied_offset_refused(),
    });

    let (_, retry) = the_apply(&intent(&mut core, Intent::ApplyChanges));
    assert_eq!(retry.expected_revision, "applied");
}

#[test]
fn a_stale_apply_answered_before_the_change_reaches_prism_opens_the_review_once_it_does() {
    let mut core = connected();
    let (id, _) = stage_and_apply(&mut core, 450);

    core.handle(Input::PrismdAnswered {
        id,
        response: stale(),
    });
    // prismd saw the change before it pushed it.
    assert_eq!(core.dialog(), None);
    tuning_changed(&mut core, GPU, "changed", 500);

    let Some(Dialog::Review(review)) = core.dialog() else {
        panic!("expected the review, got {:?}", core.dialog())
    };
    assert_eq!(review.changes[0].change, "575 W → 500 W");
    let (_, apply) = the_apply(&intent(&mut core, Intent::ApplyChanges));
    assert_eq!(apply.expected_revision, "changed");
}
