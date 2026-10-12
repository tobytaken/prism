//! The accessibility audit: every widget in every fixture state declares the role, label, value
//! and state a screen reader needs.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use prism_gpu::cooling::{CoolingConfiguration, FanSetting};
use prism_gpu::fan_curve::FanCurvePoint;
use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_gpu::{GpuCommandError, GpuCommandErrorCode, GpuTelemetryProvider};
use prism_protocol::{DaemonEvent, DaemonResponse};
use slint::ComponentHandle;

use prism_ui::{PrismdInstall, ServiceAction, SettingsSection, UnitState};
use support::Harness;
use support::audit::{
    CHART_PARTS, Element, TuningAccess, assert_accessible, elements, is_interactive, problems,
};
use support::prismd::DaemonState;

/// Audits the window in `state`, with the tuning access its prismd connection gives: without it,
/// every control must be read-only.
fn audit(state: &str, harness: &Harness) {
    let access = match harness.app.core().tuning_access().reason() {
        None => TuningAccess::Granted,
        Some(_) => TuningAccess::Denied,
    };
    assert_accessible(state, harness.app.window(), access);
}

/// The rows of the sidebar list labeled `list`, by label.
fn rows(harness: &Harness, list: &str) -> Vec<String> {
    harness
        .find(AccessibleRole::List, list)
        .query_descendants()
        .match_accessible_role(AccessibleRole::ListItem)
        .find_all()
        .iter()
        .filter_map(ElementHandle::accessible_label)
        .map(String::from)
        .collect()
}

/// Add each prismd connection reason, tuning access granted and denied, and every menu, popover
/// and dialog open as they land (#120's testing decisions).
#[test]
fn every_fixture_state_is_accessible() {
    for fixture in SimulatedFixture::ALL {
        let name = fixture.name();
        let harness = Harness::new(fixture);
        audit(&format!("{name}, looking for GPUs"), &harness);

        harness.tick();
        audit(name, &harness);

        // The edge-ranges fixture swings each reading to the ends of its range.
        harness.ticks(4);
        audit(&format!("{name}, after five readings"), &harness);

        for gpu in rows(&harness, "GPUs") {
            harness
                .find(AccessibleRole::ListItem, &gpu)
                .invoke_accessible_default_action();
            audit(&format!("{name}, {gpu} selected"), &harness);
        }
        if harness.count(AccessibleRole::Button, "Hardware details") > 0 {
            let details = harness.find(AccessibleRole::Button, "Hardware details");
            details.invoke_accessible_default_action();
            audit(&format!("{name}, hardware details open"), &harness);
            details.invoke_accessible_default_action();
        }
        for section in rows(&harness, "Sections") {
            harness
                .find(AccessibleRole::ListItem, &section)
                .invoke_accessible_default_action();
            audit(&format!("{name}, {section}"), &harness);
        }

        harness.ctrl("k");
        audit(&format!("{name}, command palette"), &harness);
        harness.type_text("zzz");
        audit(
            &format!("{name}, command palette without matches"),
            &harness,
        );
        harness.press(&[], slint::platform::Key::Escape);
    }

    let harness = Harness::new(SimulatedFixture::TunableGpu);
    harness.tick();
    // The minimum window size, where the workspace is narrow and short.
    harness.resize(support::MINIMUM_SIZE);
    audit("tunable, at the minimum window size", &harness);

    let harness = Harness::without_temperature();
    harness.tick();
    audit("tunable, temperature unknown", &harness);

    let harness = Harness::without_readings();
    harness.tick();
    audit("tunable, every reading unknown", &harness);

    let not_running = PrismdInstall::Native(UnitState::Inactive { enabled: false });
    for (daemon, install, state) in [
        (
            DaemonState::Running,
            not_running.clone(),
            "connected to prismd",
        ),
        (
            DaemonState::ReadOnly,
            not_running.clone(),
            "connected to prismd without tuning access",
        ),
        (
            DaemonState::Incompatible,
            not_running.clone(),
            "with an incompatible prismd",
        ),
        (
            DaemonState::NotRunning,
            not_running,
            "with prismd not running",
        ),
        (
            DaemonState::NotRunning,
            PrismdInstall::Native(UnitState::NotFound),
            "with prismd not installed",
        ),
        (
            DaemonState::NotRunning,
            PrismdInstall::Native(UnitState::Failed {
                result: "exit-code".into(),
            }),
            "with prismd failed",
        ),
        (
            DaemonState::NotRunning,
            PrismdInstall::Flatpak {
                run_dir_visible: false,
            },
            "in Flatpak with prismd not installed",
        ),
        (
            DaemonState::NotRunning,
            PrismdInstall::Flatpak {
                run_dir_visible: true,
            },
            "in Flatpak with prismd not running",
        ),
    ] {
        let harness = Harness::with_install(SimulatedFixture::TunableGpu, daemon, install);
        harness.tick();
        audit(&format!("tunable, {state}"), &harness);

        let item = harness.app.core().status_bar().connection;
        harness
            .find(AccessibleRole::Button, &item)
            .invoke_accessible_default_action();
        harness.tick();
        audit(&format!("tunable, {state}, Settings → Daemon"), &harness);
    }
}

