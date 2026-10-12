//! Switching GPU tuning profiles from the workspace header: the selected GPU's profiles from
//! prismd's events, applying one at once, asking before discarding a draft, the single range
//! correction review, and the palette's Apply profile commands.

use std::time::Duration;

use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_gpu::{
    GpuCommandError, GpuCommandErrorCode, GpuTelemetryProvider, TuningOutcome, TuningOutcomeStatus,
};
use prism_protocol::{
    CorrectAndApplyProfileRequest, CorrectedProfileApplication, DaemonEvent, DaemonRequest,
    DaemonResponse, DaemonSnapshot, NumericRangeCorrection, ProfileApplicationResponse,
    ProfileIdRequest, ProfileRangeCorrection, ProfileSettingKey, ProfilesState, ProtocolVersion,
    RangeBound, SessionMessage, SetEditHoldsRequest, Welcome,
};
use prism_ui::{
    CommandId, Content, Core, Dialog, Effect, GpuWorkspace, GuiSettings, Input, Intent,
    ProfileChoice, ReviewChange, SessionFailure,
};

const GPU: &str = "GPU-5a1b0000-0000-4000-8000-000000000001";

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

/// The caller's Quiet and Boost profiles of the tunable GPU, and Old card of an absent GPU, with
/// Quiet as the startup profile and Boost current.
fn profiles() -> ProfilesState {
    serde_json::from_str(include_str!(
        "../../prism-protocol/tests/contracts/profiles-state.json"
    ))
    .unwrap()
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

/// Prism connected to prismd with the tunable GPU, whose snapshot has the caller's profiles.
fn connected_as(can_tune: bool) -> Core {
    let mut core = Core::default();
    core.handle(Input::Launched {
        settings: GuiSettings::default(),
        started_unix_ms: 0,
    });
    let mut snapshot = snapshot();
    snapshot.profiles = profiles();
    core.handle(Input::PrismdConnected {
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
    core
}

fn connected() -> Core {
    connected_as(true)
}

fn intent(core: &mut Core, intent: Intent) -> Vec<Effect> {
    core.handle(Input::Intent(intent))
}

fn push(core: &mut Core, profiles: ProfilesState) {
    core.handle(Input::PrismdEvent {
        event: DaemonEvent::ProfilesChanged(profiles),
        received_at: Duration::from_secs(2),
    });
}

fn workspace(core: &Core) -> GpuWorkspace {
    match core.content() {
        Content::Gpu(workspace) => *workspace,
        other => panic!("expected the GPU workspace, got {other:?}"),
    }
}

fn choices(core: &Core) -> Vec<ProfileChoice> {
    workspace(core).profiles.choices
}

fn choice(core: &Core, name: &str) -> ProfileChoice {
    choices(core)
        .into_iter()
        .find(|choice| choice.name == name)
        .unwrap_or_else(|| panic!("no profile named {name}"))
}

/// The profile applications sent to prismd among `effects`, each with the command it answers.
fn profile_applications(effects: &[Effect]) -> Vec<(CommandId, String)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::CommandPrismd {
                id,
                request: DaemonRequest::ApplyProfileById(ProfileIdRequest { profile_id }),
            } => Some((*id, profile_id.clone())),
            _ => None,
        })
        .collect()
}

fn corrected_applications(effects: &[Effect]) -> Vec<(CommandId, CorrectAndApplyProfileRequest)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::CommandPrismd {
                id,
                request: DaemonRequest::CorrectAndApplyProfile(request),
            } => Some((*id, request.clone())),
            _ => None,
        })
        .collect()
}

/// Every request sent to prismd among `effects`, in order.
fn requests(effects: &[Effect]) -> Vec<DaemonRequest> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::CommandPrismd { request, .. } | Effect::SendToPrismd(request) => {
                Some(request.clone())
            }
            _ => None,
        })
        .collect()
}

fn outcome(status: TuningOutcomeStatus, error: Option<&str>) -> Option<TuningOutcome> {
    Some(TuningOutcome {
        status,
        error: error.map(str::to_owned),
    })
}

fn application(power: Option<TuningOutcome>) -> ProfileApplicationResponse {
    ProfileApplicationResponse {
        rollback: None,
        cooling: None,
        power,
        graphics: outcome(TuningOutcomeStatus::Succeeded, None),
        memory: outcome(TuningOutcomeStatus::Succeeded, None),
        corrections: Vec::new(),
    }
}

fn power_correction(saved_value: i64, proposed_value: i64) -> ProfileRangeCorrection {
    ProfileRangeCorrection::Numeric(NumericRangeCorrection {
        setting: ProfileSettingKey::PowerLimitWatts,
        saved_value,
        proposed_value,
        bound: RangeBound::Maximum,
    })
}

