//! View tests for Settings → Appearance, Notifications and About, with a fake settings file that
//! outlives each Prism, so the next one loads what it saved. Then Settings → Startup, rendered
//! from prismd's events.

mod support;

use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use prism_gpu::GpuTelemetryProvider;
use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_protocol::{
    CurrentProfile, DaemonEvent, DaemonRequest, ProfilesState, SetStartupBehaviorRequest,
    SetStartupProfileRequest, StartupBehavior, StartupProfile, StartupState,
};
use prism_ui::{AppearanceMode, GuiSettings, Link, SettingsSection};
use slint::ComponentHandle;

use support::prismd::DaemonState;
use support::{FakeSettings, Harness};

use AccessibleRole::{Button, Groupbox, ListItem, Switch, Text};

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
        ..GuiSettings::default()
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
            compact_mode: false,
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

const GPU: &str = "GPU-5a1b0000-0000-4000-8000-000000000001";

/// The caller's Quiet and Boost profiles of the tunable GPU.
fn profiles() -> ProfilesState {
    serde_json::from_str(include_str!(
        "../../prism-protocol/tests/contracts/profiles-state.json"
    ))
    .unwrap()
}

fn startup(behavior: StartupBehavior, startup_profile: Option<(&str, &str)>) -> StartupState {
    StartupState {
        behavior,
        profiles: startup_profile
            .map(|(id, name)| StartupProfile {
                gpu_id: GPU.into(),
                profile: CurrentProfile {
                    id: id.into(),
                    name: name.into(),
                },
            })
            .into_iter()
            .collect(),
        outcomes: vec![],
        possibly_stale: vec![],
    }
}

fn push(harness: &Harness, event: DaemonEvent) {
    harness.prismd.push(event);
    harness.app.receive_posted();
    harness.tick();
}

/// Prism connected to prismd as `daemon` says, with one reading of the tunable GPU and its
/// profiles pushed, at Settings → Startup.
fn at_startup(daemon: DaemonState) -> Harness {
    let harness = at("Startup", daemon, &FakeSettings::default());
    harness.prismd_tick(
        SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO).performance(),
    );
    push(&harness, DaemonEvent::ProfilesChanged(profiles()));
    harness
}

/// The startup requests Prism sent to prismd, oldest first.
fn startup_requests(harness: &Harness) -> Vec<DaemonRequest> {
    harness
        .prismd
        .requests()
        .into_iter()
        .filter(|request| {
            matches!(
                request,
                DaemonRequest::SetStartupBehavior(_) | DaemonRequest::SetStartupProfile(_)
            )
        })
        .collect()
}

/// The group that picks the tunable GPU's startup profile.
fn startup_profile(harness: &Harness) -> ElementHandle {
    let gpu = harness.app.core().startup_settings().gpus[0].gpu.clone();
    harness.find(Groupbox, &format!("Startup profile of {gpu}"))
}

/// The choice named `name` for the tunable GPU's startup profile.
fn startup_choice(harness: &Harness, name: &str) -> ElementHandle {
    startup_profile(harness)
        .query_descendants()
        .match_accessible_role(Button)
        .find_all()
        .into_iter()
        .find(|choice| choice.accessible_label().as_deref() == Some(name))
        .unwrap_or_else(|| panic!("the startup profile has no choice {name}"))
}

fn checked(element: &ElementHandle) -> Option<bool> {
    element.accessible_checked()
}

#[test]
fn startup_renders_the_behavior_and_each_gpus_startup_profile_from_events() {
    let harness = at_startup(DaemonState::Running);
    assert_eq!(checked(&harness.find(Button, "Nothing")), Some(true));
    assert_eq!(checked(&startup_choice(&harness, "None")), Some(true));
    assert_eq!(checked(&startup_choice(&harness, "Quiet")), Some(false));

    push(
        &harness,
        DaemonEvent::StartupStateChanged(startup(
            StartupBehavior::StartupProfiles,
            Some(("quiet", "Quiet")),
        )),
    );

    assert_eq!(checked(&harness.find(Button, "Nothing")), Some(false));
    assert_eq!(
        checked(&harness.find(Button, "Startup profiles")),
        Some(true)
    );
    assert_eq!(checked(&startup_choice(&harness, "None")), Some(false));
    assert_eq!(checked(&startup_choice(&harness, "Quiet")), Some(true));
}

#[test]
fn choosing_a_behavior_or_a_startup_profile_sends_it_and_waits_for_prismd() {
    let harness = at_startup(DaemonState::Running);

    press(&harness, Button, "Last applied tuning values");
    startup_choice(&harness, "Boost").invoke_accessible_default_action();
    harness.tick();

    assert_eq!(
        startup_requests(&harness),
        [
            DaemonRequest::SetStartupBehavior(SetStartupBehaviorRequest {
                behavior: StartupBehavior::RestoreLastApplied,
            }),
            DaemonRequest::SetStartupProfile(SetStartupProfileRequest {
                gpu_id: GPU.into(),
                profile_id: Some("boost".into()),
            }),
        ]
    );
    // Nothing changes until prismd pushes it.
    assert_eq!(checked(&harness.find(Button, "Nothing")), Some(true));
    assert_eq!(checked(&startup_choice(&harness, "None")), Some(true));
}

#[test]
fn startup_warns_about_restoring_last_applied_values_and_explains_boot_and_autostart() {
    let harness = at_startup(DaemonState::Running);

    let notice = |label| {
        harness
            .find(Text, label)
            .accessible_value()
            .map(String::from)
            .unwrap_or_default()
    };
    let warning = notice("Restoring last applied tuning values");
    assert!(
        warning.contains("could crash the host at every boot"),
        "{warning}"
    );
    assert!(warning.contains("unclean shutdown"), "{warning}");
    let boot = notice("At boot");
    assert!(boot.contains("without Prism open"), "{boot}");
    assert!(boot.contains("autostart settings"), "{boot}");
}

#[test]
fn without_tuning_access_startup_is_read_only_with_the_reason() {
    let harness = at_startup(DaemonState::ReadOnly);
    push(
        &harness,
        DaemonEvent::StartupStateChanged(startup(
            StartupBehavior::StartupProfiles,
            Some(("quiet", "Quiet")),
        )),
    );

    let reason = harness
        .app
        .core()
        .startup_settings()
        .read_only
        .expect("read-only without tuning access");
    let behavior = harness.find(Text, "Startup behavior");
    assert_eq!(
        behavior.accessible_value().as_deref(),
        Some("Startup profiles")
    );
    assert_eq!(
        behavior.accessible_description().as_deref(),
        Some(reason.as_str())
    );
    assert_eq!(
        harness
            .find(Text, "Read-only")
            .accessible_value()
            .as_deref(),
        Some(reason.as_str())
    );
    assert_eq!(harness.count(Button, "Nothing"), 0);
}