#[test]
fn a_pending_power_limit_is_accessible() {
    let (harness, stepper) = power_limit();
    audit("tunable, editable", &harness);

    stepper.set_accessible_value("450");
    audit("tunable, pending power limit", &harness);
    harness.find(AccessibleRole::Button, "Apply");
    harness.find(AccessibleRole::Button, "Discard");

    harness
        .find(AccessibleRole::Spinbox, "Graphics clock offset")
        .set_accessible_value("150");
    harness
        .find(AccessibleRole::Button, "Use default memory clock offset")
        .invoke_accessible_default_action();
    audit("tunable, pending offsets and a default", &harness);
}

#[test]
fn a_fan_curve_is_accessible_pending_and_read_only() {
    let harness = tunable();
    harness
        .find(AccessibleRole::Button, "Fan curve")
        .invoke_accessible_default_action();
    audit("tunable, pending starter fan curve", &harness);
    harness.find(AccessibleRole::Button, "Fan curve editor");

    // Short of full speed at T, the curve has a problem to explain.
    harness
        .find(AccessibleRole::Slider, "Fan curve point 4")
        .invoke_accessible_decrement_action();
    audit("tunable, fan curve with a problem", &harness);
    harness.find(AccessibleRole::Text, "Fan curve problems");

    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, DaemonState::ReadOnly);
    let mut state = support::prismd::snapshot().tuning_states[0].clone();
    state.revision = "curve".into();
    state.reported.cooling = Some(CoolingConfiguration::Shared {
        setting: FanSetting::Curve {
            points: vec![FanCurvePoint::new(45, 30), FanCurvePoint::new(85, 100)],
        },
    });
    harness.prismd.push(DaemonEvent::TuningStateChanged(state));
    harness.prismd_tick(
        SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO).performance(),
    );
    audit("tunable, read-only fan curve in effect", &harness);
    harness.find(AccessibleRole::Slider, "Fan curve point 1");
}

#[test]
fn the_review_and_the_close_confirmation_are_accessible() {
    let (harness, stepper) = power_limit();
    stepper.set_accessible_value("450");
    harness
        .find(AccessibleRole::Button, "Apply")
        .invoke_accessible_default_action();
    // Changed outside Prism while the Apply was on its way.
    let mut state = support::prismd::snapshot().tuning_states[0].clone();
    state.revision = "changed".into();
    state.reported.power_limit_watts = Some(500);
    harness.prismd.push(DaemonEvent::TuningStateChanged(state));
    harness
        .prismd
        .answer(DaemonResponse::Error(GpuCommandError::new(
            GpuCommandErrorCode::StaleRevision,
            "The GPU's tuning values changed since these changes were made.",
        )));
    harness.app.receive_posted();
    audit("tunable, review", &harness);
    harness.find(AccessibleRole::Button, "Apply again");

    harness.press(&[], slint::platform::Key::Escape);
    harness.ctrl("q");
    audit("tunable, close confirmation", &harness);
    harness.find(AccessibleRole::Button, "Keep editing");
    harness.find(AccessibleRole::Button, "Discard and close");
}

/// Activity with an unhealthy tuning store, a startup outcome to dismiss and recent events.
fn with_activity(daemon: DaemonState) -> Harness {
    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, daemon);
    harness.prismd_tick(
        SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO).performance(),
    );
    let gpu = support::prismd::snapshot().inventory.gpus[0].id.clone();
    let mut startup: prism_protocol::StartupState = serde_json::from_str(include_str!(
        "../../prism-protocol/tests/contracts/startup-state.json"
    ))
    .unwrap();
    startup.outcomes[0].gpu_id = gpu;
    let entries: Vec<prism_protocol::ActivityEntry> = serde_json::from_str(include_str!(
        "../../prism-protocol/tests/contracts/activity-entries.json"
    ))
    .unwrap();
    let events = [
        DaemonEvent::StoreHealthChanged(prism_protocol::TuningStoreHealth::Unhealthy {
            problem: prism_protocol::TuningStoreProblem::Unreadable,
            path: "/var/lib/prism/tuning-store.json".into(),
            detail: "expected value at line 1 column 1".into(),
        }),
        DaemonEvent::StartupStateChanged(startup),
    ]
    .into_iter()
    .chain(entries.into_iter().map(DaemonEvent::ActivityRecorded));
    for event in events {
        harness.prismd.push(event);
    }
    harness.app.receive_posted();
    harness.ctrl_shift("a");
    harness
}

#[test]
fn activity_is_accessible_with_and_without_tuning_access() {
    let harness = with_activity(DaemonState::Running);
    audit("tunable, Activity", &harness);
    harness.find(AccessibleRole::Groupbox, "Activity");
    harness.find(
        AccessibleRole::Button,
        "Dismiss · Startup applied Quiet in part",
    );

    let harness = with_activity(DaemonState::ReadOnly);
    audit("tunable, Activity, read-only", &harness);
    let dismiss = harness.find(
        AccessibleRole::Button,
        "Dismiss · Startup applied Quiet in part",
    );
    assert_eq!(dismiss.accessible_enabled(), Some(false));
}

