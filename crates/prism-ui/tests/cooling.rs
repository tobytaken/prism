//! Cooling in the staged loop: the Fans track's fan control mode and fixed speed, staged in the
//! GPU's draft and applied with the reviewed fans, through the core's input/effect interface.

use std::time::Duration;

use prism_gpu::cooling::{
    CoolingApplication, CoolingConfiguration, CurveSuspension, FanOutcome, FanSetting,
    SuspensionKind,
};
use prism_gpu::fan_curve::{self, FanCurvePoint, FanCurveSupport};
use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_gpu::{GpuPerformanceResponse, GpuTelemetryProvider, TuningOutcome, TuningOutcomeStatus};
use prism_protocol::{
    ApplyGpuProfileRequest, DaemonEvent, DaemonRequest, DaemonResponse, DaemonSnapshot,
    GpuCoolingControl, ProfileApplicationResponse, ProfileSettings, ProtocolVersion,
    SessionMessage, SetEditHoldsRequest, Welcome,
};
use prism_ui::{
    AppearanceMode, CommandId, Content, Core, Effect, FanCurveEditor, FanMode, FanModeControl,
    GpuWorkspace, Input, Intent, Stepper,
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
        points: vec![FanCurvePoint::new(45, 30), FanCurvePoint::new(85, 100)],
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

#[test]
fn fans_whose_speed_ranges_dont_overlap_cant_share_a_fixed_speed_and_say_so() {
    let mut core = connected_reading(|cooling| {
        cooling.fans[0].max_speed_percent = Some(40);
        cooling.fans[1].min_speed_percent = Some(50);
    });

    let reason = unavailable(&core, FanMode::Fixed).expect("fixed speed is unavailable");
    assert!(reason.contains("don't overlap"), "{reason}");
    assert_eq!(
        intent(&mut core, Intent::StageFanMode(FanMode::Fixed)),
        Vec::new()
    );
}

#[test]
fn no_mode_is_staged_before_the_gpus_fans_have_been_read() {
    let mut core = Core::default();
    core.handle(Input::Launched {
        appearance: AppearanceMode::System,
    });
    core.handle(Input::PrismdConnected {
        welcome: welcome(true),
        snapshot: snapshot(),
        socket: "/run/prism/prismd.sock".into(),
    });

    for mode in FanMode::ALL {
        assert_eq!(intent(&mut core, Intent::StageFanMode(mode)), Vec::new());
    }
    assert_eq!(changes(&core), "No unapplied changes");
}

/// The Fans track's fan curve editor, while it shows a curve.
fn fan_curve(core: &Core) -> Option<FanCurveEditor> {
    workspace(core).fan_curve().cloned()
}

/// The editor's points as (temperature, speed).
fn curve_points(core: &Core) -> Vec<(i64, i64)> {
    fan_curve(core)
        .expect("the editor shows a curve")
        .points
        .iter()
        .map(|point| (point.temperature_c, point.speed_percent))
        .collect()
}

fn curve(points: &[(i64, i64)]) -> CoolingConfiguration {
    shared(FanSetting::Curve {
        points: points
            .iter()
            .map(|&(temperature_c, speed_percent)| FanCurvePoint::new(temperature_c, speed_percent))
            .collect(),
    })
}

/// Prism with tuning access to the tunable GPU while `points` is the fan curve in effect.
fn connected_with_curve(points: &[(i64, i64)]) -> Core {
    let mut snapshot = snapshot();
    snapshot.tuning_states[0].reported.cooling = Some(curve(points));
    connected_with(snapshot, reading(SimulatedFixture::TunableGpu), true)
}

/// The tunable GPU's starter fan curve: T is 85 °C, 5 °C below its 90 °C maximum operating
/// temperature, and its fans run at 30–100%.
const STARTER: [(i64, i64); 4] = [(45, 30), (65, 65), (75, 83), (85, 100)];

fn move_point(core: &mut Core, point: usize, temperature_by: i64, speed_by: i64) -> Vec<Effect> {
    intent(
        core,
        Intent::MoveCurvePoint {
            point,
            temperature_by,
            speed_by,
        },
    )
}

#[test]
fn without_a_curve_the_editor_shows_nothing_until_fan_curve_mode_proposes_the_starter_curve() {
    let mut core = connected();
    assert_eq!(fan_curve(&core), None);

    intent(&mut core, Intent::StageFanMode(FanMode::Curve));

    let editor = fan_curve(&core).expect("the starter curve shows in the editor");
    assert_eq!(curve_points(&core), STARTER);
    assert!(editor.open);
    assert!(editor.pending);
    assert_eq!(editor.read_only, None);
    assert_eq!(editor.problems, Vec::<String>::new());
    // Nothing is in effect to draw behind it.
    assert_eq!(editor.in_effect_line, None);
    assert!(editor.pending_line.is_some());
    assert_eq!(
        editor.points[1].value_text, "65% at 65 °C",
        "a point says its speed and temperature"
    );
}

#[test]
fn a_curve_in_effect_shows_as_one_curve_and_an_edit_stages_the_complete_curve_beside_it() {
    let mut core = connected_with_curve(&STARTER);
    assert_eq!(picked(&core), Some(FanMode::Curve));
    let editor = fan_curve(&core).expect("the curve in effect shows");
    assert!(!editor.pending);
    assert!(editor.in_effect_line.is_some());
    assert_eq!(editor.pending_line, None);
    assert_eq!(changes(&core), "No unapplied changes");

    let effects = move_point(&mut core, 1, 2, -5);

    assert_eq!(edit_holds(&effects), [vec![GPU.to_owned()]]);
    assert_eq!(
        curve_points(&core),
        [(45, 30), (67, 60), (75, 83), (85, 100)]
    );
    let editor = fan_curve(&core).unwrap();
    assert!(editor.pending);
    // The pending curve is drawn as a second curve over the one in effect.
    assert!(editor.in_effect_line.is_some());
    assert!(editor.pending_line.is_some());
    assert_eq!(fan_mode(&core).pending.as_deref(), Some("→ curve"));
    assert_eq!(picked(&core), Some(FanMode::Curve));
    assert_eq!(changes(&core), "1 unapplied change");

    // Moving it back leaves cooling unchanged.
    move_point(&mut core, 1, -2, 5);
    assert_eq!(changes(&core), "No unapplied changes");
    assert!(!fan_curve(&core).unwrap().pending);
}

#[test]
fn dragging_a_point_stages_the_temperature_and_speed_drawn_where_it_is_let_go() {
    let mut core = connected_with_curve(&STARTER);
    let editor = fan_curve(&core).unwrap();
    let (from, to) = editor.temperature_axis;
    let x = |celsius: i64| (celsius - from) as f32 / (to - from) as f32;
    let y = |percent: i64| (100 - percent) as f32 / 100.0;

    intent(
        &mut core,
        Intent::DragCurvePoint {
            point: 2,
            x: x(70),
            y: y(75),
        },
    );
    assert_eq!(
        curve_points(&core),
        [(45, 30), (65, 65), (70, 75), (85, 100)]
    );
    let point = &fan_curve(&core).unwrap().points[2];
    assert!((point.at.x - x(70)).abs() < 1e-4 && (point.at.y - y(75)).abs() < 1e-4);

    // A point stays between its neighbors' temperatures, and within the speeds every fan
    // supports.
    intent(
        &mut core,
        Intent::DragCurvePoint {
            point: 2,
            x: 0.0,
            y: 1.0,
        },
    );
    assert_eq!(
        curve_points(&core),
        [(45, 30), (65, 65), (66, 30), (85, 100)]
    );
    intent(
        &mut core,
        Intent::DragCurvePoint {
            point: 3,
            x: 1.0,
            y: -1.0,
        },
    );
    // The last point stops at the maximum operating temperature.
    assert_eq!(curve_points(&core)[3], (90, 100));
}

#[test]
fn a_zero_percent_request_is_offered_only_when_every_fan_supports_it() {
    let mut core = connected_reading(|cooling| {
        for fan in &mut cooling.fans {
            fan.min_speed_percent = Some(0);
        }
    });
    intent(&mut core, Intent::StageFanMode(FanMode::Curve));
    assert_eq!(curve_points(&core)[0], (45, 0));

    let mut core = connected_reading(|cooling| cooling.fans[1].min_speed_percent = Some(0));
    intent(&mut core, Intent::StageFanMode(FanMode::Curve));
    move_point(&mut core, 0, 0, -50);
    assert_eq!(curve_points(&core)[0], (45, 30));
}

#[test]
fn a_curve_that_reaches_full_speed_too_late_cant_be_applied_and_says_why() {
    let mut core = connected();
    intent(&mut core, Intent::StageFanMode(FanMode::Curve));

    // The last point moves past T, 85 °C.
    move_point(&mut core, 3, 2, 0);

    let editor = fan_curve(&core).unwrap();
    assert_eq!(
        editor.problems,
        [
            "The curve must reach 100% by 85°C, 5°C below the GPU's 90°C maximum operating \
          temperature."
        ]
    );
    let why = workspace(&core)
        .changes
        .apply_unavailable
        .expect("Apply is unavailable");
    assert!(why.contains("must reach 100% by 85°C"), "{why}");
    assert_eq!(
        applies(&intent(&mut core, Intent::ApplyChanges)),
        Vec::new()
    );

    // Lowering the full speed instead breaks the same rule.
    move_point(&mut core, 3, -2, -1);
    assert_eq!(fan_curve(&core).unwrap().problems.len(), 1);
    // Back at full speed by T, it can be applied.
    move_point(&mut core, 3, 0, 1);
    assert_eq!(fan_curve(&core).unwrap().problems, Vec::<String>::new());
    assert_eq!(workspace(&core).changes.apply_unavailable, None);
}

#[test]
fn a_speed_lower_than_the_point_before_marks_the_point_and_blocks_apply() {
    let mut core = connected();
    intent(&mut core, Intent::StageFanMode(FanMode::Curve));

    move_point(&mut core, 1, 0, -40);

    let editor = fan_curve(&core).unwrap();
    assert_eq!(curve_points(&core)[1], (65, 30));
    move_point(&mut core, 0, 0, 10);
    let editor_after = fan_curve(&core).unwrap();
    assert!(editor.problems.is_empty(), "{:?}", editor.problems);
    assert_eq!(
        editor_after.problems,
        ["Point 2: speed can't be lower than point 1's 40%."]
    );
    assert_eq!(
        editor_after
            .points
            .iter()
            .map(|point| point.faulty)
            .collect::<Vec<_>>(),
        [false, true, false, false]
    );
    assert!(workspace(&core).changes.apply_unavailable.is_some());
}

#[test]
fn applying_an_edited_curve_sends_the_complete_curve_in_one_apply() {
    let mut core = connected_with_curve(&STARTER);
    move_point(&mut core, 0, -5, 0);
    move_point(&mut core, 2, 0, 2);

    let sent = applies(&intent(&mut core, Intent::ApplyChanges));

    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(
        sent[0].1,
        ApplyGpuProfileRequest {
            gpu_id: GPU.into(),
            settings: ProfileSettings {
                cooling: Some(curve(&[(40, 30), (65, 65), (75, 85), (85, 100)])),
                ..ProfileSettings::default()
            },
            expected_revision: REVISION.into(),
            fan_indexes: Some(vec![0, 1]),
        }
    );
}

#[test]
fn the_live_marker_follows_the_temperature_and_fan_speed_prismd_reports() {
    let mut core = connected_with_curve(&STARTER);
    let marker = |core: &Core| fan_curve(core).unwrap().marker.expect("a live marker");
    let (from, to) = fan_curve(&core).unwrap().temperature_axis;

    let mut sample = reading(SimulatedFixture::TunableGpu);
    sample.gpus[0].telemetry.temperature_c = Some(60);
    sample.gpus[0].telemetry.fan_speed_percent = Some(55);
    core.handle(Input::PrismdEvent {
        event: DaemonEvent::TelemetrySampled(sample.clone()),
        received_at: Duration::from_secs(2),
    });
    let first = marker(&core);
    assert_eq!(first.text, "Now 60 °C, fans at 55%");
    assert!((first.at.x - (60 - from) as f32 / (to - from) as f32).abs() < 1e-4);
    assert!((first.at.y - 0.45).abs() < 1e-4);

    sample.gpus[0].telemetry.temperature_c = Some(70);
    sample.gpus[0].telemetry.fan_speed_percent = Some(75);
    core.handle(Input::PrismdEvent {
        event: DaemonEvent::TelemetrySampled(sample.clone()),
        received_at: Duration::from_secs(3),
    });
    let second = marker(&core);
    assert_eq!(second.text, "Now 70 °C, fans at 75%");
    assert!(second.at.x > first.at.x && second.at.y < first.at.y);

    // Without a temperature, there's nowhere on the curve to put it.
    sample.gpus[0].telemetry.temperature_c = None;
    core.handle(Input::PrismdEvent {
        event: DaemonEvent::TelemetrySampled(sample),
        received_at: Duration::from_secs(4),
    });
    assert_eq!(fan_curve(&core).unwrap().marker, None);
}

#[test]
fn without_tuning_access_the_curve_in_effect_shows_read_only_with_the_reason() {
    let mut snapshot = snapshot();
    snapshot.tuning_states[0].reported.cooling = Some(curve(&STARTER));
    let mut core = connected_with(snapshot, reading(SimulatedFixture::TunableGpu), false);

    let editor = fan_curve(&core).expect("the curve in effect shows");
    assert_eq!(
        editor.read_only.as_deref(),
        Some("bob isn't in group wheel")
    );
    assert_eq!(move_point(&mut core, 1, 1, 1), Vec::new());
    assert_eq!(
        intent(
            &mut core,
            Intent::DragCurvePoint {
                point: 1,
                x: 0.5,
                y: 0.5
            }
        ),
        Vec::new()
    );
    assert_eq!(curve_points(&core), STARTER);
}

#[test]
fn the_editor_collapses_and_opens_again() {
    let mut core = connected_with_curve(&STARTER);
    assert!(fan_curve(&core).unwrap().open);

    intent(&mut core, Intent::ToggleFanCurve);
    assert!(!fan_curve(&core).unwrap().open);

    intent(&mut core, Intent::ToggleFanCurve);
    assert!(fan_curve(&core).unwrap().open);
}

#[test]
fn the_editor_marks_where_the_curve_must_reach_full_speed() {
    let core = connected_with_curve(&STARTER);
    let editor = fan_curve(&core).unwrap();
    let (from, to) = editor.temperature_axis;

    let full_speed = editor.full_speed_x.expect("T is known");
    assert!((full_speed - (85 - from) as f32 / (to - from) as f32).abs() < 1e-4);
    // The axis covers the maximum operating temperature.
    assert!(from <= 45 && to >= 90, "{from}–{to}");
}

#[test]
fn a_pending_curve_cant_be_applied_while_the_gpus_fans_go_unread() {
    let mut core = connected();
    intent(&mut core, Intent::StageFanMode(FanMode::Curve));
    let mut sample = reading(SimulatedFixture::TunableGpu);
    sample.gpus.clear();
    core.handle(Input::PrismdEvent {
        event: DaemonEvent::TelemetrySampled(sample),
        received_at: Duration::from_secs(2),
    });

    let why = workspace(&core)
        .changes
        .apply_unavailable
        .expect("Apply is unavailable");
    assert!(why.contains("reading of the GPU's fans"), "{why}");
    assert_eq!(
        applies(&intent(&mut core, Intent::ApplyChanges)),
        Vec::new()
    );
}
