//! Managing GPU tuning profiles from the workspace header's `⋯` menu and the palette: saving to
//! the current profile or as a new one under the save-blocking rules, renaming, deleting after a
//! confirmation, managing them all and choosing the startup profile, with names unique per GPU
//! regardless of letter case, and "not saved to profile" in the header (ADR 0001).

use std::time::Duration;

use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_gpu::{
    ControlSupport, GpuCommandError, GpuCommandErrorCode, GpuPerformanceResponse,
    GpuTelemetryProvider,
};
use prism_protocol::{
    CurrentProfile, DaemonCommandResponse, DaemonEvent, DaemonRequest, DaemonResponse,
    DaemonSnapshot, GpuTuningState, ProfileIdRequest, ProfileSettingKey, ProfilesState,
    ProtocolVersion, RenameProfileRequest, ReportedTuningValues, SaveProfileAsNewRequest,
    SaveToCurrentProfileRequest, SessionMessage, SetStartupProfileRequest, Welcome,
};
use prism_ui::{
    AppearanceMode, CommandId, Content, Core, Dialog, Effect, FanMode, GpuWorkspace, Input, Intent,
    ManagedProfile, ProfileNameDialog, SessionFailure,
};

const GPU: &str = "GPU-5a1b0000-0000-4000-8000-000000000001";
/// The revision of the tunable GPU's tuning state in the snapshot.
const REVISION: &str = "4f1c2b9d0e8a7c35";

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

fn reading() -> GpuPerformanceResponse {
    SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO).performance()
}