#[test]
fn the_profile_buttons_and_their_dialogs_are_accessible() {
    let profiles: prism_protocol::ProfilesState = serde_json::from_str(include_str!(
        "../../prism-protocol/tests/contracts/profiles-state.json"
    ))
    .unwrap();
    let harness = tunable();
    harness
        .prismd
        .push(DaemonEvent::ProfilesChanged(profiles.clone()));
    harness.app.receive_posted();
    audit("tunable, profiles", &harness);
    harness.find(AccessibleRole::Button, "Apply profile Quiet");

    harness
        .find(AccessibleRole::Spinbox, "Power limit")
        .set_accessible_value("450");
    harness
        .find(AccessibleRole::Button, "Apply profile Quiet")
        .invoke_accessible_default_action();
    audit("tunable, discard for a profile", &harness);
    harness
        .find(AccessibleRole::Button, "Discard and apply")
        .invoke_accessible_default_action();

    let unattempted = Some(prism_gpu::TuningOutcome {
        status: prism_gpu::TuningOutcomeStatus::Unattempted,
        error: Some("out of range".into()),
    });
    harness.prismd.answer(DaemonResponse::ProfileApplication(
        prism_protocol::ProfileApplicationResponse {
            rollback: None,
            cooling: None,
            power: unattempted.clone(),
            graphics: unattempted.clone(),
            memory: unattempted,
            corrections: vec![prism_protocol::ProfileRangeCorrection::Numeric(
                prism_protocol::NumericRangeCorrection {
                    setting: prism_protocol::ProfileSettingKey::PowerLimitWatts,
                    saved_value: 650,
                    proposed_value: 600,
                    bound: prism_protocol::RangeBound::Maximum,
                },
            )],
        },
    ));
    harness.app.receive_posted();
    audit("tunable, profile corrections", &harness);
    harness.find(AccessibleRole::Button, "Adjust profile and apply");

    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, DaemonState::ReadOnly);
    harness.prismd_tick(
        SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO).performance(),
    );
    harness.prismd.push(DaemonEvent::ProfilesChanged(profiles));
    harness.app.receive_posted();
    audit("tunable, read-only profiles", &harness);
    harness.find(AccessibleRole::Text, "Profiles");
    harness
        .find(AccessibleRole::Button, "Profile actions")
        .invoke_accessible_default_action();
    audit("tunable, read-only profile menu", &harness);
    harness.find(AccessibleRole::List, "Profile actions");
}

/// The caller's games: Dota 2 linked to Quiet on the tunable GPU, and Portal 2, not installed.
fn games() -> prism_protocol::GamesState {
    prism_protocol::GamesState {
        automatic_switching: true,
        games: vec![
            prism_protocol::GameEntry {
                game: prism_protocol::GameId::Steam { app_id: 570 },
                name: "Dota 2".into(),
                installed: true,
                profiles: [(
                    "GPU-5a1b0000-0000-4000-8000-000000000001".to_owned(),
                    "quiet".to_owned(),
                )]
                .into(),
            },
            prism_protocol::GameEntry {
                game: prism_protocol::GameId::Steam { app_id: 620 },
                name: "Portal 2".into(),
                installed: false,
                profiles: Default::default(),
            },
        ],
    }
}

/// Games on `harness`, with the tunable GPU's profiles, the caller's games and running games that
/// can't be recognized.
fn show_games(harness: &Harness) {
    harness.prismd.push(DaemonEvent::ProfilesChanged(
        serde_json::from_str(include_str!(
            "../../prism-protocol/tests/contracts/profiles-state.json"
        ))
        .unwrap(),
    ));
    harness.prismd.push(DaemonEvent::GamesChanged(games()));
    harness.prismd.push(DaemonEvent::RunningGamesChanged(
        prism_protocol::RunningGamesState::Unavailable {
            reason: "The first process scan has not completed.".into(),
        },
    ));
    harness.app.receive_posted();
    harness.ctrl_shift("g");
}

#[test]
fn games_add_steam_game_and_add_program_are_accessible() {
    let harness = tunable();
    show_games(&harness);
    audit("tunable, games", &harness);
    harness.find(AccessibleRole::Switch, "Automatic switching");

    harness
        .find(AccessibleRole::Button, "Add Steam game…")
        .invoke_accessible_default_action();
    audit("tunable, add Steam game, reading", &harness);
    harness.prismd.answer(DaemonResponse::SteamApps(vec![
        prism_protocol::SteamApp {
            app_id: 570,
            name: "Dota 2".into(),
        },
        prism_protocol::SteamApp {
            app_id: 1145360,
            name: "Hades".into(),
        },
    ]));
    harness.app.receive_posted();
    audit("tunable, add Steam game", &harness);
    harness.find(AccessibleRole::Button, "Add Hades");
    harness.press(&[], slint::platform::Key::Escape);

    harness
        .find(AccessibleRole::Button, "Add program…")
        .invoke_accessible_default_action();
    audit("tunable, add program, reading", &harness);
    harness.prismd.answer(DaemonResponse::RunningPrograms(vec![
        prism_protocol::RunningProgram {
            pid: 4100,
            name: "Balatro.exe".into(),
            path: Some("/games/Balatro/Balatro.exe".into()),
            launcher: None,
            environment: Vec::new(),
            helper: false,
            recognition_warning: None,
        },
        prism_protocol::RunningProgram {
            pid: 3000,
            name: "game.exe".into(),
            path: None,
            launcher: None,
            environment: Vec::new(),
            helper: false,
            recognition_warning: Some(
                "The host program location could not be recovered; browse for the program \
                     file instead."
                    .into(),
            ),
        },
    ]));
    harness.app.receive_posted();
    audit("tunable, add program", &harness);
    harness.find(AccessibleRole::Button, "Add Balatro.exe");
    harness
        .find(AccessibleRole::Button, "Browse…")
        .invoke_accessible_default_action();
    harness.file_chooser.answer(prism_ui::ProgramFile::Failed(
        "Prism couldn't find where game.exe is on the host: the document portal doesn't know it."
            .into(),
    ));
    harness.app.receive_posted();
    audit("tunable, add program, file not found", &harness);

    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, DaemonState::ReadOnly);
    harness.prismd_tick(
        SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO).performance(),
    );
    show_games(&harness);
    audit("tunable, read-only games", &harness);
    harness.find(AccessibleRole::Button, "Remove Dota 2");
}

