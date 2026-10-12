//! View tests for Settings → Daemon: why there's no tuning access and how to fix it, with a fake
//! host that answers for systemd, the Flatpak sandbox and os-release.

mod support;

use i_slint_backend_testing::AccessibleRole;
use prism_gpu::simulated::SimulatedFixture;
use prism_ui::{PrismdInstall, ServiceAction, ServiceOutcome, SettingsSection, UnitState};

use support::Harness;
use support::prismd::DaemonState;

use AccessibleRole::{Button, List, ListItem, Text};

const ENABLE: &str = "sudo systemctl enable --now prismd";

/// Prism with no prismd session, on a host whose prismd is as `install` says.
fn without_prismd(install: PrismdInstall) -> Harness {
    let harness = Harness::with_install(
        SimulatedFixture::TunableGpu,
        DaemonState::NotRunning,
        install,
    );
    harness.tick();
    harness
}

fn native(unit: UnitState) -> Harness {
    without_prismd(PrismdInstall::Native(unit))
}

/// Opens Settings → Daemon through the status bar's prismd item, labeled `item`.
fn open_daemon(harness: &Harness, item: &str) {
    harness
        .find(Button, item)
        .invoke_accessible_default_action();
    harness.tick();
    assert_eq!(
        harness.app.core().settings_section(),
        SettingsSection::Daemon
    );
}

fn press(harness: &Harness, button: &str) {
    harness
        .find(Button, button)
        .invoke_accessible_default_action();
    harness.tick();
}

/// The value of the fact labeled `label`.
fn fact(harness: &Harness, label: &str) -> String {
    harness
        .find(Text, label)
        .accessible_value()
        .map(String::from)
        .unwrap_or_default()
}

#[test]
fn the_status_bar_item_shows_the_reason_and_opens_settings_daemon() {
    let harness = native(UnitState::Inactive { enabled: false });

    open_daemon(&harness, "PRISMD · NOT RUNNING");

    assert_eq!(
        harness.find(ListItem, "Daemon").accessible_item_selected(),
        Some(true)
    );
    assert!(harness.shows("prismd isn't running"));
    assert!(harness.shows("prismd is installed, but isn't enabled."));
    harness.find(Button, "Enable prismd");
}

#[test]
fn set_up_opens_settings_daemon() {
    let harness = native(UnitState::NotFound);
    assert_eq!(
        harness
            .find(Text, "Tuning needs prismd")
            .accessible_value()
            .as_deref(),
        Some("not installed")
    );

    press(&harness, "Set up");

    assert_eq!(
        harness.app.core().settings_section(),
        SettingsSection::Daemon
    );
    assert!(harness.shows("prismd isn't installed"));
}

#[test]
fn settings_has_its_sections_and_only_startup_isnt_built_yet() {
    let harness = native(UnitState::Inactive { enabled: false });
    harness
        .find(ListItem, "Settings")
        .invoke_accessible_default_action();

    let sections: Vec<_> = harness
        .find(List, "Settings sections")
        .query_descendants()
        .match_accessible_role(ListItem)
        .find_all()
        .iter()
        .filter_map(|row| row.accessible_label().map(String::from))
        .collect();
    assert_eq!(
        sections,
        ["Appearance", "Startup", "Notifications", "Daemon", "About"]
    );
    assert!(!harness.shows("This section isn't built yet."));
    harness
        .find(ListItem, "Startup")
        .invoke_accessible_default_action();
    harness.tick();
    assert!(harness.shows("This section isn't built yet."));

    harness
        .find(ListItem, "About")
        .invoke_accessible_default_action();
    assert_eq!(
        harness.app.core().settings_section(),
        SettingsSection::About
    );
}

#[test]
fn not_installed_gives_the_install_commands_with_copy_buttons() {
    let harness = native(UnitState::NotFound);
    open_daemon(&harness, "PRISMD · NOT INSTALLED");

    assert!(harness.shows("Install it on Ubuntu 24.04 LTS, then start it:"));
    assert!(harness.shows("sudo apt install prismd"));
    assert_eq!(harness.count(Button, "Enable prismd"), 0);

    press(&harness, "Copy sudo apt install prismd");
    assert_eq!(harness.clipboard(), "sudo apt install prismd");
    press(&harness, &format!("Copy {ENABLE}"));
    assert_eq!(harness.clipboard(), ENABLE);
}

