//! The Settings shell: its sections, how to reach each one, and checking the host when Daemon
//! opens without a prismd session.

use prism_protocol::{DaemonSnapshot, ProtocolVersion, SessionMessage, Welcome};
use prism_ui::{
    AppearanceMode, Content, Core, Effect, Input, Intent, Section, SessionFailure, SettingsSection,
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
        appearance: AppearanceMode::System,
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