fn graphics_correction(saved_value: i64, proposed_value: i64) -> ProfileRangeCorrection {
    ProfileRangeCorrection::Numeric(NumericRangeCorrection {
        setting: ProfileSettingKey::GraphicsOffsetMhz,
        saved_value,
        proposed_value,
        bound: RangeBound::Minimum,
    })
}

/// prismd's answer that nothing was applied because these values are out of range.
fn needs_corrections(corrections: Vec<ProfileRangeCorrection>) -> DaemonResponse {
    let unattempted = outcome(TuningOutcomeStatus::Unattempted, Some("out of range"));
    DaemonResponse::ProfileApplication(ProfileApplicationResponse {
        rollback: None,
        cooling: None,
        power: unattempted.clone(),
        graphics: unattempted.clone(),
        memory: unattempted,
        corrections,
    })
}

/// Chooses `profile_id` without a draft, returning the one application sent.
fn choose(core: &mut Core, profile_id: &str) -> CommandId {
    let sent = profile_applications(&intent(core, Intent::ApplyProfile(profile_id.into())));
    assert_eq!(sent.len(), 1, "{sent:?}");
    sent[0].0
}

fn strip_outcome(core: &Core) -> Option<String> {
    workspace(core).changes.outcome
}

#[test]
fn the_header_shows_the_selected_gpus_profiles_with_the_current_one_selected_and_the_startup_one_marked()
 {
    let core = connected();

    let choices = choices(&core);
    let names: Vec<_> = choices.iter().map(|choice| choice.name.as_str()).collect();
    // Old card belongs to a GPU that isn't here.
    assert_eq!(names, ["Quiet", "Boost"]);
    assert!(!choice(&core, "Quiet").current);
    assert!(choice(&core, "Quiet").startup);
    assert!(choice(&core, "Boost").current);
    assert!(!choice(&core, "Boost").startup);
    assert!(choices.iter().all(|choice| choice.unavailable.is_none()));
    assert_eq!(workspace(&core).profiles.current.as_deref(), Some("Boost"));
    assert_eq!(workspace(&core).profiles.read_only, None);
}

#[test]
fn the_header_follows_pushed_profiles() {
    let mut core = connected();

    let mut changed = profiles();
    changed.current_profiles.insert(
        GPU.into(),
        prism_protocol::CurrentProfile {
            id: "quiet".into(),
            name: "Silent".into(),
        },
    );
    changed.startup_profiles.clear();
    changed.profiles[0].name = "Silent".into();
    push(&mut core, changed);

    assert!(choice(&core, "Silent").current);
    assert!(!choice(&core, "Silent").startup);
    assert!(!choice(&core, "Boost").current);
    assert_eq!(workspace(&core).profiles.current.as_deref(), Some("Silent"));

    push(&mut core, ProfilesState::default());
    assert_eq!(choices(&core), Vec::new());
    assert_eq!(workspace(&core).profiles.current, None);
}

#[test]
fn a_session_lost_forgets_the_profiles() {
    let mut core = connected();

    core.handle(Input::PrismdNotConnected(SessionFailure::Unavailable {
        message: "The session with prismd ended".into(),
    }));

    assert_eq!(choices(&core), Vec::new());
}

#[test]
fn choosing_a_profile_applies_it_at_once() {
    let mut core = connected();

    let effects = intent(&mut core, Intent::ApplyProfile("quiet".into()));

    assert_eq!(
        requests(&effects),
        [DaemonRequest::ApplyProfileById(ProfileIdRequest {
            profile_id: "quiet".into()
        })]
    );
    assert_eq!(core.dialog(), None);
    // The current profile changes only once prismd pushes it.
    assert!(choice(&core, "Boost").current);
    // Another choice waits for its answer.
    assert_eq!(
        choice(&core, "Boost").unavailable.as_deref(),
        Some("Applying…")
    );
    assert_eq!(
        requests(&intent(&mut core, Intent::ApplyProfile("boost".into()))),
        []
    );
}

