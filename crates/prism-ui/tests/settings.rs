//! The Settings shell: its sections, how to reach each one, and checking the host when Daemon
//! opens without a prismd session. Then Appearance, Notifications and About.

use prism_protocol::{DaemonSnapshot, ProtocolVersion, SessionMessage, Welcome};
use prism_ui::{
    About, AppearanceMode, Content, Core, Effect, GuiSettings, Input, Intent, Link, Section,
    SessionFailure, SettingsSection,
};

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

fn launched() -> Core {
    let mut core = Core::default();
    core.handle(Input::Launched {
        settings: GuiSettings::default(),
        started_unix_ms: 0,
    });
    core
}

fn not_running(core: &mut Core) {
    core.handle(Input::PrismdNotConnected(SessionFailure::Unavailable {
        message: "No such file or directory (os error 2)".into(),
    }));
}

fn connect(core: &mut Core) {
    core.handle(Input::PrismdConnected {
        welcome: Welcome {
            protocol: ProtocolVersion::CURRENT,
            prismd_version: "0.1.0".into(),
            can_tune: true,
            reason: None,
            authorization: None,
        },
        snapshot: snapshot(),
        socket: "/run/prism/prismd.sock".into(),
    });
}

fn intent(core: &mut Core, intent: Intent) -> Vec<Effect> {
    core.handle(Input::Intent(intent))
}

/// The palette's entries matching `query`, as (title, why it can't run).
fn palette(core: &mut Core, query: &str) -> Vec<(String, Option<String>)> {
    intent(core, Intent::OpenPalette);
    intent(core, Intent::FilterPalette(query.into()));
    let entries = core
        .palette()
        .unwrap()
        .entries
        .into_iter()
        .map(|entry| (entry.title, entry.unavailable))
        .collect();
    intent(core, Intent::Dismiss);
    entries
}

#[test]
fn settings_opens_at_appearance_and_remembers_the_last_section() {
    let mut core = launched();

    intent(&mut core, Intent::GoTo(Section::Settings));
    assert_eq!(core.content(), Content::Section(Section::Settings));
    assert_eq!(core.settings_section(), SettingsSection::Appearance);

    intent(&mut core, Intent::GoToSettings(SettingsSection::About));
    intent(&mut core, Intent::GoTo(Section::Games));
    intent(&mut core, Intent::GoTo(Section::Settings));
    assert_eq!(core.settings_section(), SettingsSection::About);
}

#[test]
fn the_palette_goes_to_each_settings_section() {
    let mut core = launched();

    let titles: Vec<_> = palette(&mut core, "go to settings:")
        .into_iter()
        .map(|(title, _)| title)
        .collect();
    assert_eq!(
        titles,
        [
            "Go to Settings: Appearance",
            "Go to Settings: Startup",
            "Go to Settings: Notifications",
            "Go to Settings: Daemon",
            "Go to Settings: About",
        ]
    );

    intent(&mut core, Intent::OpenPalette);
    intent(&mut core, Intent::FilterPalette("settings: daemon".into()));
    intent(&mut core, Intent::RunHighlighted);
    assert_eq!(core.content(), Content::Section(Section::Settings));
    assert_eq!(core.settings_section(), SettingsSection::Daemon);

    assert_eq!(
        palette(&mut core, "settings: daemon"),
        [(
            "Go to Settings: Daemon".to_owned(),
            Some("Already open".to_owned())
        )]
    );
}

#[test]
fn opening_daemon_without_a_session_checks_the_host() {
    let mut core = launched();
    not_running(&mut core);

    assert_eq!(
        intent(&mut core, Intent::GoToSettings(SettingsSection::Daemon)),
        [Effect::CheckPrismdHost]
    );
    // Already open.
    assert_eq!(
        intent(&mut core, Intent::GoToSettings(SettingsSection::Daemon)),
        []
    );

    // Back to Daemon through the sidebar's Settings row, which remembers it.
    intent(&mut core, Intent::GoTo(Section::Games));
    assert_eq!(
        intent(&mut core, Intent::GoTo(Section::Settings)),
        [Effect::CheckPrismdHost]
    );
}

#[test]
fn opening_daemon_with_a_session_checks_nothing() {
    let mut core = launched();
    connect(&mut core);

    assert_eq!(
        intent(&mut core, Intent::GoToSettings(SettingsSection::Daemon)),
        []
    );
}