/// The contract's driver job in `status`.
fn driver_job(status: prism_drivers::DriverJobStatus) -> prism_drivers::DriverJob {
    let jobs: Vec<prism_drivers::DriverJob> = serde_json::from_str(include_str!(
        "../../prism-protocol/tests/contracts/driver-jobs.json"
    ))
    .unwrap();
    jobs.into_iter().find(|job| job.status == status).unwrap()
}

/// Shows Drivers with the contract's driver status, then pushes each of `jobs`.
fn show_drivers(harness: &Harness, jobs: &[prism_drivers::DriverJobStatus]) {
    harness.ctrl_shift("d");
    harness.prismd.answer(DaemonResponse::DriverStatus(
        serde_json::from_str(include_str!(
            "../../prism-protocol/tests/contracts/driver-status.json"
        ))
        .unwrap(),
    ));
    for status in jobs {
        harness
            .prismd
            .push(DaemonEvent::DriverJobChanged(driver_job(*status)));
    }
    harness.app.receive_posted();
}

#[test]
fn drivers_and_its_confirmations_are_accessible() {
    use prism_drivers::DriverJobStatus::{RebootRequired, Running};

    let harness = tunable();
    show_drivers(&harness, &[]);
    audit("tunable, drivers", &harness);
    harness
        .find(AccessibleRole::Button, "Run recommended driver plan…")
        .invoke_accessible_default_action();
    audit("tunable, drivers, run confirmation", &harness);
    harness.press(&[], slint::platform::Key::Escape);

    harness
        .prismd
        .push(DaemonEvent::DriverJobChanged(driver_job(Running)));
    harness.app.receive_posted();
    audit("tunable, drivers, job running", &harness);
    harness
        .prismd
        .push(DaemonEvent::DriverJobChanged(driver_job(RebootRequired)));
    harness.app.receive_posted();
    audit("tunable, drivers, reboot required", &harness);
    harness
        .find(AccessibleRole::Button, "Reboot now…")
        .invoke_accessible_default_action();
    audit("tunable, drivers, reboot confirmation", &harness);

    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, DaemonState::ReadOnly);
    harness.prismd_tick(
        SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO).performance(),
    );
    show_drivers(&harness, &[Running]);
    audit("tunable, read-only drivers", &harness);
}

#[test]
fn a_switch_s_default_action_toggles_it_unless_it_s_read_only() {
    let harness = tunable();
    show_games(&harness);

    harness
        .find(AccessibleRole::Switch, "Automatic switching")
        .invoke_accessible_default_action();

    assert_eq!(
        harness.prismd.requests().last(),
        Some(&prism_protocol::DaemonRequest::SetAutomaticSwitching(
            prism_protocol::SetAutomaticSwitchingRequest { enabled: false }
        ))
    );

    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, DaemonState::ReadOnly);
    harness.prismd_tick(
        SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO).performance(),
    );
    show_games(&harness);
    let requests = harness.prismd.requests().len();

    harness
        .find(AccessibleRole::Switch, "Automatic switching")
        .invoke_accessible_default_action();

    assert_eq!(harness.prismd.requests().len(), requests);
}

/// Opens the `⋯` profile menu of the tunable GPU, with its Quiet and Boost profiles, and runs the
/// entry titled `title`.
fn run_profile_menu(harness: &Harness, title: &str) {
    harness
        .find(AccessibleRole::Button, "Profile actions")
        .invoke_accessible_default_action();
    harness
        .find(AccessibleRole::ListItem, title)
        .invoke_accessible_default_action();
}

fn with_profiles() -> Harness {
    let harness = tunable();
    harness.prismd.push(DaemonEvent::ProfilesChanged(
        serde_json::from_str(include_str!(
            "../../prism-protocol/tests/contracts/profiles-state.json"
        ))
        .unwrap(),
    ));
    harness.app.receive_posted();
    harness
}