#[test]
fn an_applied_profile_says_so_and_a_partial_or_failed_one_says_what_it_left() {
    let mut core = connected();

    let command = choose(&mut core, "quiet");
    core.handle(Input::PrismdAnswered {
        id: command,
        response: DaemonResponse::ProfileApplication(application(outcome(
            TuningOutcomeStatus::Succeeded,
            None,
        ))),
    });
    assert_eq!(strip_outcome(&core).as_deref(), Some("Quiet applied"));
    assert_eq!(choice(&core, "Boost").unavailable, None);

    let command = choose(&mut core, "boost");
    core.handle(Input::PrismdAnswered {
        id: command,
        response: DaemonResponse::ProfileApplication(application(outcome(
            TuningOutcomeStatus::Failed,
            Some("Insufficient permissions"),
        ))),
    });
    assert_eq!(
        strip_outcome(&core).as_deref(),
        Some("Boost applied in part · Power limit not applied: Insufficient permissions")
    );

    let command = choose(&mut core, "quiet");
    core.handle(Input::PrismdAnswered {
        id: command,
        response: DaemonResponse::Error(GpuCommandError::new(
            GpuCommandErrorCode::NotFound,
            "The profile no longer exists.",
        )),
    });
    assert_eq!(
        strip_outcome(&core).as_deref(),
        Some("Quiet not applied: The profile no longer exists.")
    );

    // A new edit replaces what the profile application said.
    intent(
        &mut core,
        Intent::Stage(ProfileSettingKey::PowerLimitWatts, 450),
    );
    assert_eq!(strip_outcome(&core), None);
}

#[test]
fn choosing_a_profile_over_unapplied_changes_asks_before_discarding_them() {
    let mut core = connected();
    intent(
        &mut core,
        Intent::Stage(ProfileSettingKey::PowerLimitWatts, 450),
    );
    intent(
        &mut core,
        Intent::Stage(ProfileSettingKey::GraphicsOffsetMhz, 100),
    );

    let effects = intent(&mut core, Intent::ApplyProfile("quiet".into()));

    assert_eq!(requests(&effects), []);
    let Some(Dialog::DiscardForProfile(question)) = core.dialog() else {
        panic!("expected the discard question, got {:?}", core.dialog())
    };
    assert_eq!(
        question.title,
        "Discard 2 unapplied changes and apply Quiet?"
    );

    // Keep editing changes nothing.
    assert_eq!(requests(&intent(&mut core, Intent::Dismiss)), []);
    assert_eq!(core.dialog(), None);
    assert_eq!(workspace(&core).changes.summary, "2 unapplied changes");

    intent(&mut core, Intent::ApplyProfile("quiet".into()));
    let effects = intent(&mut core, Intent::ConfirmProfile);
    assert_eq!(
        requests(&effects),
        [
            DaemonRequest::ApplyProfileById(ProfileIdRequest {
                profile_id: "quiet".into()
            }),
            DaemonRequest::SetEditHolds(SetEditHoldsRequest { gpu_ids: vec![] }),
        ]
    );
    assert_eq!(core.dialog(), None);
    assert_eq!(workspace(&core).changes.summary, "No unapplied changes");
}

#[test]
fn out_of_range_values_open_one_review_whose_confirmation_corrects_and_applies() {
    let mut core = connected();
    let command = choose(&mut core, "quiet");

    let corrections = vec![
        power_correction(650, 600),
        graphics_correction(-1200, -1000),
    ];
    let effects = core.handle(Input::PrismdAnswered {
        id: command,
        response: needs_corrections(corrections.clone()),
    });
    assert_eq!(requests(&effects), []);

    let Some(Dialog::ProfileCorrections(review)) = core.dialog() else {
        panic!("expected the correction review, got {:?}", core.dialog())
    };
    assert_eq!(review.title, "Adjust Quiet to apply it?");
    assert_eq!(
        review.corrections,
        [
            ReviewChange {
                name: "Power limit".into(),
                change: "650 W → 600 W".into(),
                yours: None,
            },
            ReviewChange {
                name: "Graphics clock offset".into(),
                change: "-1200 MHz → -1000 MHz".into(),
                yours: None,
            },
        ]
    );
    // Nothing was applied, so nothing is said about it yet.
    assert_eq!(strip_outcome(&core), None);

    let effects = intent(&mut core, Intent::ConfirmProfile);
    let sent = corrected_applications(&effects);
    assert_eq!(
        sent.iter().map(|(_, request)| request).collect::<Vec<_>>(),
        [&CorrectAndApplyProfileRequest {
            profile_id: "quiet".into(),
            corrections,
        }]
    );
    assert_eq!(core.dialog(), None);

    let mut corrected = profiles().profiles[0].clone();
    corrected.settings.power_limit_watts =
        Some(prism_protocol::ProfileSetting::Value { value: 600 });
    core.handle(Input::PrismdAnswered {
        id: sent[0].0,
        response: DaemonResponse::CorrectedProfileApplication(CorrectedProfileApplication {
            profile: corrected,
            application: application(outcome(TuningOutcomeStatus::Succeeded, None)),
        }),
    });
    assert_eq!(strip_outcome(&core).as_deref(), Some("Quiet applied"));
}

#[test]
fn canceling_the_correction_review_sends_nothing() {
    let mut core = connected();
    let command = choose(&mut core, "quiet");
    core.handle(Input::PrismdAnswered {
        id: command,
        response: needs_corrections(vec![power_correction(650, 600)]),
    });

    let effects = intent(&mut core, Intent::Dismiss);

    assert_eq!(requests(&effects), []);
    assert_eq!(core.dialog(), None);
    assert_eq!(choice(&core, "Quiet").unavailable, None);
    assert_eq!(strip_outcome(&core), None);
}

