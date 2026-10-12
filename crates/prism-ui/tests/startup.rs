//! Settings → Startup: the host-wide startup behavior and each GPU's startup profile, rendered
//! from prismd's snapshot and `StartupStateChanged`, changed through `SetStartupBehavior` and
//! `SetStartupProfile`, read-only without tuning access, with refusals, possibly stale last
//! applied tuning values and a suspended startup explained.

use std::time::Duration;

use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_gpu::{GpuCommandError, GpuCommandErrorCode, GpuTelemetryProvider};
use prism_protocol::{
    CurrentProfile, DaemonCommandResponse, DaemonEvent, DaemonRequest, DaemonResponse,
    DaemonSnapshot, ProfilesState, ProtocolVersion, SessionMessage, SetStartupBehaviorRequest,
    SetStartupProfileRequest, StartupBehavior, StartupProfile, StartupState, TuningStoreHealth,
    TuningStoreProblem, Welcome,
};
use prism_ui::{
    CommandId, Core, Effect, GuiSettings, Input, Intent, Section, SessionFailure, SettingsSection,
    StartupBehaviorChoice, StartupChoice, StartupSection,
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

/// The caller's Quiet and Boost profiles of the tunable GPU, and Old card of an absent GPU.
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

fn launched() -> Core {
    let mut core = Core::default();
    core.handle(Input::Launched {
        settings: GuiSettings::default(),
        started_unix_ms: 0,
    });
    core.handle(Input::PrismdEvent {
        event: DaemonEvent::TelemetrySampled(
            SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO).performance(),
        ),
        received_at: Duration::from_secs(1),
    });
    core
}

/// Prism connected to prismd with the tunable GPU and the caller's profiles, its startup
/// behavior off and no startup profiles, showing Settings → Startup.
fn connected_with(can_tune: bool, change: impl FnOnce(&mut DaemonSnapshot)) -> Core {
    let mut core = launched();
    let mut snapshot = snapshot();
    snapshot.profiles = profiles();
    change(&mut snapshot);
    core.handle(Input::PrismdConnected {
        welcome: welcome(can_tune),
        snapshot,
        socket: "/run/prism/prismd.sock".into(),
    });
    intent(&mut core, Intent::GoToSettings(SettingsSection::Startup));
    core
}

fn connected() -> Core {
    connected_with(true, |_| {})
}

fn intent(core: &mut Core, intent: Intent) -> Vec<Effect> {
    core.handle(Input::Intent(intent))
}

fn push(core: &mut Core, event: DaemonEvent) {
    core.handle(Input::PrismdEvent {
        event,
        received_at: Duration::from_secs(2),
    });
}

fn startup(behavior: StartupBehavior, profiles: &[(&str, &str)]) -> StartupState {
    StartupState {
        behavior,
        profiles: profiles
            .iter()
            .map(|(id, name)| StartupProfile {
                gpu_id: GPU.into(),
                profile: CurrentProfile {
                    id: (*id).into(),
                    name: (*name).into(),
                },
            })
            .collect(),
        outcomes: vec![],
        possibly_stale: vec![],
    }
}

/// The one command sent to prismd among `effects`, with the id its answer comes back with.
fn command(effects: &[Effect]) -> (CommandId, DaemonRequest) {
    match effects {
        [Effect::CommandPrismd { id, request }] => (*id, request.clone()),
        other => panic!("expected one command, got {other:?}"),
    }
}

fn answer(core: &mut Core, id: CommandId, response: DaemonResponse) {
    core.handle(Input::PrismdAnswered { id, response });
}

fn done() -> DaemonResponse {
    DaemonResponse::Command(DaemonCommandResponse {
        success: true,
        error: None,
    })
}

/// The behaviors' names, each with whether it's the one chosen.
fn behaviors(section: &StartupSection) -> Vec<(&str, bool)> {
    section
        .behaviors
        .iter()
        .map(|choice| (choice.name.as_str(), choice.chosen))
        .collect()
}

/// The tunable GPU's startup profile choices, as (name, chosen, why it can't be chosen).
fn choices(section: &StartupSection) -> Vec<(&str, bool, Option<&str>)> {
    section.gpus[0]
        .choices
        .iter()
        .map(|choice| {
            (
                choice.name.as_str(),
                choice.chosen,
                choice.unavailable.as_deref(),
            )
        })
        .collect()
}

#[test]
fn the_startup_choices_render_from_the_snapshot_and_then_from_events() {
    let mut core = connected();
    let section = core.startup_settings();

    assert_eq!(
        behaviors(&section),
        [
            ("Nothing", true),
            ("Startup profiles", false),
            ("Last applied tuning values", false),
        ]
    );
    assert_eq!(section.read_only, None);
    assert_eq!(section.gpus.len(), 1);
    assert_eq!(section.gpus[0].gpu_id, GPU);
    assert!(section.gpus[0].gpu.starts_with("GPU 0"), "{section:?}");
    assert_eq!(section.gpus[0].startup, "None");
    // None, then the caller's profiles of the GPU; Old card belongs to a GPU that isn't here.
    assert_eq!(
        choices(&section),
        [
            ("None", true, None),
            ("Quiet", false, None),
            ("Boost", false, None)
        ]
    );

    push(
        &mut core,
        DaemonEvent::StartupStateChanged(startup(
            StartupBehavior::StartupProfiles,
            &[("quiet", "Quiet")],
        )),
    );

    let section = core.startup_settings();
    assert_eq!(
        behaviors(&section),
        [
            ("Nothing", false),
            ("Startup profiles", true),
            ("Last applied tuning values", false),
        ]
    );
    assert_eq!(section.gpus[0].startup, "Quiet");
    assert_eq!(
        choices(&section),
        [
            ("None", false, None),
            ("Quiet", true, None),
            ("Boost", false, None)
        ]
    );
}

