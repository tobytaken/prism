//! View tests for Settings → Appearance, Notifications and About, with a fake settings file that
//! outlives each Prism, so the next one loads what it saved.

mod support;

use i_slint_backend_testing::AccessibleRole;
use prism_gpu::simulated::SimulatedFixture;
use prism_ui::{AppearanceMode, GuiSettings, Link, SettingsSection};
use slint::ComponentHandle;

use support::prismd::DaemonState;
use support::{FakeSettings, Harness};

use AccessibleRole::{Button, ListItem, Switch, Text};

/// Prism, loading `settings`, at the Settings section `section`.
fn at(section: &str, daemon: DaemonState, settings: &FakeSettings) -> Harness {
    let harness = Harness::with_settings(SimulatedFixture::TunableGpu, daemon, settings.clone());
    harness.tick();
    harness.ctrl(",");
    harness.tick();
    harness
        .find(ListItem, section)
        .invoke_accessible_default_action();
    harness.tick();
    harness
}

fn settings(appearance: AppearanceMode, desktop_notifications: bool) -> FakeSettings {
    FakeSettings::new(GuiSettings {
        appearance,
        desktop_notifications,
    })
}

fn press(harness: &Harness, role: AccessibleRole, label: &str) {
    harness.find(role, label).invoke_accessible_default_action();
    harness.tick();
}

fn mode(harness: &Harness) -> prism::AppearanceMode {
    harness.app.window().global::<prism::Theme>().get_mode()
}

fn dark(harness: &Harness) -> bool {
    harness.app.window().global::<prism::Theme>().get_dark()
}

#[test]
fn choosing_an_appearance_applies_it_at_once_and_saves_it() {
    let saved = settings(AppearanceMode::Light, true);
    let harness = at("Appearance", DaemonState::NotRunning, &saved);
    assert_eq!(
        harness.app.core().settings_section(),
        SettingsSection::Appearance
    );
    assert_eq!(
        harness.find(Button, "Light").accessible_checked(),
        Some(true)
    );
    assert!(!dark(&harness));

    press(&harness, Button, "Dark");

    assert_eq!(mode(&harness), prism::AppearanceMode::Dark);
    assert!(dark(&harness));
    assert_eq!(
        harness.find(Button, "Dark").accessible_checked(),
        Some(true)
    );
    assert_eq!(
        harness.find(Button, "Light").accessible_checked(),
        Some(false)
    );
    assert_eq!(
        saved.saved(),
        GuiSettings {
            appearance: AppearanceMode::Dark,
            desktop_notifications: true,
        }
    );
}

#[test]
fn the_appearance_chosen_is_used_after_a_restart() {
    let saved = settings(AppearanceMode::Light, true);
    let harness = at("Appearance", DaemonState::NotRunning, &saved);
    press(&harness, Button, "Dark");
    drop(harness);

    let restarted = Harness::with_settings(
        SimulatedFixture::TunableGpu,
        DaemonState::NotRunning,
        saved.clone(),
    );

    assert_eq!(mode(&restarted), prism::AppearanceMode::Dark);
    assert!(dark(&restarted));
}

#[test]
fn system_follows_the_desktop_again_once_an_override_is_cleared() {
    let saved = settings(AppearanceMode::Dark, true);
    let harness = at("Appearance", DaemonState::NotRunning, &saved);
    assert!(dark(&harness));

    press(&harness, Button, "System");

    // The testing backend reports no desktop preference, which reads as light.
    assert_eq!(mode(&harness), prism::AppearanceMode::System);
    assert!(!dark(&harness));
    assert_eq!(saved.saved().appearance, AppearanceMode::System);
}

#[test]
fn the_palette_chooses_the_appearance() {
    let saved = settings(AppearanceMode::Light, true);
    let harness = Harness::with_settings(
        SimulatedFixture::TunableGpu,
        DaemonState::NotRunning,
        saved.clone(),
    );
    harness.tick();

    harness.ctrl("k");
    harness.type_text("use dark appearance");
    harness.press(&[], slint::platform::Key::Return);
    harness.tick();

    assert!(dark(&harness));
    assert_eq!(saved.saved().appearance, AppearanceMode::Dark);
}

#[test]
fn turning_desktop_notifications_off_saves_it_and_stops_them() {
    let saved = settings(AppearanceMode::Light, true);
    let harness = at("Notifications", DaemonState::Running, &saved);
    assert_eq!(
        harness
            .find(Switch, "Desktop notifications")
            .accessible_checked(),
        Some(true)
    );

    press(&harness, Switch, "Desktop notifications");

    assert_eq!(
        harness
            .find(Switch, "Desktop notifications")
            .accessible_checked(),
        Some(false)
    );
    assert!(!saved.saved().desktop_notifications);
    // A connection still lost after 10 s would notify.
    harness.prismd.stop_daemon();
    harness.app.receive_posted();
    harness.ticks(15);
    assert_eq!(harness.notifier.sent(), []);
}

#[test]
fn desktop_notifications_stay_off_after_a_restart() {
    let saved = settings(AppearanceMode::Light, false);

    let harness = at("Notifications", DaemonState::NotRunning, &saved);

    assert_eq!(
        harness
            .find(Switch, "Desktop notifications")
            .accessible_checked(),
        Some(false)
    );
    press(&harness, Switch, "Desktop notifications");
    assert!(saved.saved().desktop_notifications);
}

#[test]
fn about_shows_the_version_the_license_and_the_slint_credit() {
    let harness = at("About", DaemonState::NotRunning, &FakeSettings::default());

    let fact = |label| {
        harness
            .find(Text, label)
            .accessible_value()
            .map(String::from)
    };
    assert_eq!(fact("Version").as_deref(), Some(env!("CARGO_PKG_VERSION")));
    assert_eq!(fact("License").as_deref(), Some("GPL-3.0-or-later"));
    assert!(harness.shows("Built with Slint"));
}

#[test]
fn about_opens_the_third_party_licenses_and_slints_website() {
    let harness = at("About", DaemonState::NotRunning, &FakeSettings::default());

    press(&harness, Button, "Open third-party licenses");
    press(&harness, Button, "Open Slint's website");

    assert_eq!(
        harness.opener.opened(),
        [Link::ThirdPartyLicenses, Link::SlintWebsite]
    );
}