#[test]
fn the_profile_menu_and_its_dialogs_are_accessible() {
    let harness = with_profiles();
    harness
        .find(AccessibleRole::Button, "Profile actions")
        .invoke_accessible_default_action();
    audit("tunable, profile menu", &harness);
    harness.find(AccessibleRole::List, "Profile actions");
    harness.press(&[], "\u{1b}");

    run_profile_menu(&harness, "Save as new profile…");
    harness.type_text("Quiet");
    audit("tunable, save as a new profile, name clash", &harness);
    harness.find(AccessibleRole::TextInput, "Profile name");
    harness.find(AccessibleRole::Text, "Problem");
    harness.press(&[], "\u{1b}");

    run_profile_menu(&harness, "Rename profile…");
    audit("tunable, rename a profile", &harness);
    harness.find(AccessibleRole::Groupbox, "Rename Boost");
    harness.press(&[], "\u{1b}");

    run_profile_menu(&harness, "Delete profile…");
    audit("tunable, delete a profile", &harness);
    harness.find(AccessibleRole::Groupbox, "Delete Boost?");
    harness.press(&[], "\u{1b}");

    run_profile_menu(&harness, "Manage profiles…");
    audit("tunable, manage profiles", &harness);
    harness.find(AccessibleRole::Button, "Rename Quiet");
    harness.find(AccessibleRole::Button, "Delete Boost");
}

#[test]
fn a_menu_item_s_default_action_runs_it_unless_it_can_t_run() {
    let harness = with_profiles();
    harness
        .find(AccessibleRole::Button, "Profile actions")
        .invoke_accessible_default_action();
    harness.prismd.push(DaemonEvent::ProfilesChanged(
        prism_protocol::ProfilesState::default(),
    ));
    harness.app.receive_posted();

    // Without a current profile, there's nothing to rename.
    harness
        .find(AccessibleRole::ListItem, "Rename profile…")
        .invoke_accessible_default_action();
    assert_eq!(harness.count(AccessibleRole::List, "Profile actions"), 1);

    harness
        .find(AccessibleRole::ListItem, "Save as new profile…")
        .invoke_accessible_default_action();
    assert_eq!(harness.count(AccessibleRole::List, "Profile actions"), 0);
    assert_eq!(
        harness.count(AccessibleRole::Groupbox, "Save as new profile"),
        1
    );
}

/// Every `accessible-action-*` a widget or view declares, and the test below that invokes it.
const ACTION_TESTS: &[(&str, &str, &str)] = &[
    (
        "Button",
        "default",
        "a_button_s_default_action_activates_it_unless_it_s_disabled",
    ),
    (
        "ConnectionItem",
        "default",
        "the_connection_item_s_default_action_opens_settings_daemon",
    ),
    (
        "CurveHandle",
        "decrement",
        "a_curve_point_s_increment_and_decrement_actions_raise_and_lower_its_speed",
    ),
    (
        "CurveHandle",
        "increment",
        "a_curve_point_s_increment_and_decrement_actions_raise_and_lower_its_speed",
    ),
    (
        "CurveToggle",
        "default",
        "the_curve_toggle_s_default_action_collapses_and_opens_the_fan_curve_editor",
    ),
    (
        "Disclosure",
        "default",
        "a_disclosure_s_default_action_opens_and_collapses_what_it_discloses",
    ),
    (
        "LinkButton",
        "default",
        "a_link_button_s_default_action_activates_it",
    ),
    (
        "ListRow",
        "default",
        "a_list_row_s_default_action_activates_it",
    ),
    (
        "MenuItem",
        "default",
        "a_menu_item_s_default_action_runs_it_unless_it_can_t_run",
    ),
    (
        "PaletteRow",
        "default",
        "a_palette_row_s_default_action_runs_its_command",
    ),
    (
        "ProblemIndicator",
        "default",
        "the_problem_indicator_s_default_action_opens_activity",
    ),
    ("Segment", "default", "a_segment_s_default_action_picks_it"),
    (
        "Stepper",
        "decrement",
        "a_stepper_s_increment_and_decrement_actions_step_it",
    ),
    (
        "Stepper",
        "increment",
        "a_stepper_s_increment_and_decrement_actions_step_it",
    ),
    (
        "Stepper",
        "set-value(text)",
        "a_stepper_s_set_value_action_sets_it_within_its_range",
    ),
    (
        "Switch",
        "default",
        "a_switch_s_default_action_toggles_it_unless_it_s_read_only",
    ),
    (
        "WindowButton",
        "default",
        "the_window_buttons_default_actions_minimize_maximize_and_close",
    ),
];

#[test]
fn every_declared_accessible_action_has_a_test() {
    let ui = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
    let mut declared = Vec::new();
    for file in slint_files(&ui) {
        let source = fs::read_to_string(&file).unwrap();
        let mut component = None;
        for line in source.lines().map(str::trim) {
            let line = line.strip_prefix("export ").unwrap_or(line);
            if let Some(rest) = line.strip_prefix("component ") {
                component = rest.split_whitespace().next();
            }
            if let Some(rest) = line.strip_prefix("accessible-action-") {
                let action = rest.split([' ', '=']).next().unwrap();
                let component = component.expect("an action inside a component");
                declared.push((component.to_owned(), action.to_owned()));
            }
        }
    }
    declared.sort();

    let tests = include_str!("accessibility.rs");
    for (_, _, test) in ACTION_TESTS {
        assert!(
            tests.contains(&format!("#[test]\nfn {test}()")),
            "ACTION_TESTS names {test}, which isn't a test here"
        );
    }
    let mut tested: Vec<_> = ACTION_TESTS
        .iter()
        .map(|(component, action, _)| (component.to_string(), action.to_string()))
        .collect();
    tested.sort();
    assert_eq!(
        declared, tested,
        "each declared accessible action needs a test that invokes it, listed in ACTION_TESTS"
    );
}