#[test]
fn choosing_a_startup_behavior_sends_it_and_shows_it_only_once_prismd_pushes_it() {
    let mut core = connected();

    let (id, request) = command(&intent(
        &mut core,
        Intent::SetStartupBehavior(StartupBehavior::RestoreLastApplied),
    ));

    assert_eq!(
        request,
        DaemonRequest::SetStartupBehavior(SetStartupBehaviorRequest {
            behavior: StartupBehavior::RestoreLastApplied,
        })
    );
    answer(&mut core, id, done());
    // Events are the only source of state.
    assert_eq!(behaviors(&core.startup_settings())[0], ("Nothing", true));

    push(
        &mut core,
        DaemonEvent::StartupStateChanged(startup(StartupBehavior::RestoreLastApplied, &[])),
    );
    assert_eq!(
        behaviors(&core.startup_settings())[2],
        ("Last applied tuning values", true)
    );
    assert_eq!(core.startup_settings().outcome, None);

    // Already chosen, so there's nothing to send.
    assert_eq!(
        intent(
            &mut core,
            Intent::SetStartupBehavior(StartupBehavior::RestoreLastApplied)
        ),
        []
    );
}

#[test]
fn choosing_a_gpus_startup_profile_sends_it_and_none_clears_it() {
    let mut core = connected();

    let (_, request) = command(&intent(
        &mut core,
        Intent::SetStartupProfile {
            gpu_id: GPU.into(),
            profile_id: Some("boost".into()),
        },
    ));
    assert_eq!(
        request,
        DaemonRequest::SetStartupProfile(SetStartupProfileRequest {
            gpu_id: GPU.into(),
            profile_id: Some("boost".into()),
        })
    );

    push(
        &mut core,
        DaemonEvent::StartupStateChanged(startup(StartupBehavior::Off, &[("boost", "Boost")])),
    );
    assert_eq!(core.startup_settings().gpus[0].startup, "Boost");

    let (_, request) = command(&intent(
        &mut core,
        Intent::SetStartupProfile {
            gpu_id: GPU.into(),
            profile_id: None,
        },
    ));
    assert_eq!(
        request,
        DaemonRequest::SetStartupProfile(SetStartupProfileRequest {
            gpu_id: GPU.into(),
            profile_id: None,
        })
    );

    // Already the startup profile, and a profile Prism doesn't know: nothing to send.
    assert_eq!(
        intent(
            &mut core,
            Intent::SetStartupProfile {
                gpu_id: GPU.into(),
                profile_id: Some("boost".into()),
            }
        ),
        []
    );
    assert_eq!(
        intent(
            &mut core,
            Intent::SetStartupProfile {
                gpu_id: GPU.into(),
                profile_id: Some("gone".into()),
            }
        ),
        []
    );
}

#[test]
fn another_users_startup_profile_shows_by_name_and_can_be_cleared() {
    let mut core = connected();

    push(
        &mut core,
        DaemonEvent::StartupStateChanged(startup(
            StartupBehavior::StartupProfiles,
            &[("silent", "Silent")],
        )),
    );

    let section = core.startup_settings();
    assert_eq!(section.gpus[0].startup, "Silent");
    assert_eq!(
        choices(&section),
        [
            ("None", false, None),
            ("Quiet", false, None),
            ("Boost", false, None),
            ("Silent", true, Some("Another user's profile")),
        ]
    );
    let (_, request) = command(&intent(
        &mut core,
        Intent::SetStartupProfile {
            gpu_id: GPU.into(),
            profile_id: None,
        },
    ));
    assert_eq!(
        request,
        DaemonRequest::SetStartupProfile(SetStartupProfileRequest {
            gpu_id: GPU.into(),
            profile_id: None,
        })
    );
}

#[test]
fn a_refused_change_is_explained_until_the_next_one() {
    let mut core = connected();

    let (id, _) = command(&intent(
        &mut core,
        Intent::SetStartupProfile {
            gpu_id: GPU.into(),
            profile_id: Some("quiet".into()),
        },
    ));
    answer(
        &mut core,
        id,
        DaemonResponse::Error(GpuCommandError::new(
            GpuCommandErrorCode::Io,
            "The tuning store can't be written.",
        )),
    );
    assert_eq!(
        core.startup_settings().outcome.as_deref(),
        Some("Quiet not made the startup profile: The tuning store can't be written.")
    );

    let (id, _) = command(&intent(
        &mut core,
        Intent::SetStartupBehavior(StartupBehavior::StartupProfiles),
    ));
    assert_eq!(core.startup_settings().outcome, None);
    answer(
        &mut core,
        id,
        DaemonResponse::Command(DaemonCommandResponse {
            success: false,
            error: None,
        }),
    );
    assert_eq!(
        core.startup_settings().outcome.as_deref(),
        Some("Startup behavior not set to Startup profiles: prismd refused it.")
    );
}

