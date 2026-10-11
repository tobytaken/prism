//! Cooling in the staged loop: the Fans track's fan control mode and fixed speed, staged in the
//! GPU's draft and applied with the reviewed fans, through the core's input/effect interface.

use std::time::Duration;

use prism_gpu::cooling::{
    CoolingApplication, CoolingConfiguration, CurveSuspension, FanOutcome, FanSetting,
    SuspensionKind,
};
use prism_gpu::fan_curve::{self, FanCurveSupport};
use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_gpu::{GpuPerformanceResponse, GpuTelemetryProvider, TuningOutcome, TuningOutcomeStatus};
use prism_protocol::{
    ApplyGpuProfileRequest, DaemonEvent, DaemonRequest, DaemonResponse, DaemonSnapshot,
    GpuCoolingControl, ProfileApplicationResponse, ProfileSettings, ProtocolVersion,
    SessionMessage, SetEditHoldsRequest, Welcome,
};
use prism_ui::{
    AppearanceMode, CommandId, Content, Core, Effect, FanMode, FanModeControl, GpuWorkspace, Input,
    Intent, Stepper,
};

const GPU: &str = "GPU-5a1b0000-0000-4000-8000-000000000001";
/// The tunable GPU's tuning-state revision in the shared snapshot.
const REVISION: &str = "4f1c2b9d0e8a7c35";

fn reading(fixture: SimulatedFixture) -> GpuPerformanceResponse {
    SimulatedGpus::new(fixture, || Duration::ZERO).performance()
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

/// Prism connected to prismd on `snapshot`, with `sample` from prismd.
fn connected_with(
    snapshot: DaemonSnapshot,
    sample: GpuPerformanceResponse,
    can_tune: bool,
) -> Core {
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
        event: DaemonEvent::TelemetrySampled(sample),
        received_at: Duration::from_secs(1),
    });
    core
}

/// Prism with tuning access to the tunable GPU, whose two fans run at 30–100% under automatic
/// cooling, with a 90 °C maximum operating temperature.
fn connected() -> Core {
    connected_with(snapshot(), reading(SimulatedFixture::TunableGpu), true)
}

