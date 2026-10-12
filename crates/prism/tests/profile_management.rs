//! View tests for managing profiles from the workspace header's `⋯` menu: each action sends its
//! request and the header changes only from prismd's events, names that clash show as they're
//! typed, deleting asks first from the menu and the palette, Manage profiles renames and deletes
//! any of the GPU's profiles, and "not saved to profile" shows and clears.

mod support;

use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use prism_gpu::GpuTelemetryProvider;
use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_protocol::{
    DaemonCommandResponse, DaemonEvent, DaemonRequest, DaemonResponse, GpuTuningState,
    ProfileIdRequest, ProfileSettingKey, ProfilesState, RenameProfileRequest,
    SaveProfileAsNewRequest, SaveToCurrentProfileRequest, SetStartupProfileRequest,
};

use support::Harness;
use support::prismd::DaemonState;

use AccessibleRole::{Button, Groupbox, List, ListItem, Text, TextInput};

const GPU: &str = "GPU-5a1b0000-0000-4000-8000-000000000001";
const MENU: &str = "Profile actions";

/// The caller's Quiet and Boost profiles of the tunable GPU, with Quiet as its startup profile and
/// Boost current.
fn profiles() -> ProfilesState {
    serde_json::from_str(include_str!(
        "../../prism-protocol/tests/contracts/profiles-state.json"
    ))
    .unwrap()
}

/// Prism connected to prismd as `daemon` says, with one reading of the tunable GPU and its
/// profiles pushed.
fn connected_as(daemon: DaemonState) -> Harness {
    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, daemon);
    harness.prismd_tick(
        SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO).performance(),
    );
    push(&harness, profiles());
    harness
}

fn connected() -> Harness {
    connected_as(DaemonState::Running)
}

fn push(harness: &Harness, profiles: ProfilesState) {
    harness.prismd.push(DaemonEvent::ProfilesChanged(profiles));
    harness.app.receive_posted();
}

fn answer(harness: &Harness, response: DaemonResponse) {
    harness.prismd.answer(response);
    harness.app.receive_posted();
}

fn click(element: ElementHandle) {
    element.invoke_accessible_default_action();
}

/// Opens the `⋯` menu and runs its entry titled `title`.
fn run_menu(harness: &Harness, title: &str) {
    click(harness.find(Button, MENU));
    click(menu_entry(harness, title));
}

fn menu_entry(harness: &Harness, title: &str) -> ElementHandle {
    harness
        .find(List, MENU)
        .query_descendants()
        .match_accessible_role(ListItem)
        .find_all()
        .into_iter()
        .find(|entry| entry.accessible_label().as_deref() == Some(title))
        .unwrap_or_else(|| panic!("no menu entry {title}"))
}

fn description(element: &ElementHandle) -> String {
    element
        .accessible_description()
        .map(String::from)
        .unwrap_or_default()
}

/// The profile management requests Prism sent to prismd, oldest first.
fn profile_requests(harness: &Harness) -> Vec<DaemonRequest> {
    harness
        .prismd
        .requests()
        .into_iter()
        .filter(|request| {
            matches!(
                request,
                DaemonRequest::SaveToCurrentProfile(_)
                    | DaemonRequest::SaveProfileAsNew(_)
                    | DaemonRequest::RenameProfile(_)
                    | DaemonRequest::DeleteProfile(_)
                    | DaemonRequest::SetStartupProfile(_)
            )
        })
        .collect()
}

fn done() -> DaemonResponse {
    DaemonResponse::Command(DaemonCommandResponse {
        success: true,
        error: None,
    })
}

fn profile_named(name: &str) -> DaemonResponse {
    let mut profile = profiles().profiles[1].clone();
    profile.name = name.into();
    DaemonResponse::Profile(profile)
}

/// What the open dialog says is wrong, if anything.
fn problem(harness: &Harness) -> Option<String> {
    (harness.count(Text, "Problem") > 0).then(|| {
        harness
            .find(Text, "Problem")
            .accessible_value()
            .unwrap()
            .to_string()
    })
}

#[test]
fn the_menu_lists_the_profile_actions_and_escape_closes_it() {
    let harness = connected();
    assert_eq!(harness.count(List, MENU), 0);

    click(harness.find(Button, MENU));

    let titles: Vec<_> = harness
        .find(List, MENU)
        .query_descendants()
        .match_accessible_role(ListItem)
        .find_all()
        .iter()
        .filter_map(|entry| entry.accessible_label().map(String::from))
        .collect();
    assert_eq!(
        titles,
        [
            "Save to current profile",
            "Save as new profile…",
            "Rename profile…",
            "Delete profile…",
            "Use Boost as startup profile",
            "Manage profiles…",
        ]
    );
    harness.press(&[], "\u{1b}");
    assert_eq!(harness.count(List, MENU), 0);
}