/// The palette's command titled `title` run, as picking it does.
fn run_command(core: &mut Core, title: &str) -> Vec<Effect> {
    intent(core, Intent::OpenPalette);
    intent(core, Intent::FilterPalette(title.into()));
    let index = core
        .palette()
        .unwrap()
        .entries
        .iter()
        .position(|entry| entry.title == title)
        .unwrap_or_else(|| panic!("the palette lists {title}"));
    intent(core, Intent::RunPaletteEntry(index))
}

fn saved(appearance: AppearanceMode, desktop_notifications: bool) -> Effect {
    Effect::SaveSettings(GuiSettings {
        appearance,
        desktop_notifications,
        compact_mode: false,
    })
}

#[test]
fn choosing_an_appearance_applies_it_and_saves_it() {
    let mut core = launched();

    assert_eq!(
        intent(&mut core, Intent::SetAppearance(AppearanceMode::Dark)),
        [saved(AppearanceMode::Dark, true)]
    );
    assert_eq!(core.appearance(), AppearanceMode::Dark);

    // Already in use, so there's nothing to save.
    assert_eq!(
        intent(&mut core, Intent::SetAppearance(AppearanceMode::Dark)),
        []
    );

    // Back to following the desktop.
    assert_eq!(
        intent(&mut core, Intent::SetAppearance(AppearanceMode::System)),
        [saved(AppearanceMode::System, true)]
    );
    assert_eq!(core.appearance(), AppearanceMode::System);
}

#[test]
fn the_palette_chooses_the_appearance() {
    let mut core = launched();

    assert_eq!(
        palette(&mut core, "appearance"),
        [
            ("Go to Settings: Appearance".to_owned(), None),
            (
                "Use system appearance".to_owned(),
                Some("Already in use".to_owned())
            ),
            ("Use light appearance".to_owned(), None),
            ("Use dark appearance".to_owned(), None),
        ]
    );

    assert_eq!(
        run_command(&mut core, "Use light appearance"),
        [saved(AppearanceMode::Light, true)]
    );
    assert_eq!(core.appearance(), AppearanceMode::Light);
}

#[test]
fn turning_desktop_notifications_off_and_on_saves_it() {
    let mut core = launched();
    assert!(core.desktop_notifications());

    assert_eq!(
        intent(&mut core, Intent::SetDesktopNotifications(false)),
        [saved(AppearanceMode::System, false)]
    );
    assert!(!core.desktop_notifications());
    assert_eq!(
        intent(&mut core, Intent::SetDesktopNotifications(false)),
        []
    );

    assert_eq!(
        intent(&mut core, Intent::SetDesktopNotifications(true)),
        [saved(AppearanceMode::System, true)]
    );
    assert!(core.desktop_notifications());
}

#[test]
fn the_palette_turns_desktop_notifications_off_and_on() {
    let mut core = launched();
    let titles = |core: &mut Core| -> Vec<String> {
        palette(core, "desktop notifications")
            .into_iter()
            .map(|(title, _)| title)
            .collect()
    };

    assert_eq!(titles(&mut core), ["Turn desktop notifications off"]);
    run_command(&mut core, "Turn desktop notifications off");
    assert!(!core.desktop_notifications());

    assert_eq!(titles(&mut core), ["Turn desktop notifications on"]);
    run_command(&mut core, "Turn desktop notifications on");
    assert!(core.desktop_notifications());
}

#[test]
fn prism_starts_with_the_saved_settings() {
    let mut core = Core::default();
    core.handle(Input::Launched {
        settings: GuiSettings {
            appearance: AppearanceMode::Dark,
            desktop_notifications: false,
            compact_mode: false,
        },
        started_unix_ms: 0,
    });

    assert_eq!(core.appearance(), AppearanceMode::Dark);
    assert!(!core.desktop_notifications());
    // Changing one setting keeps the other.
    assert_eq!(
        intent(&mut core, Intent::SetAppearance(AppearanceMode::Light)),
        [saved(AppearanceMode::Light, false)]
    );
}

#[test]
fn about_shows_the_version_the_license_and_the_slint_credit() {
    let core = launched();

    assert_eq!(
        core.about(),
        About {
            version: env!("CARGO_PKG_VERSION").into(),
            license: "GPL-3.0-or-later".into(),
            slint_website: "https://slint.dev".into(),
        }
    );
}

#[test]
fn about_opens_the_third_party_licenses_and_slints_website() {
    let mut core = launched();

    assert_eq!(
        intent(&mut core, Intent::Open(Link::ThirdPartyLicenses)),
        [Effect::Open(Link::ThirdPartyLicenses)]
    );
    assert_eq!(
        intent(&mut core, Intent::Open(Link::SlintWebsite)),
        [Effect::Open(Link::SlintWebsite)]
    );
}