fn slint_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(slint_files(&path));
        } else if path
            .extension()
            .is_some_and(|extension| extension == "slint")
        {
            files.push(path);
        }
    }
    files
}

#[test]
fn a_list_row_s_default_action_activates_it() {
    let harness = Harness::new(SimulatedFixture::TwoIdenticalGpus);
    harness.tick();
    let second_gpu = harness.find(AccessibleRole::ListItem, "GPU 1 · RTX 4090 · 02:00.0");
    let drivers = harness.find(AccessibleRole::ListItem, "Drivers");

    second_gpu.invoke_accessible_default_action();
    assert_eq!(second_gpu.accessible_item_selected(), Some(true));

    drivers.invoke_accessible_default_action();
    assert_eq!(drivers.accessible_item_selected(), Some(true));
    assert_eq!(second_gpu.accessible_item_selected(), Some(false));
}

#[test]
fn a_palette_row_s_default_action_runs_its_command() {
    let harness = Harness::new(SimulatedFixture::TwoIdenticalGpus);
    harness.tick();
    harness.ctrl("k");

    harness
        .find(AccessibleRole::ListItem, "Go to Games")
        .invoke_accessible_default_action();

    assert_eq!(
        harness.count(AccessibleRole::Groupbox, "Command palette"),
        0
    );
    assert_eq!(
        harness
            .find(AccessibleRole::ListItem, "Games")
            .accessible_item_selected(),
        Some(true)
    );
}

#[test]
fn a_button_s_default_action_activates_it_unless_it_s_disabled() {
    let harness = Harness::new(SimulatedFixture::TunableGpu);
    harness.tick();
    harness.ctrl(",");
    harness
        .find(AccessibleRole::ListItem, "Daemon")
        .invoke_accessible_default_action();
    harness.tick();

    harness
        .find(AccessibleRole::Button, "Enable prismd")
        .invoke_accessible_default_action();
    // Until Prism takes in how it ended, it's still running, and the button is disabled.
    let enabling = harness.find(AccessibleRole::Button, "Enabling prismd…");
    assert_eq!(enabling.accessible_enabled(), Some(false));
    enabling.invoke_accessible_default_action();

    assert_eq!(harness.host.runs(), [ServiceAction::Enable]);
}

#[test]
fn the_connection_item_s_default_action_opens_settings_daemon() {
    let harness = Harness::new(SimulatedFixture::TunableGpu);
    harness.tick();

    harness
        .find(AccessibleRole::Button, "PRISMD · NOT RUNNING")
        .invoke_accessible_default_action();

    assert_eq!(
        harness.app.core().settings_section(),
        SettingsSection::Daemon
    );
    assert_eq!(
        harness
            .find(AccessibleRole::ListItem, "Daemon")
            .accessible_item_selected(),
        Some(true)
    );
}

#[test]
fn the_problem_indicator_s_default_action_opens_activity() {
    let harness = Harness::new(SimulatedFixture::TunableGpu);
    harness.tick();

    harness
        .find(AccessibleRole::Button, "NO PROBLEMS")
        .invoke_accessible_default_action();

    assert!(harness.app.core().activity_open());
    assert_eq!(harness.count(AccessibleRole::Groupbox, "Activity"), 1);
}

#[test]
fn a_link_button_s_default_action_activates_it() {
    let harness = Harness::new(SimulatedFixture::TunableGpu);
    harness.tick();

    harness
        .find(AccessibleRole::Button, "Set up")
        .invoke_accessible_default_action();

    assert_eq!(
        harness
            .find(AccessibleRole::ListItem, "Settings")
            .accessible_item_selected(),
        Some(true)
    );
}

#[test]
fn a_disclosure_s_default_action_opens_and_collapses_what_it_discloses() {
    let harness = Harness::new(SimulatedFixture::TunableGpu);
    // Tall enough to show what it discloses without scrolling.
    harness.resize((1280, 1100));
    harness.tick();
    let details = harness.find(AccessibleRole::Button, "Hardware details");

    details.invoke_accessible_default_action();
    assert_eq!(details.accessible_expanded(), Some(true));
    assert_eq!(harness.count(AccessibleRole::Text, "Driver"), 1);

    details.invoke_accessible_default_action();
    assert_eq!(details.accessible_expanded(), Some(false));
    assert_eq!(harness.count(AccessibleRole::Text, "Driver"), 0);
}

#[test]
fn a_segment_s_default_action_picks_it() {
    let harness = Harness::new(SimulatedFixture::TunableGpu);
    harness.tick();
    let fifteen_minutes = harness.find(AccessibleRole::Button, "Show last 15 minutes");
    assert_eq!(fifteen_minutes.accessible_checked(), Some(false));

    fifteen_minutes.invoke_accessible_default_action();

    assert_eq!(fifteen_minutes.accessible_checked(), Some(true));
    assert_eq!(
        harness
            .find(AccessibleRole::Button, "Show last 5 minutes")
            .accessible_checked(),
        Some(false)
    );
}