#[test]
fn saving_to_the_current_profile_sends_it_and_says_how_it_went() {
    let harness = connected();

    run_menu(&harness, "Save to current profile");

    assert_eq!(harness.count(List, MENU), 0);
    assert_eq!(
        profile_requests(&harness),
        [DaemonRequest::SaveToCurrentProfile(
            SaveToCurrentProfileRequest {
                gpu_id: GPU.into(),
                revision: "4f1c2b9d0e8a7c35".into(),
            }
        )]
    );
    answer(&harness, profile_named("Boost"));
    assert_eq!(
        description(&harness.find(Groupbox, "Unapplied changes")),
        "Saved to Boost"
    );
}

#[test]
fn saving_as_a_new_profile_names_it_in_a_dialog_and_shows_a_name_clash_as_its_typed() {
    let harness = connected();

    run_menu(&harness, "Save as new profile…");
    let dialog = "Save as new profile";
    assert_eq!(harness.count(Groupbox, dialog), 1);
    assert_eq!(
        harness.find(Button, "Save").accessible_enabled(),
        Some(false)
    );

    harness.type_text("qUIET");
    assert_eq!(
        problem(&harness).as_deref(),
        Some("A profile named \"Quiet\" already exists for this GPU. Choose another name.")
    );
    assert_eq!(
        harness.find(Button, "Save").accessible_enabled(),
        Some(false)
    );
    click(harness.find(Button, "Save"));
    assert_eq!(profile_requests(&harness), []);

    harness.type_text(" 2");
    assert_eq!(problem(&harness), None);
    harness.press(&[], "\n");
    assert_eq!(
        profile_requests(&harness),
        [DaemonRequest::SaveProfileAsNew(SaveProfileAsNewRequest {
            gpu_id: GPU.into(),
            name: "qUIET 2".into(),
            revision: "4f1c2b9d0e8a7c35".into(),
        })]
    );
    assert_eq!(description(&harness.find(Button, "Save")), "Saving…");

    answer(&harness, profile_named("qUIET 2"));
    assert_eq!(harness.count(Groupbox, dialog), 0);
    // The header changes only once prismd pushes the profiles.
    assert_eq!(harness.count(Button, "Apply profile qUIET 2"), 0);
    let mut saved = profiles();
    let mut new = saved.profiles[0].clone();
    new.id = "quiet-2".into();
    new.name = "qUIET 2".into();
    saved.profiles.push(new);
    push(&harness, saved);
    assert_eq!(harness.count(Button, "Apply profile qUIET 2"), 1);
}

#[test]
fn a_refused_name_says_why_in_the_dialog() {
    let harness = connected();
    run_menu(&harness, "Rename profile…");
    let input = harness.find(TextInput, "Profile name");
    assert_eq!(input.accessible_value().as_deref(), Some("Boost"));

    // The name starts selected, so typing replaces it.
    harness.type_text("Fast");
    click(harness.find(Button, "Rename"));
    assert_eq!(
        profile_requests(&harness),
        [DaemonRequest::RenameProfile(RenameProfileRequest {
            profile_id: "boost".into(),
            name: "Fast".into(),
        })]
    );
    answer(
        &harness,
        DaemonResponse::Error(prism_gpu::GpuCommandError::new(
            prism_gpu::GpuCommandErrorCode::Io,
            "Failed to write Prism's tuning store.",
        )),
    );
    assert_eq!(
        problem(&harness).as_deref(),
        Some("Failed to write Prism's tuning store.")
    );
    assert_eq!(harness.count(Groupbox, "Rename Boost"), 1);
}

#[test]
fn deleting_from_the_menu_asks_first() {
    let harness = connected();

    run_menu(&harness, "Delete profile…");

    let question = "Delete Boost?";
    assert_eq!(harness.count(Groupbox, question), 1);
    assert_eq!(profile_requests(&harness), []);
    click(harness.find(Button, "Cancel"));
    assert_eq!(harness.count(Groupbox, question), 0);
    assert_eq!(profile_requests(&harness), []);

    run_menu(&harness, "Delete profile…");
    click(harness.find(Button, "Delete"));
    assert_eq!(
        profile_requests(&harness),
        [DaemonRequest::DeleteProfile(ProfileIdRequest {
            profile_id: "boost".into(),
        })]
    );
    answer(&harness, done());
    assert_eq!(harness.count(Groupbox, question), 0);
    // The button goes only once prismd pushes the profiles.
    assert_eq!(harness.count(Button, "Apply profile Boost"), 1);
}