#[test]
fn a_profile_of_an_absent_gpu_stays_listed_by_prismd_but_cant_be_applied() {
    let mut core = connected();

    assert_eq!(
        requests(&intent(&mut core, Intent::ApplyProfile("old-card".into()))),
        []
    );
    assert_eq!(core.dialog(), None);
}

#[test]
fn without_tuning_access_the_profiles_are_read_only() {
    let mut core = connected_as(false);

    let profiles = workspace(&core).profiles;
    assert_eq!(
        profiles.read_only.as_deref(),
        Some("bob isn't in group wheel")
    );
    assert_eq!(profiles.current.as_deref(), Some("Boost"));
    assert_eq!(
        requests(&intent(&mut core, Intent::ApplyProfile("quiet".into()))),
        []
    );
}

#[test]
fn the_palette_applies_each_profile_of_the_selected_gpu_and_still_asks_before_discarding() {
    let mut core = connected();
    intent(&mut core, Intent::OpenPalette);
    intent(&mut core, Intent::FilterPalette("apply profile".into()));
    let titles: Vec<_> = core
        .palette()
        .unwrap()
        .entries
        .into_iter()
        .map(|entry| entry.title)
        .collect();
    assert_eq!(titles, ["Apply profile Quiet", "Apply profile Boost"]);

    let effects = intent(&mut core, Intent::RunHighlighted);
    assert_eq!(profile_applications(&effects).len(), 1);
    assert!(!core.palette_open());

    let mut core = connected();
    intent(
        &mut core,
        Intent::Stage(ProfileSettingKey::PowerLimitWatts, 450),
    );
    intent(&mut core, Intent::OpenPalette);
    intent(
        &mut core,
        Intent::FilterPalette("apply profile boost".into()),
    );
    let effects = intent(&mut core, Intent::RunHighlighted);
    assert_eq!(requests(&effects), []);
    assert!(matches!(
        core.dialog(),
        Some(Dialog::DiscardForProfile(question))
            if question.title == "Discard 1 unapplied change and apply Boost?"
    ));
}

#[test]
fn the_palette_says_why_a_profile_cant_be_applied() {
    let mut core = connected_as(false);
    intent(&mut core, Intent::OpenPalette);
    intent(
        &mut core,
        Intent::FilterPalette("apply profile quiet".into()),
    );

    let entry = core.palette().unwrap().entries.remove(0);
    assert_eq!(entry.title, "Apply profile Quiet");
    assert_eq!(
        entry.unavailable.as_deref(),
        Some("Needs tuning access · bob isn't in group wheel")
    );
}

#[test]
fn a_profile_dialog_closes_once_its_profile_is_gone() {
    let mut core = connected();
    let command = choose(&mut core, "quiet");
    core.handle(Input::PrismdAnswered {
        id: command,
        response: needs_corrections(vec![power_correction(650, 600)]),
    });
    assert!(core.dialog_open());

    let mut deleted = profiles();
    deleted.profiles.retain(|profile| profile.id != "quiet");
    push(&mut core, deleted);
    assert!(!core.dialog_open());
    assert_eq!(requests(&intent(&mut core, Intent::ConfirmProfile)), []);

    let mut core = connected();
    intent(
        &mut core,
        Intent::Stage(ProfileSettingKey::PowerLimitWatts, 450),
    );
    intent(&mut core, Intent::ApplyProfile("quiet".into()));
    assert!(core.dialog_open());
    core.handle(Input::PrismdNotConnected(SessionFailure::Unavailable {
        message: "The session with prismd ended".into(),
    }));
    assert!(!core.dialog_open());
}

#[test]
fn corrections_that_arrive_under_another_dialog_say_so_in_the_strip() {
    let mut core = connected();
    let command = choose(&mut core, "quiet");
    intent(
        &mut core,
        Intent::Stage(ProfileSettingKey::PowerLimitWatts, 450),
    );
    intent(&mut core, Intent::CloseWindow);
    assert!(matches!(core.dialog(), Some(Dialog::ConfirmClose(_))));

    core.handle(Input::PrismdAnswered {
        id: command,
        response: needs_corrections(vec![power_correction(650, 600)]),
    });

    assert!(matches!(core.dialog(), Some(Dialog::ConfirmClose(_))));
    intent(&mut core, Intent::Dismiss);
    intent(&mut core, Intent::DiscardChanges);
    assert_eq!(
        strip_outcome(&core).as_deref(),
        Some("Quiet needs values adjusted to what the GPU allows. Choose it again to review them.")
    );
}