/// Prism with tuning access to the tunable GPU and the starter fan curve pending, and the
/// editor's point `number`.
fn starter_curve_point(number: usize) -> (Harness, ElementHandle) {
    let harness = tunable();
    harness
        .find(AccessibleRole::Button, "Fan curve")
        .invoke_accessible_default_action();
    let point = harness.find(AccessibleRole::Slider, &format!("Fan curve point {number}"));
    (harness, point)
}

#[test]
fn a_curve_point_s_increment_and_decrement_actions_raise_and_lower_its_speed() {
    let (_harness, point) = starter_curve_point(2);
    assert_eq!(point.accessible_value().as_deref(), Some("65% at 65 °C"));

    point.invoke_accessible_increment_action();
    assert_eq!(point.accessible_value().as_deref(), Some("66% at 65 °C"));

    point.invoke_accessible_decrement_action();
    point.invoke_accessible_decrement_action();
    assert_eq!(point.accessible_value().as_deref(), Some("64% at 65 °C"));
}

#[test]
fn the_curve_toggle_s_default_action_collapses_and_opens_the_fan_curve_editor() {
    let (harness, _) = starter_curve_point(1);
    let toggle = harness.find(AccessibleRole::Button, "Fan curve editor");

    toggle.invoke_accessible_default_action();
    assert_eq!(toggle.accessible_expanded(), Some(false));
    assert_eq!(harness.count(AccessibleRole::Groupbox, "Fan curve"), 0);

    toggle.invoke_accessible_default_action();
    assert_eq!(toggle.accessible_expanded(), Some(true));
    assert_eq!(harness.count(AccessibleRole::Groupbox, "Fan curve"), 1);
}

/// Prism with tuning access to the tunable GPU.
fn tunable() -> Harness {
    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, DaemonState::Running);
    harness.prismd_tick(
        SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO).performance(),
    );
    harness
}

/// Prism with tuning access to the tunable GPU, whose power limit is 575 W in a 400–600 W range,
/// and its power limit's stepper.
fn power_limit() -> (Harness, ElementHandle) {
    let harness = tunable();
    let stepper = harness.find(AccessibleRole::Spinbox, "Power limit");
    (harness, stepper)
}

/// The pending value the stepper draws next to itself, as its description says it.
fn pending(stepper: &ElementHandle) -> String {
    stepper
        .accessible_description()
        .map(String::from)
        .unwrap_or_default()
}

#[test]
fn a_stepper_s_increment_and_decrement_actions_step_it() {
    let (_harness, stepper) = power_limit();
    assert_eq!(stepper.accessible_value().as_deref(), Some("575 W"));

    stepper.invoke_accessible_decrement_action();
    assert_eq!(stepper.accessible_value().as_deref(), Some("574 W"));
    assert_eq!(pending(&stepper), "In effect 575 W, → 574 W");

    stepper.invoke_accessible_increment_action();
    stepper.invoke_accessible_increment_action();
    assert_eq!(stepper.accessible_value().as_deref(), Some("576 W"));
    assert_eq!(pending(&stepper), "In effect 575 W, → 576 W");
}

#[test]
fn a_stepper_s_set_value_action_sets_it_within_its_range() {
    let (_harness, stepper) = power_limit();

    stepper.set_accessible_value("450");
    assert_eq!(stepper.accessible_value().as_deref(), Some("450 W"));
    assert_eq!(pending(&stepper), "In effect 575 W, → 450 W");

    stepper.set_accessible_value("9999");
    assert_eq!(stepper.accessible_value().as_deref(), Some("600 W"));
}

#[test]
fn the_window_buttons_default_actions_minimize_maximize_and_close() {
    let harness = Harness::new(SimulatedFixture::TunableGpu);
    let window = harness.app.window().window();
    let press = |label| {
        harness
            .find(AccessibleRole::Button, label)
            .invoke_accessible_default_action()
    };

    press("Maximize");
    assert!(window.is_maximized());
    press("Maximize");
    assert!(!window.is_maximized());

    press("Minimize");
    assert!(window.is_minimized());

    press("Close");
    assert!(!window.is_visible());
}

#[test]
fn the_audit_sees_the_window_s_interactive_elements_and_chart_parts() {
    let harness = Harness::new(SimulatedFixture::TwoIdenticalGpus);
    // A series needs two readings.
    harness.ticks(2);

    let interactive: Vec<_> = elements(harness.app.window())
        .into_iter()
        .filter(|element| is_interactive(element.role))
        .filter_map(|element| element.label)
        .collect();
    let mut chart_parts: Vec<_> = elements(harness.app.window())
        .into_iter()
        .filter(|element| CHART_PARTS.contains(&element.type_name.as_str()))
        .map(|element| (element.type_name, element.role))
        .collect();
    chart_parts.sort_by(|a, b| a.0.cmp(&b.0));
    chart_parts.dedup();

    assert_eq!(
        interactive,
        [
            "Minimize",
            "Maximize",
            "Close",
            "GPU 0 · RTX 4090 · 01:00.0",
            "GPU 1 · RTX 4090 · 02:00.0",
            "Games",
            "Drivers",
            "Settings",
            "Profile actions",
            "Show last 1 minute",
            "Show last 5 minutes",
            "Show last 15 minutes",
            "Show last 1 hour",
            "Set up",
            "Power limit",
            "Graphics clock offset",
            "Memory clock offset",
            "Hardware details",
            "PRISMD · NOT RUNNING",
            "NO PROBLEMS",
        ]
    );
    // Each part appears, with no role.
    assert_eq!(
        chart_parts,
        [
            ("Area".to_owned(), AccessibleRole::None),
            ("Grid".to_owned(), AccessibleRole::None),
            ("LimitLine".to_owned(), AccessibleRole::None),
            ("Series".to_owned(), AccessibleRole::None),
        ]
    );
}