/// Prism with tuning access to the tunable GPU as `change` makes its reading.
fn connected_reading(change: impl FnOnce(&mut prism_gpu::CoolingInfo)) -> Core {
    let mut sample = reading(SimulatedFixture::TunableGpu);
    change(&mut sample.gpus[0].cooling);
    connected_with(snapshot(), sample, true)
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

/// The Fans track's fan control mode.
fn fan_mode(core: &Core) -> FanModeControl {
    workspace(core)
        .fan_mode()
        .cloned()
        .expect("the Fans track has a fan control mode")
}

/// The mode the fan control mode shows picked, if any.
fn picked(core: &Core) -> Option<FanMode> {
    fan_mode(core)
        .modes
        .iter()
        .find(|choice| choice.selected)
        .map(|choice| choice.mode)
}

/// Why the mode can't be picked, when it can't.
fn unavailable(core: &Core, mode: FanMode) -> Option<String> {
    fan_mode(core)
        .modes
        .into_iter()
        .find(|choice| choice.mode == mode)
        .expect("every mode is offered")
        .unavailable
}

/// The fixed speed's stepper, while the Fans track shows one.
fn fixed_speed(core: &Core) -> Option<Stepper> {
    workspace(core).fixed_fan_speed().cloned()
}

fn changes(core: &Core) -> String {
    workspace(core).changes.summary
}

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

fn shared(setting: FanSetting) -> CoolingConfiguration {
    CoolingConfiguration::Shared { setting }
}

fn fixed(speed_percent: i64) -> CoolingConfiguration {
    shared(FanSetting::Fixed { speed_percent })
}

/// prismd's answer that cooling was applied as `status`, with `error`.
fn cooling_outcome(status: TuningOutcomeStatus, error: Option<&str>) -> DaemonResponse {
    let outcome = TuningOutcome {
        status,
        error: error.map(Into::into),
    };
    DaemonResponse::ProfileApplication(ProfileApplicationResponse {
        rollback: None,
        cooling: Some(CoolingApplication {
            revision: 1,
            outcome: outcome.clone(),
            fans: (0..2)
                .map(|fan_index| FanOutcome {
                    fan_index,
                    outcome: outcome.clone(),
                })
                .collect(),
        }),
        power: None,
        graphics: None,
        memory: None,
        corrections: Vec::new(),
    })
}

#[test]
fn the_fans_track_offers_each_fan_control_mode_with_automatic_cooling_in_effect() {
    let core = connected();

    let control = fan_mode(&core);

    assert_eq!(
        control
            .modes
            .iter()
            .map(|choice| (choice.mode, choice.name, choice.unavailable.clone()))
            .collect::<Vec<_>>(),
        [
            (FanMode::Automatic, "Automatic", None),
            (FanMode::Fixed, "Fixed speed", None),
            (FanMode::Curve, "Fan curve", None),
        ]
    );
    assert_eq!(picked(&core), Some(FanMode::Automatic));
    assert_eq!(control.pending, None);
    assert_eq!(control.read_only, None);
    // A fixed speed is set only in fixed-speed mode.
    assert_eq!(fixed_speed(&core), None);
}

#[test]
fn switching_to_fixed_speed_stages_cooling_with_a_stepper_within_the_fans_range() {
    let mut core = connected();

    let effects = intent(&mut core, Intent::StageFanMode(FanMode::Fixed));

    assert_eq!(edit_holds(&effects), [vec![GPU.to_owned()]]);
    assert_eq!(picked(&core), Some(FanMode::Fixed));
    let stepper = fixed_speed(&core).expect("fixed speed shows its stepper");
    assert_eq!((stepper.min, stepper.max), (30, 100));
    assert_eq!(fan_mode(&core).pending.as_deref(), Some("→ fixed"));
    assert_eq!(stepper.pending, Some(format!("→ {}%", stepper.value)));
    assert_eq!(changes(&core), "1 unapplied change");

    intent(&mut core, Intent::StageFanSpeed(55));
    assert_eq!(fixed_speed(&core).unwrap().value, 55);
    assert_eq!(
        fixed_speed(&core).unwrap().pending.as_deref(),
        Some("→ 55%")
    );
    // Kept within the range every fan supports.
    intent(&mut core, Intent::StageFanSpeed(5));
    assert_eq!(
        fixed_speed(&core).unwrap().pending.as_deref(),
        Some("→ 30%")
    );
    assert_eq!(changes(&core), "1 unapplied change");
}

#[test]
fn applying_a_fixed_speed_sends_one_apply_with_the_cooling_setting_and_the_reviewed_fans() {
    let mut core = connected();
    intent(&mut core, Intent::StageFanMode(FanMode::Fixed));
    intent(&mut core, Intent::StageFanSpeed(55));

    let sent = applies(&intent(&mut core, Intent::ApplyChanges));

    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(
        sent[0].1,
        ApplyGpuProfileRequest {
            gpu_id: GPU.into(),
            settings: ProfileSettings {
                cooling: Some(fixed(55)),
                ..ProfileSettings::default()
            },
            expected_revision: REVISION.into(),
            fan_indexes: Some(vec![0, 1]),
        }
    );
}

#[test]
fn an_applied_cooling_change_leaves_no_draft_and_one_that_failed_stays_pending_with_why() {
    let mut core = connected();
    intent(&mut core, Intent::StageFanMode(FanMode::Fixed));
    intent(&mut core, Intent::StageFanSpeed(55));
    let (command, _) = applies(&intent(&mut core, Intent::ApplyChanges))[0].clone();

    core.handle(Input::PrismdAnswered {
        id: command,
        response: cooling_outcome(
            TuningOutcomeStatus::Failed,
            Some("Insufficient permissions"),
        ),
    });

    assert_eq!(fan_mode(&core).pending.as_deref(), Some("→ fixed"));
    assert_eq!(
        fan_mode(&core).failed.as_deref(),
        Some("Not applied: Insufficient permissions")
    );
    assert_eq!(
        workspace(&core).changes.outcome.as_deref(),
        Some("Cooling not applied: Insufficient permissions")
    );

    let (command, _) = applies(&intent(&mut core, Intent::ApplyChanges))[0].clone();
    let effects = core.handle(Input::PrismdAnswered {
        id: command,
        response: cooling_outcome(TuningOutcomeStatus::Succeeded, None),
    });
    assert_eq!(edit_holds(&effects), [Vec::<String>::new()]);
    assert_eq!(changes(&core), "No unapplied changes");
}

#[test]
fn picking_the_mode_in_effect_again_leaves_cooling_unchanged() {
    let mut core = connected();
    intent(&mut core, Intent::StageFanMode(FanMode::Fixed));

    let effects = intent(&mut core, Intent::StageFanMode(FanMode::Automatic));

    assert_eq!(edit_holds(&effects), [Vec::<String>::new()]);
    assert_eq!(picked(&core), Some(FanMode::Automatic));
    assert_eq!(fan_mode(&core).pending, None);
    assert_eq!(changes(&core), "No unapplied changes");
}

#[test]
fn a_fixed_speed_in_effect_shows_in_its_stepper_and_staging_it_again_changes_nothing() {
    let mut snapshot = snapshot();
    snapshot.tuning_states[0].reported.cooling = Some(fixed(60));
    let mut core = connected_with(snapshot, reading(SimulatedFixture::TunableGpu), true);

    assert_eq!(picked(&core), Some(FanMode::Fixed));
    let stepper = fixed_speed(&core).unwrap();
    assert_eq!((stepper.value, stepper.readout.text()), (60, "60%".into()));

    intent(&mut core, Intent::StageFanMode(FanMode::Automatic));
    assert_eq!(fan_mode(&core).pending.as_deref(), Some("→ auto"));
    intent(&mut core, Intent::StageFanMode(FanMode::Fixed));
    assert_eq!(fixed_speed(&core).unwrap().value, 60);
    assert_eq!(changes(&core), "No unapplied changes");
}

#[test]
fn fan_curve_mode_stages_the_starter_fan_curve() {
    let mut core = connected();
    let FanCurveSupport::Available { starter, .. } =
        fan_curve::curve_support(&reading(SimulatedFixture::TunableGpu).gpus[0].cooling)
    else {
        panic!("the tunable GPU supports fan curves")
    };

    intent(&mut core, Intent::StageFanMode(FanMode::Curve));
    let sent = applies(&intent(&mut core, Intent::ApplyChanges));

    assert_eq!(
        sent[0].1.settings.cooling,
        Some(shared(FanSetting::Curve { points: starter }))
    );
}

#[test]
fn an_unknown_maximum_operating_temperature_makes_curves_unavailable_with_the_reason() {
    let mut core = connected_reading(|cooling| {
        cooling.max_operating_temperature_c = None;
        cooling.max_operating_temperature_reason =
            Some("This GPU or driver doesn't report a maximum operating temperature.".into());
    });

    let reason = unavailable(&core, FanMode::Curve).expect("curves are unavailable");
    assert!(reason.contains("maximum operating temperature"), "{reason}");
    assert_eq!(fan_mode(&core).note.as_deref(), Some(reason.as_str()));
    assert_eq!(unavailable(&core, FanMode::Automatic), None);
    assert_eq!(unavailable(&core, FanMode::Fixed), None);

    let effects = intent(&mut core, Intent::StageFanMode(FanMode::Curve));
    assert_eq!(effects, Vec::new());
    assert_eq!(changes(&core), "No unapplied changes");
}

#[test]
fn unknown_fan_speed_bounds_make_curves_unavailable_with_the_reason() {
    let core = connected_reading(|cooling| {
        for fan in &mut cooling.fans {
            fan.min_speed_percent = None;
            fan.max_speed_percent = None;
        }
    });

    let reason = unavailable(&core, FanMode::Curve).expect("curves are unavailable");
    assert!(reason.to_lowercase().contains("speed"), "{reason}");
    assert_eq!(unavailable(&core, FanMode::Automatic), None);
}

#[test]
fn fan_control_mode_is_read_only_with_the_reason_without_tuning_access_or_fan_control() {
    let mut core = connected_with(snapshot(), reading(SimulatedFixture::TunableGpu), false);
    assert_eq!(
        fan_mode(&core).read_only.as_deref(),
        Some("bob isn't in group wheel")
    );
    assert_eq!(
        intent(&mut core, Intent::StageFanMode(FanMode::Fixed)),
        Vec::new()
    );

    let core = connected_with(snapshot(), reading(SimulatedFixture::NoFanControl), true);
    assert!(fan_mode(&core).read_only.is_some());
}

#[test]
fn prismd_pushes_the_gpus_cooling_control_and_a_suspended_curve_says_why() {
    let mut core = connected();
    let reason = "The GPU's temperature is unavailable, so Prism stopped following its curve.";
    let curve = shared(FanSetting::Curve {
        points: vec![
            prism_gpu::fan_curve::FanCurvePoint::new(45, 30),
            prism_gpu::fan_curve::FanCurvePoint::new(85, 100),
        ],
    });

    core.handle(Input::PrismdEvent {
        event: DaemonEvent::CoolingControlChanged(GpuCoolingControl {
            gpu_id: GPU.into(),
            revision: 2,
            applied: Some(curve.clone()),
            suspension: Some(CurveSuspension {
                kind: SuspensionKind::Fault,
                reason: reason.into(),
                restoration: Vec::new(),
            }),
            recovery: None,
            remembered_curve: Some(curve),
        }),
        received_at: Duration::from_secs(2),
    });

    assert_eq!(fan_mode(&core).note.as_deref(), Some(reason));
}