#[test]
fn without_tuning_access_the_choices_are_read_only_with_the_reason() {
    let mut core = connected_with(false, |snapshot| {
        snapshot.startup = startup(StartupBehavior::StartupProfiles, &[("quiet", "Quiet")]);
    });

    let section = core.startup_settings();
    assert_eq!(
        section.read_only.as_deref(),
        Some("Needs tuning access · bob isn't in group wheel")
    );
    // Still showing what boot applies.
    assert_eq!(behaviors(&section)[1], ("Startup profiles", true));
    assert_eq!(section.gpus[0].startup, "Quiet");
    assert_eq!(
        intent(&mut core, Intent::SetStartupBehavior(StartupBehavior::Off)),
        []
    );
    assert_eq!(
        intent(
            &mut core,
            Intent::SetStartupProfile {
                gpu_id: GPU.into(),
                profile_id: None,
            }
        ),
        []
    );
}

#[test]
fn without_a_session_the_choices_are_unknown() {
    let mut core = launched();
    core.handle(Input::PrismdNotConnected(SessionFailure::Unavailable {
        message: "No such file or directory (os error 2)".into(),
    }));
    intent(&mut core, Intent::GoTo(Section::Settings));
    intent(&mut core, Intent::GoToSettings(SettingsSection::Startup));

    let section = core.startup_settings();
    assert!(
        section
            .behaviors
            .iter()
            .all(|choice: &StartupBehaviorChoice| !choice.chosen)
    );
    assert!(section.read_only.is_some());
    assert_eq!(section.gpus, []);
    assert_eq!(
        section.empty.as_deref(),
        Some("The startup choices show once Prism is connected to prismd.")
    );
    assert_eq!(
        intent(
            &mut core,
            Intent::SetStartupBehavior(StartupBehavior::StartupProfiles)
        ),
        []
    );
}

#[test]
fn a_lost_session_forgets_the_choices_and_why_a_change_was_refused() {
    let mut core = connected();
    let (id, _) = command(&intent(
        &mut core,
        Intent::SetStartupBehavior(StartupBehavior::StartupProfiles),
    ));
    answer(
        &mut core,
        id,
        DaemonResponse::Error(GpuCommandError::new(GpuCommandErrorCode::Io, "No.")),
    );

    core.handle(Input::PrismdNotConnected(SessionFailure::Unavailable {
        message: "Connection refused".into(),
    }));

    let section = core.startup_settings();
    assert_eq!(section.outcome, None);
    assert!(section.behaviors.iter().all(|choice| !choice.chosen));
}

#[test]
fn gpus_whose_last_applied_values_are_possibly_stale_are_named_while_restoring_them() {
    let mut stale = startup(StartupBehavior::StartupProfiles, &[]);
    stale.possibly_stale = vec![GPU.into()];
    let mut core = connected_with(true, |snapshot| snapshot.startup = stale.clone());

    // Only restoring last applied tuning values uses them.
    assert_eq!(core.startup_settings().possibly_stale, None);

    stale.behavior = StartupBehavior::RestoreLastApplied;
    push(&mut core, DaemonEvent::StartupStateChanged(stale));

    let section = core.startup_settings();
    let gpu = &section.gpus[0].gpu;
    assert_eq!(
        section.possibly_stale,
        Some(format!(
            "A recent change to {gpu} couldn't be recorded for startup, so the next boot may \
             restore older values until a later change is recorded."
        ))
    );
}

#[test]
fn an_unhealthy_tuning_store_suspends_startup_behavior() {
    let mut core = connected();
    assert_eq!(core.startup_settings().suspended, None);

    push(
        &mut core,
        DaemonEvent::StoreHealthChanged(TuningStoreHealth::Unhealthy {
            problem: TuningStoreProblem::Unreadable,
            path: "/var/lib/prism/tuning.json".into(),
            detail: "expected value at line 1".into(),
        }),
    );

    assert_eq!(
        core.startup_settings().suspended.as_deref(),
        Some(
            "Startup behavior is suspended because Prism's tuning store can't be used. \
             Activity explains why."
        )
    );
}

#[test]
fn the_choice_types_carry_what_choosing_them_sends() {
    let core = connected();
    let section = core.startup_settings();

    assert_eq!(
        section
            .behaviors
            .iter()
            .map(|choice| choice.behavior)
            .collect::<Vec<_>>(),
        [
            StartupBehavior::Off,
            StartupBehavior::StartupProfiles,
            StartupBehavior::RestoreLastApplied,
        ]
    );
    assert_eq!(
        section.gpus[0].choices[1],
        StartupChoice {
            profile_id: Some("quiet".into()),
            name: "Quiet".into(),
            chosen: false,
            unavailable: None,
        }
    );
}