/// An element as the audit sees it, with the role and label set and nothing else.
fn element(type_name: &str, id: &str, role: AccessibleRole, label: &str) -> Element {
    Element {
        type_name: type_name.into(),
        id: id.into(),
        role,
        label: Some(label.into()),
        ..Element::default()
    }
}

fn spinbox() -> Element {
    Element {
        value: Some("300 W".into()),
        minimum: Some(100.0),
        maximum: Some(450.0),
        step: Some(5.0),
        ..element(
            "Stepper",
            "PowerTrack::limit",
            AccessibleRole::Spinbox,
            "Power limit",
        )
    }
}

fn switch() -> Element {
    Element {
        checkable: Some(true),
        checked: Some(false),
        ..element(
            "Toggle",
            "FansTrack::curve",
            AccessibleRole::Switch,
            "Fan curve",
        )
    }
}

#[test]
fn an_interactive_element_without_a_label_fails() {
    let unlabeled = Element {
        label: None,
        ..element("WindowButton", "", AccessibleRole::Button, "")
    };
    let blank = element("SectionRow", "Sidebar::games", AccessibleRole::ListItem, "");

    assert_eq!(
        problems(&[unlabeled, blank], TuningAccess::Granted),
        [
            "WindowButton <no id> (Button): no label",
            "SectionRow Sidebar::games (ListItem): no label",
        ]
    );
}

#[test]
fn text_and_containers_need_no_label() {
    let elements = [
        element("StyledText", "", AccessibleRole::Text, ""),
        element("Rectangle", "", AccessibleRole::None, ""),
        element("Sidebar", "", AccessibleRole::Groupbox, ""),
    ];

    assert_eq!(
        problems(&elements, TuningAccess::Granted),
        Vec::<String>::new()
    );
}

#[test]
fn a_spinbox_or_slider_without_its_value_and_range_fails() {
    let without_range = Element {
        minimum: None,
        step: None,
        ..spinbox()
    };
    let without_value = Element {
        role: AccessibleRole::Slider,
        value: None,
        maximum: None,
        ..spinbox()
    };

    assert_eq!(
        problems(&[without_range, without_value], TuningAccess::Granted),
        [
            "Stepper PowerTrack::limit (Spinbox \"Power limit\"): no minimum",
            "Stepper PowerTrack::limit (Spinbox \"Power limit\"): no step",
            "Stepper PowerTrack::limit (Slider \"Power limit\"): no value",
            "Stepper PowerTrack::limit (Slider \"Power limit\"): no maximum",
        ]
    );
}

#[test]
fn a_switch_or_checkbox_without_its_checked_state_fails() {
    let not_checkable = Element {
        checkable: None,
        ..switch()
    };
    let without_checked = Element {
        role: AccessibleRole::Checkbox,
        checked: None,
        ..switch()
    };

    assert_eq!(
        problems(&[not_checkable, without_checked], TuningAccess::Granted),
        [
            "Toggle FansTrack::curve (Switch \"Fan curve\"): not checkable",
            "Toggle FansTrack::curve (Checkbox \"Fan curve\"): no checked state",
        ]
    );
}

#[test]
fn a_control_that_stays_editable_without_tuning_access_fails() {
    let read_only = |element: Element| Element {
        read_only: Some(true),
        ..element
    };
    let editable = [spinbox(), switch()];
    let locked = editable.clone().map(read_only);

    assert_eq!(
        problems(&editable, TuningAccess::Granted),
        Vec::<String>::new()
    );
    assert_eq!(
        problems(&locked, TuningAccess::Denied),
        Vec::<String>::new()
    );
    assert_eq!(
        problems(&editable, TuningAccess::Denied),
        [
            "Stepper PowerTrack::limit (Spinbox \"Power limit\"): not read-only",
            "Toggle FansTrack::curve (Switch \"Fan curve\"): not read-only",
        ]
    );
}

#[test]
fn a_text_field_may_stay_editable_without_tuning_access() {
    let field = element(
        "TextInput",
        "TextField::input",
        AccessibleRole::TextInput,
        "Command",
    );

    assert_eq!(
        problems(&[field], TuningAccess::Denied),
        Vec::<String>::new()
    );
}

#[test]
fn a_chart_part_with_an_accessible_role_fails() {
    let series = Element {
        bases: vec!["Path".into()],
        ..element("Series", "", AccessibleRole::Image, "")
    };
    let unrolled = Element {
        role: AccessibleRole::None,
        ..series.clone()
    };

    assert_eq!(
        problems(&[series, unrolled], TuningAccess::Granted),
        ["Series <no id> (Image): a chart part with a role; the track's readout carries the value"]
    );
}