#[test]
fn deleting_from_the_palette_asks_first() {
    let harness = connected();
    harness.ctrl("k");
    harness.type_text("delete profile");
    harness.press(&[], "\n");

    assert_eq!(harness.count(Groupbox, "Delete Boost?"), 1);
    assert_eq!(profile_requests(&harness), []);
}

#[test]
fn using_the_current_profile_at_startup_moves_the_dot_once_prismd_pushes_it() {
    let harness = connected();

    run_menu(&harness, "Use Boost as startup profile");

    assert_eq!(
        profile_requests(&harness),
        [DaemonRequest::SetStartupProfile(SetStartupProfileRequest {
            gpu_id: GPU.into(),
            profile_id: Some("boost".into()),
        })]
    );
    answer(&harness, done());
    let boost = || harness.find(Button, "Apply profile Boost");
    assert_eq!(description(&boost()), "");
    let mut startup = profiles();
    startup.startup_profiles.insert(
        GPU.into(),
        prism_protocol::CurrentProfile {
            id: "boost".into(),
            name: "Boost".into(),
        },
    );
    push(&harness, startup);
    assert_eq!(description(&boost()), "Startup profile");

    // It stays listed once it is, and says so.
    click(harness.find(Button, MENU));
    let entry = menu_entry(&harness, "Use Boost as startup profile");
    assert_eq!(entry.accessible_enabled(), Some(false));
    assert_eq!(description(&entry), "Already the startup profile");
}

#[test]
fn manage_profiles_renames_and_deletes_any_of_the_gpus_profiles() {
    let harness = connected();

    run_menu(&harness, "Manage profiles…");

    let manage = "Manage profiles";
    let quiet = harness.find(Text, "Quiet");
    assert_eq!(quiet.accessible_value().as_deref(), Some("Startup profile"));
    assert_eq!(
        harness.find(Text, "Boost").accessible_value().as_deref(),
        Some("Current profile")
    );

    click(harness.find(Button, "Rename Quiet"));
    assert_eq!(harness.count(Groupbox, "Rename Quiet"), 1);
    harness.type_text("Silent");
    harness.press(&[], "\n");
    answer(&harness, profile_named("Silent"));
    assert_eq!(harness.count(Groupbox, manage), 1);

    click(harness.find(Button, "Delete Quiet"));
    click(harness.find(Button, "Delete"));
    answer(&harness, done());
    assert_eq!(harness.count(Groupbox, manage), 1);
    assert_eq!(
        profile_requests(&harness),
        [
            DaemonRequest::RenameProfile(RenameProfileRequest {
                profile_id: "quiet".into(),
                name: "Silent".into(),
            }),
            DaemonRequest::DeleteProfile(ProfileIdRequest {
                profile_id: "quiet".into(),
            }),
        ]
    );

    let mut deleted = profiles();
    deleted.profiles.retain(|profile| profile.id != "quiet");
    push(&harness, deleted);
    assert_eq!(harness.count(Button, "Delete Quiet"), 0);
    click(harness.find(Button, "Done"));
    assert_eq!(harness.count(Groupbox, manage), 0);
}

#[test]
fn not_saved_to_profile_appears_and_clears_with_the_values_in_effect() {
    let harness = connected();
    let unsaved = || harness.count(Text, "not saved to profile");
    // Boost's values aren't in effect.
    assert_eq!(unsaved(), 1);

    let state = |defaults| GpuTuningState {
        gpu_id: GPU.into(),
        revision: format!("boost {defaults:?}"),
        reported: prism_protocol::ReportedTuningValues {
            cooling: None,
            power_limit_watts: Some(575),
            graphics_offset_mhz: Some(150),
            memory_offset_mhz: Some(500),
        },
        defaults,
        current_profile: None,
    };
    harness
        .prismd
        .push(DaemonEvent::TuningStateChanged(state(vec![
            ProfileSettingKey::PowerLimitWatts,
        ])));
    harness.app.receive_posted();
    assert_eq!(unsaved(), 0);

    harness
        .prismd
        .push(DaemonEvent::TuningStateChanged(state(Vec::new())));
    harness.app.receive_posted();
    assert_eq!(unsaved(), 1);
}

#[test]
fn without_tuning_access_the_menu_says_why_each_action_cant_run() {
    let harness = connected_as(DaemonState::ReadOnly);

    click(harness.find(Button, MENU));

    let entry = menu_entry(&harness, "Save as new profile…");
    assert_eq!(entry.accessible_enabled(), Some(false));
    assert_eq!(
        description(&entry),
        "Needs tuning access · bob isn't in group wheel"
    );
    click(entry);
    assert_eq!(harness.count(Groupbox, "Save as new profile"), 0);
}