/// Prism connected to prismd with the tunable GPU as read into `sample`, whose snapshot has the
/// caller's profiles.
fn connected_with(can_tune: bool, sample: GpuPerformanceResponse) -> Core {
    let mut core = Core::default();
    core.handle(Input::Launched {
        appearance: AppearanceMode::System,
    });
    let mut snapshot = snapshot();
    snapshot.profiles = profiles();
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

fn connected() -> Core {
    connected_with(true, reading())
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

/// prismd pushes the tunable GPU's tuning state with these values in effect, and these settings
/// following an applied default.
fn push_tuning(core: &mut Core, values: [Option<i64>; 3], defaults: Vec<ProfileSettingKey>) {
    let [power, graphics, memory] = values;
    core.handle(Input::PrismdEvent {
        event: DaemonEvent::TuningStateChanged(GpuTuningState {
            gpu_id: GPU.into(),
            revision: format!("{values:?} {defaults:?}"),
            reported: ReportedTuningValues {
                cooling: None,
                power_limit_watts: power,
                graphics_offset_mhz: graphics,
                memory_offset_mhz: memory,
            },
            defaults,
            current_profile: None,
        }),
        received_at: Duration::from_secs(2),
    });
}

fn workspace(core: &Core) -> GpuWorkspace {
    match core.content() {
        Content::Gpu(workspace) => *workspace,
        other => panic!("expected the GPU workspace, got {other:?}"),
    }
}

/// The open `⋯` menu's entries: each title, with why it can't run when it can't.
fn menu(core: &Core) -> Vec<(String, Option<String>)> {
    core.profile_menu()
        .expect("the profile menu is open")
        .entries
        .into_iter()
        .map(|entry| (entry.title, entry.unavailable))
        .collect()
}

/// Opens the `⋯` menu and runs its entry titled `title`.
fn run_menu(core: &mut Core, title: &str) -> Vec<Effect> {
    if core.profile_menu().is_none() {
        intent(core, Intent::ToggleProfileMenu);
    }
    let index = menu(core)
        .iter()
        .position(|(entry, _)| entry == title)
        .unwrap_or_else(|| panic!("no menu entry {title}: {:?}", menu(core)));
    intent(core, Intent::RunProfileMenuEntry(index))
}

/// Why the menu entry titled `title` can't run, when it can't.
fn menu_unavailable(core: &mut Core, title: &str) -> Option<String> {
    if core.profile_menu().is_none() {
        intent(core, Intent::ToggleProfileMenu);
    }
    let entries = menu(core);
    intent(core, Intent::Dismiss);
    entries
        .into_iter()
        .find(|(entry, _)| entry == title)
        .unwrap_or_else(|| panic!("no menu entry {title}"))
        .1
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

/// The one command sent to prismd among `effects`.
fn command(effects: &[Effect]) -> (CommandId, DaemonRequest) {
    let commands: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::CommandPrismd { id, request } => Some((*id, request.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(commands.len(), 1, "{effects:?}");
    commands[0].clone()
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

fn saved(name: &str) -> DaemonResponse {
    let mut profile = profiles().profiles[1].clone();
    profile.name = name.into();
    DaemonResponse::Profile(profile)
}

fn refused(message: &str) -> DaemonResponse {
    DaemonResponse::Error(GpuCommandError::new(
        GpuCommandErrorCode::InvalidArgument,
        message,
    ))
}

fn name_dialog(core: &Core) -> ProfileNameDialog {
    match core.dialog() {
        Some(Dialog::ProfileName(dialog)) => dialog,
        other => panic!("expected a profile name dialog, got {other:?}"),
    }
}

fn managed(core: &Core) -> Vec<ManagedProfile> {
    match core.dialog() {
        Some(Dialog::ManageProfiles(manage)) => manage.profiles,
        other => panic!("expected Manage profiles, got {other:?}"),
    }
}

fn strip_outcome(core: &Core) -> Option<String> {
    workspace(core).changes.outcome
}

fn save_as_unavailable(core: &mut Core) -> Option<String> {
    menu_unavailable(core, "Save as new profile…")
}

const SAVE_TO_CURRENT: &str = "Save to current profile";
const SAVE_AS: &str = "Save as new profile…";
const RENAME: &str = "Rename profile…";
const DELETE: &str = "Delete profile…";
const MANAGE: &str = "Manage profiles…";

#[test]
fn the_menu_lists_every_profile_action_for_the_current_profile() {
    let mut core = connected();
    assert_eq!(core.profile_menu(), None);

    intent(&mut core, Intent::ToggleProfileMenu);

    assert_eq!(
        menu(&core),
        [
            (SAVE_TO_CURRENT.into(), None),
            (SAVE_AS.into(), None),
            (RENAME.into(), None),
            (DELETE.into(), None),
            ("Use Boost as startup profile".into(), None),
            (MANAGE.into(), None),
        ]
    );
    intent(&mut core, Intent::ToggleProfileMenu);
    assert_eq!(core.profile_menu(), None);
    intent(&mut core, Intent::ToggleProfileMenu);
    intent(&mut core, Intent::Dismiss);
    assert_eq!(core.profile_menu(), None);
}

#[test]
fn saving_to_the_current_profile_sends_the_revision_seen_and_says_how_it_went() {
    let mut core = connected();

    let effects = run_menu(&mut core, SAVE_TO_CURRENT);

    assert_eq!(core.profile_menu(), None);
    let (id, request) = command(&effects);
    assert_eq!(
        request,
        DaemonRequest::SaveToCurrentProfile(SaveToCurrentProfileRequest {
            gpu_id: GPU.into(),
            revision: REVISION.into(),
        })
    );
    assert_eq!(
        menu_unavailable(&mut core, SAVE_TO_CURRENT).as_deref(),
        Some("Saving…")
    );
    answer(&mut core, id, saved("Boost"));
    assert_eq!(strip_outcome(&core).as_deref(), Some("Saved to Boost"));
    assert_eq!(menu_unavailable(&mut core, SAVE_TO_CURRENT), None);

    let (id, _) = command(&run_menu(&mut core, SAVE_TO_CURRENT));
    answer(
        &mut core,
        id,
        refused("The GPU's values changed since you last looked. Review them and save again."),
    );
    assert_eq!(
        strip_outcome(&core).as_deref(),
        Some(
            "Not saved to Boost: The GPU's values changed since you last looked. Review them and \
             save again."
        )
    );
}

#[test]
fn saving_needs_a_current_profile_of_the_callers_own() {
    let mut core = connected();
    let mut none = profiles();
    none.current_profiles.clear();
    push(&mut core, none);
    intent(&mut core, Intent::ToggleProfileMenu);
    assert_eq!(
        menu(&core),
        [
            (SAVE_TO_CURRENT.into(), Some("No current profile".into())),
            (SAVE_AS.into(), None),
            (RENAME.into(), Some("No current profile".into())),
            (DELETE.into(), Some("No current profile".into())),
            (
                "Use current profile as startup profile".into(),
                Some("No current profile".into())
            ),
            (MANAGE.into(), None),
        ]
    );
    intent(&mut core, Intent::Dismiss);

    // Another user's current profile shows by name, but isn't the caller's to change.
    let mut theirs = profiles();
    theirs.current_profiles.insert(
        GPU.into(),
        CurrentProfile {
            id: "alices-quiet".into(),
            name: "Alice's quiet".into(),
        },
    );
    push(&mut core, theirs);
    let another_user = Some("The current profile belongs to another user".to_owned());
    assert_eq!(menu_unavailable(&mut core, SAVE_TO_CURRENT), another_user);
    assert_eq!(menu_unavailable(&mut core, RENAME), another_user);
    assert_eq!(menu_unavailable(&mut core, DELETE), another_user);
    assert_eq!(
        menu_unavailable(&mut core, "Use Alice's quiet as startup profile"),
        another_user
    );
    assert_eq!(requests(&run_menu(&mut core, SAVE_TO_CURRENT)), []);
}

#[test]
fn saving_as_a_new_profile_names_it_and_sends_known_current_settings_without_an_apply() {
    let mut core = connected();

    let effects = run_menu(&mut core, SAVE_AS);

    assert_eq!(requests(&effects), []);
    let dialog = name_dialog(&core);
    assert_eq!(dialog.title, "Save as new profile");
    assert_eq!(dialog.name, "");
    assert_eq!(dialog.confirm_text, "Save");
    assert_eq!(dialog.error, None);
    assert_eq!(
        dialog.confirm_unavailable.as_deref(),
        Some("Enter a name for the profile")
    );
    assert_eq!(requests(&intent(&mut core, Intent::ConfirmProfile)), []);

    intent(&mut core, Intent::EditProfileName("  Silent  ".into()));
    assert_eq!(name_dialog(&core).confirm_unavailable, None);
    let (id, request) = command(&intent(&mut core, Intent::ConfirmProfile));
    assert_eq!(
        request,
        DaemonRequest::SaveProfileAsNew(SaveProfileAsNewRequest {
            gpu_id: GPU.into(),
            name: "Silent".into(),
            revision: REVISION.into(),
        })
    );
    // It waits for prismd's answer.
    assert_eq!(
        name_dialog(&core).confirm_unavailable.as_deref(),
        Some("Saving…")
    );
    assert_eq!(requests(&intent(&mut core, Intent::ConfirmProfile)), []);

    answer(&mut core, id, saved("Silent"));
    assert_eq!(core.dialog(), None);
    assert_eq!(strip_outcome(&core).as_deref(), Some("Saved as Silent"));
    // The header changes only once prismd pushes the profiles.
    let names: Vec<_> = workspace(&core)
        .profiles
        .choices
        .into_iter()
        .map(|choice| choice.name)
        .collect();
    assert_eq!(names, ["Quiet", "Boost"]);
}

#[test]
fn a_refused_save_says_why_in_its_dialog_until_the_name_changes() {
    let mut core = connected();
    run_menu(&mut core, SAVE_AS);
    intent(&mut core, Intent::EditProfileName("Silent".into()));
    let (id, _) = command(&intent(&mut core, Intent::ConfirmProfile));

    answer(
        &mut core,
        id,
        refused("Power limit: support is unknown. Refresh the GPU readings and try again."),
    );

    let dialog = name_dialog(&core);
    assert_eq!(
        dialog.error.as_deref(),
        Some("Power limit: support is unknown. Refresh the GPU readings and try again.")
    );
    assert_eq!(dialog.confirm_unavailable, None);
    intent(&mut core, Intent::EditProfileName("Silent 2".into()));
    assert_eq!(name_dialog(&core).error, None);

    // Answered after the dialog closed, the strip says it instead.
    let (id, _) = command(&intent(&mut core, Intent::ConfirmProfile));
    intent(&mut core, Intent::Dismiss);
    answer(
        &mut core,
        id,
        refused("Failed to write Prism's tuning store."),
    );
    assert_eq!(
        strip_outcome(&core).as_deref(),
        Some("Silent 2 not saved: Failed to write Prism's tuning store.")
    );
}

#[test]
fn names_are_unique_per_gpu_regardless_of_letter_case() {
    let mut core = connected();
    run_menu(&mut core, SAVE_AS);

    intent(&mut core, Intent::EditProfileName("qUIET ".into()));
    let dialog = name_dialog(&core);
    let clash = "A profile named \"Quiet\" already exists for this GPU. Choose another name.";
    assert_eq!(dialog.error.as_deref(), Some(clash));
    assert_eq!(dialog.confirm_unavailable.as_deref(), Some(clash));
    assert_eq!(requests(&intent(&mut core, Intent::ConfirmProfile)), []);

    // Another GPU's profile names don't clash.
    intent(&mut core, Intent::EditProfileName("old CARD".into()));
    assert_eq!(name_dialog(&core).error, None);
    intent(&mut core, Intent::Dismiss);

    // A profile can take another case of its own name, but not another profile's.
    run_menu(&mut core, RENAME);
    intent(&mut core, Intent::EditProfileName("QUIET".into()));
    assert_eq!(name_dialog(&core).error.as_deref(), Some(clash));
    assert_eq!(requests(&intent(&mut core, Intent::ConfirmProfile)), []);
    intent(&mut core, Intent::EditProfileName("BOOST".into()));
    assert_eq!(name_dialog(&core).error, None);
    assert_eq!(
        requests(&intent(&mut core, Intent::ConfirmProfile)),
        [DaemonRequest::RenameProfile(RenameProfileRequest {
            profile_id: "boost".into(),
            name: "BOOST".into(),
        })]
    );
}

#[test]
fn renaming_starts_from_the_profiles_name_and_closes_once_prismd_agrees() {
    let mut core = connected();

    run_menu(&mut core, RENAME);

    let dialog = name_dialog(&core);
    assert_eq!(dialog.title, "Rename Boost");
    assert_eq!(dialog.name, "Boost");
    assert_eq!(dialog.confirm_text, "Rename");
    intent(&mut core, Intent::EditProfileName("Fast".into()));
    let (id, request) = command(&intent(&mut core, Intent::ConfirmProfile));
    assert_eq!(
        request,
        DaemonRequest::RenameProfile(RenameProfileRequest {
            profile_id: "boost".into(),
            name: "Fast".into(),
        })
    );
    assert_eq!(
        name_dialog(&core).confirm_unavailable.as_deref(),
        Some("Renaming…")
    );
    answer(&mut core, id, saved("Fast"));
    assert_eq!(core.dialog(), None);
    // The button keeps its name until prismd pushes the profiles.
    assert_eq!(workspace(&core).profiles.current.as_deref(), Some("Boost"));
}

#[test]
fn deleting_asks_for_confirmation_first() {
    let mut core = connected();

    let effects = run_menu(&mut core, DELETE);

    assert_eq!(requests(&effects), []);
    let Some(Dialog::DeleteProfile(question)) = core.dialog() else {
        panic!("expected the delete confirmation, got {:?}", core.dialog())
    };
    assert_eq!(question.title, "Delete Boost?");
    assert_eq!(question.error, None);
    assert_eq!(requests(&intent(&mut core, Intent::Dismiss)), []);
    assert_eq!(core.dialog(), None);

    run_menu(&mut core, DELETE);
    let (id, request) = command(&intent(&mut core, Intent::ConfirmProfile));
    assert_eq!(
        request,
        DaemonRequest::DeleteProfile(ProfileIdRequest {
            profile_id: "boost".into(),
        })
    );
    answer(&mut core, id, done());
    assert_eq!(core.dialog(), None);
}

#[test]
fn deleting_the_startup_profile_says_the_gpu_will_have_none() {
    let mut core = connected();
    let mut quiet = profiles();
    quiet.current_profiles.insert(
        GPU.into(),
        CurrentProfile {
            id: "quiet".into(),
            name: "Quiet".into(),
        },
    );
    push(&mut core, quiet);

    run_menu(&mut core, DELETE);

    let Some(Dialog::DeleteProfile(question)) = core.dialog() else {
        panic!("expected the delete confirmation, got {:?}", core.dialog())
    };
    assert!(
        question.message.contains("startup profile"),
        "{}",
        question.message
    );
}

#[test]
fn using_a_profile_as_the_startup_profile_is_listed_even_when_it_already_is() {
    let mut core = connected();

    let (id, request) = command(&run_menu(&mut core, "Use Boost as startup profile"));
    assert_eq!(
        request,
        DaemonRequest::SetStartupProfile(SetStartupProfileRequest {
            gpu_id: GPU.into(),
            profile_id: Some("boost".into()),
        })
    );
    answer(&mut core, id, done());
    // The dot moves only once prismd pushes the profiles.
    assert!(!workspace(&core).profiles.choices[1].startup);

    let mut quiet = profiles();
    quiet.current_profiles.insert(
        GPU.into(),
        CurrentProfile {
            id: "quiet".into(),
            name: "Quiet".into(),
        },
    );
    push(&mut core, quiet);
    assert_eq!(
        menu_unavailable(&mut core, "Use Quiet as startup profile").as_deref(),
        Some("Already the startup profile")
    );
    assert_eq!(
        requests(&run_menu(&mut core, "Use Quiet as startup profile")),
        []
    );
}

#[test]
fn managing_lists_the_gpus_profiles_and_renames_or_deletes_any_of_them() {
    let mut core = connected();

    run_menu(&mut core, MANAGE);

    assert_eq!(
        managed(&core),
        [
            ManagedProfile {
                id: "quiet".into(),
                name: "Quiet".into(),
                marks: "Startup profile".into(),
            },
            ManagedProfile {
                id: "boost".into(),
                name: "Boost".into(),
                marks: "Current profile".into(),
            },
        ]
    );

    // Renaming one goes back to the list, whether it's done or canceled.
    intent(&mut core, Intent::RenameProfile("quiet".into()));
    assert_eq!(name_dialog(&core).title, "Rename Quiet");
    intent(&mut core, Intent::Dismiss);
    assert_eq!(managed(&core).len(), 2);
    intent(&mut core, Intent::RenameProfile("quiet".into()));
    intent(&mut core, Intent::EditProfileName("Silent".into()));
    let (id, _) = command(&intent(&mut core, Intent::ConfirmProfile));
    answer(&mut core, id, saved("Silent"));
    assert_eq!(managed(&core).len(), 2);

    // So does deleting one, which still asks first.
    assert_eq!(
        requests(&intent(&mut core, Intent::DeleteProfile("quiet".into()))),
        []
    );
    assert!(matches!(core.dialog(), Some(Dialog::DeleteProfile(_))));
    let (id, request) = command(&intent(&mut core, Intent::ConfirmProfile));
    assert_eq!(
        request,
        DaemonRequest::DeleteProfile(ProfileIdRequest {
            profile_id: "quiet".into(),
        })
    );
    answer(&mut core, id, done());
    let mut deleted = profiles();
    deleted.profiles.retain(|profile| profile.id != "quiet");
    deleted.startup_profiles.clear();
    push(&mut core, deleted);
    assert_eq!(
        managed(&core)
            .into_iter()
            .map(|profile| profile.name)
            .collect::<Vec<_>>(),
        ["Boost"]
    );
    intent(&mut core, Intent::Dismiss);
    assert_eq!(core.dialog(), None);
}

#[test]
fn saving_is_blocked_while_profile_setting_edits_are_unapplied() {
    let mut core = connected();
    intent(
        &mut core,
        Intent::Stage(ProfileSettingKey::PowerLimitWatts, 450),
    );

    let blocked = Some("Apply or discard the unapplied changes first".to_owned());
    assert_eq!(save_as_unavailable(&mut core), blocked);
    assert_eq!(menu_unavailable(&mut core, SAVE_TO_CURRENT), blocked);
    assert_eq!(requests(&run_menu(&mut core, SAVE_TO_CURRENT)), []);
    assert_eq!(core.dialog(), None);
    assert_eq!(requests(&run_menu(&mut core, SAVE_AS)), []);
    assert_eq!(core.dialog(), None);

    intent(&mut core, Intent::DiscardChanges);
    assert_eq!(save_as_unavailable(&mut core), None);
}

#[test]
fn pending_fan_edits_dont_block_saving() {
    let mut core = connected();
    intent(&mut core, Intent::StageFanMode(FanMode::Fixed));
    assert!(workspace(&core).changes.pending);

    assert_eq!(save_as_unavailable(&mut core), None);
    assert_eq!(menu_unavailable(&mut core, SAVE_TO_CURRENT), None);
}

#[test]
fn saving_is_blocked_while_an_apply_waits_for_prismd() {
    let mut core = connected();
    intent(
        &mut core,
        Intent::Stage(ProfileSettingKey::PowerLimitWatts, 450),
    );
    intent(&mut core, Intent::ApplyChanges);

    assert_eq!(save_as_unavailable(&mut core).as_deref(), Some("Applying…"));
}

#[test]
fn unknown_support_blocks_saving() {
    let mut sample = reading();
    sample.gpus[0].power.support = ControlSupport::Unknown;
    let mut core = connected_with(true, sample);

    assert_eq!(
        save_as_unavailable(&mut core).as_deref(),
        Some("Power limit: support is unknown")
    );
}

#[test]
fn an_unavailable_value_blocks_saving() {
    let mut sample = reading();
    sample.gpus[0].clocks.graphics.current_offset_mhz = None;
    let mut core = connected_with(true, sample);
    push_tuning(&mut core, [Some(575), None, Some(0)], Vec::new());

    assert_eq!(
        save_as_unavailable(&mut core).as_deref(),
        Some("Graphics clock offset: the current value is unavailable")
    );
}

#[test]
fn a_gpu_without_supported_settings_cant_be_saved() {
    let mut sample = reading();
    let gpu = &mut sample.gpus[0];
    gpu.power.support = ControlSupport::Unsupported;
    gpu.clocks.graphics.support = ControlSupport::Unsupported;
    gpu.clocks.memory.support = ControlSupport::Unsupported;
    gpu.cooling.is_supported = false;
    let mut core = connected_with(true, sample);

    assert_eq!(
        save_as_unavailable(&mut core).as_deref(),
        Some("This GPU has no supported profile settings")
    );
}

#[test]
fn without_tuning_access_no_profile_action_can_run() {
    let mut core = connected_with(false, reading());
    intent(&mut core, Intent::ToggleProfileMenu);

    let reason = Some("Needs tuning access · bob isn't in group wheel".to_owned());
    for (title, unavailable) in menu(&core) {
        assert_eq!(unavailable, reason, "{title}");
    }
}

#[test]
fn not_saved_to_profile_shows_while_applied_values_differ_from_the_current_profile() {
    let mut core = connected();
    // Boost asks for the default power limit, +150 MHz and +500 MHz; none of it is in effect.
    assert!(workspace(&core).profiles.not_saved_to_profile);

    push_tuning(
        &mut core,
        [Some(575), Some(150), Some(500)],
        vec![ProfileSettingKey::PowerLimitWatts],
    );
    assert!(!workspace(&core).profiles.not_saved_to_profile);

    // The default's number, applied as a number, isn't the default the profile asks for.
    push_tuning(&mut core, [Some(575), Some(150), Some(500)], Vec::new());
    assert!(workspace(&core).profiles.not_saved_to_profile);

    push_tuning(
        &mut core,
        [Some(575), Some(100), Some(500)],
        vec![ProfileSettingKey::PowerLimitWatts],
    );
    assert!(workspace(&core).profiles.not_saved_to_profile);

    // Saving to the profile clears it once prismd pushes the saved values.
    let mut boost = profiles();
    boost.profiles[1].settings.graphics_offset_mhz =
        Some(prism_protocol::ProfileSetting::Value { value: 100 });
    push(&mut core, boost);
    assert!(!workspace(&core).profiles.not_saved_to_profile);
}

#[test]
fn not_saved_to_profile_needs_a_current_profile_whose_values_prism_knows() {
    let mut core = connected();

    let mut none = profiles();
    none.current_profiles.clear();
    push(&mut core, none);
    assert!(!workspace(&core).profiles.not_saved_to_profile);

    let mut theirs = profiles();
    theirs.current_profiles.insert(
        GPU.into(),
        CurrentProfile {
            id: "alices-quiet".into(),
            name: "Alice's quiet".into(),
        },
    );
    push(&mut core, theirs);
    assert!(!workspace(&core).profiles.not_saved_to_profile);

    // A value prismd can't read is no difference it knows of.
    push(&mut core, profiles());
    push_tuning(&mut core, [None, Some(150), Some(500)], Vec::new());
    assert!(!workspace(&core).profiles.not_saved_to_profile);
}

#[test]
fn the_palette_offers_the_profile_actions_and_still_asks_before_deleting() {
    let mut core = connected();
    intent(&mut core, Intent::OpenPalette);
    intent(&mut core, Intent::FilterPalette("profile".into()));
    let titles: Vec<_> = core
        .palette()
        .unwrap()
        .entries
        .into_iter()
        .map(|entry| entry.title)
        .collect();
    for title in [
        SAVE_TO_CURRENT,
        SAVE_AS,
        RENAME,
        DELETE,
        "Use Boost as startup profile",
        MANAGE,
    ] {
        assert!(titles.contains(&title.to_owned()), "{title} in {titles:?}");
    }

    intent(&mut core, Intent::FilterPalette("delete profile".into()));
    let effects = intent(&mut core, Intent::RunHighlighted);
    assert_eq!(requests(&effects), []);
    assert!(!core.palette_open());
    assert!(matches!(core.dialog(), Some(Dialog::DeleteProfile(_))));
}

#[test]
fn a_lost_session_ends_the_profile_dialogs_and_says_what_wasnt_answered() {
    let mut core = connected();
    run_menu(&mut core, SAVE_TO_CURRENT);
    run_menu(&mut core, MANAGE);

    core.handle(Input::PrismdNotConnected(SessionFailure::Unavailable {
        message: "The session with prismd ended".into(),
    }));

    assert_eq!(core.dialog(), None);
    assert_eq!(
        strip_outcome(&core).as_deref(),
        Some(
            "The prismd connection ended before saving to Boost was answered. Check the \
             profile, then save again."
        )
    );
}

#[test]
fn a_dialog_that_opens_closes_the_menu() {
    let mut core = connected();
    let (id, _) = command(&intent(&mut core, Intent::ApplyProfile("quiet".into())));
    intent(&mut core, Intent::ToggleProfileMenu);

    answer(
        &mut core,
        id,
        DaemonResponse::ProfileApplication(prism_protocol::ProfileApplicationResponse {
            rollback: None,
            cooling: None,
            power: None,
            graphics: None,
            memory: None,
            corrections: vec![prism_protocol::ProfileRangeCorrection::Numeric(
                prism_protocol::NumericRangeCorrection {
                    setting: ProfileSettingKey::PowerLimitWatts,
                    saved_value: 650,
                    proposed_value: 600,
                    bound: prism_protocol::RangeBound::Maximum,
                },
            )],
        }),
    );

    assert!(matches!(core.dialog(), Some(Dialog::ProfileCorrections(_))));
    assert_eq!(core.profile_menu(), None);
    // The menu doesn't open over a dialog.
    intent(&mut core, Intent::ToggleProfileMenu);
    assert_eq!(core.profile_menu(), None);
}

#[test]
fn an_unknown_cooling_policy_blocks_saving() {
    let mut core = connected();
    assert_eq!(save_as_unavailable(&mut core), None);

    // prismd couldn't tell how the fans are set, such as after a cooling fault.
    push_tuning(&mut core, [Some(575), Some(0), Some(0)], Vec::new());

    assert_eq!(
        save_as_unavailable(&mut core).as_deref(),
        Some("Cooling: the current policy is unavailable")
    );
}