#[test]
fn enable_starts_prismd_and_prism_connects() {
    let harness = native(UnitState::Inactive { enabled: false });
    open_daemon(&harness, "PRISMD · NOT RUNNING");

    press(&harness, "Enable prismd");
    harness.tick();

    assert_eq!(harness.host.runs(), [ServiceAction::Enable]);
    assert!(harness.shows("Connected to prismd"));
    harness.find(Button, "PRISMD · CONNECTED");
    assert_eq!(fact(&harness, "Tuning access"), "Yes");
    assert_eq!(
        fact(&harness, "Authorized"),
        "as alice, a member of group wheel"
    );
    assert_eq!(fact(&harness, "prismd version"), "0.1.0");
    assert_eq!(fact(&harness, "Protocol"), "0.1");
    assert_eq!(fact(&harness, "Socket"), "/run/prism/prismd.sock");
}

#[test]
fn canceling_the_polkit_prompt_leaves_settings_as_they_were() {
    let harness = native(UnitState::Inactive { enabled: false });
    harness.host.answer_next_run(ServiceOutcome::Canceled);
    open_daemon(&harness, "PRISMD · NOT RUNNING");

    press(&harness, "Enable prismd");

    assert_eq!(harness.host.runs(), [ServiceAction::Enable]);
    assert!(harness.shows("prismd isn't running"));
    harness.find(Button, "Enable prismd");
    assert!(!harness.shows("Couldn't enable prismd"));
}

#[test]
fn failed_shows_systemd_s_result_and_offers_a_restart_and_the_journal_command() {
    let harness = native(UnitState::Failed {
        result: "exit-code".into(),
    });
    open_daemon(&harness, "PRISMD · FAILED");

    assert!(harness.shows("systemd gave up on it with result exit-code: it exited with an error."));
    press(&harness, "Copy journalctl -u prismd -b");
    assert_eq!(harness.clipboard(), "journalctl -u prismd -b");

    harness.host.answer_next_run(ServiceOutcome::Failed {
        message: "Job for prismd.service failed.".into(),
    });
    press(&harness, "Restart prismd");
    assert_eq!(harness.host.runs(), [ServiceAction::Restart]);
    assert!(harness.shows("Couldn't restart prismd: Job for prismd.service failed."));
}

#[test]
fn a_user_without_tuning_access_gets_the_usermod_command() {
    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, DaemonState::ReadOnly);
    harness.tick();
    open_daemon(&harness, "PRISMD · NOT ALLOWED");

    assert!(harness.shows("No tuning access"));
    assert!(harness.shows(
        "User bob isn't in group wheel, so prismd lets this Prism monitor GPUs but not tune them."
    ));
    assert!(harness.shows("Add bob to the group, then log out and back in so it applies:"));
    press(&harness, "Copy sudo usermod -aG wheel bob");
    assert_eq!(harness.clipboard(), "sudo usermod -aG wheel bob");
    assert_eq!(
        fact(&harness, "Tuning access"),
        "No: bob isn't in group wheel"
    );
}

#[test]
fn incompatible_shows_both_versions() {
    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, DaemonState::Incompatible);
    harness.tick();
    open_daemon(&harness, "PRISMD · INCOMPATIBLE");

    assert!(harness.shows("prismd speaks another protocol version"));
    assert_eq!(fact(&harness, "prismd version"), "0.1.0 · protocol 0.1");
    assert!(!fact(&harness, "Prism version").is_empty());
}

#[test]
fn flatpak_offers_commands_to_copy_and_no_buttons_that_need_systemd() {
    let harness = without_prismd(PrismdInstall::Flatpak {
        run_dir_visible: true,
    });
    open_daemon(&harness, "PRISMD · NOT RUNNING");

    assert_eq!(harness.count(Button, "Enable prismd"), 0);
    press(&harness, &format!("Copy {ENABLE}"));
    assert_eq!(harness.clipboard(), ENABLE);

    let harness = without_prismd(PrismdInstall::Flatpak {
        run_dir_visible: false,
    });
    open_daemon(&harness, "PRISMD · NOT INSTALLED");
    assert!(harness.shows("Just installed it? Restart Prism."));
}
