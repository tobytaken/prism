//! View tests for the Games section: the automatic switching toggle, each game's profile link per
//! GPU and removing it, rendered from prismd's events; Add Steam game; read-only without tuning
//! access; and recognition problems explained.

mod support;

use std::collections::BTreeMap;
use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_gpu::{GpuCommandError, GpuCommandErrorCode, GpuTelemetryProvider};
use prism_protocol::{
    AddSteamGameRequest, DaemonEvent, DaemonRequest, DaemonResponse, GameEntry, GameId,
    GameRequest, GamesState, ProfilesState, RunningGamesState, SetAutomaticSwitchingRequest,
    SetGameProfileRequest, SteamApp,
};

use support::Harness;
use support::prismd::DaemonState;

use AccessibleRole::{Button, Groupbox, Switch, Text};

const GPU: &str = "GPU-5a1b0000-0000-4000-8000-000000000001";
const DOTA: GameId = GameId::Steam { app_id: 570 };

/// The caller's Quiet and Boost profiles of the tunable GPU.
fn profiles() -> ProfilesState {
    serde_json::from_str(include_str!(
        "../../prism-protocol/tests/contracts/profiles-state.json"
    ))
    .unwrap()
}

/// Dota 2 linked to Quiet on the tunable GPU, and Portal 2, not installed.
fn games(automatic_switching: bool) -> GamesState {
    GamesState {
        automatic_switching,
        games: vec![
            GameEntry {
                game: DOTA,
                name: "Dota 2".into(),
                installed: true,
                profiles: BTreeMap::from([(GPU.to_owned(), "quiet".to_owned())]),
            },
            GameEntry {
                game: GameId::Steam { app_id: 620 },
                name: "Portal 2".into(),
                installed: false,
                profiles: BTreeMap::new(),
            },
        ],
    }
}

/// Prism connected to prismd as `daemon` says, with one reading of the tunable GPU, its profiles
/// and the caller's games pushed, showing Games.
fn connected_as(daemon: DaemonState) -> Harness {
    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, daemon);
    harness.prismd_tick(
        SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO).performance(),
    );
    push(&harness, DaemonEvent::ProfilesChanged(profiles()));
    push(&harness, DaemonEvent::GamesChanged(games(false)));
    harness.ctrl_shift("g");
    harness
}

fn connected() -> Harness {
    connected_as(DaemonState::Running)
}

fn push(harness: &Harness, event: DaemonEvent) {
    harness.prismd.push(event);
    harness.app.receive_posted();
}

fn answer(harness: &Harness, response: DaemonResponse) {
    harness.prismd.answer(response);
    harness.app.receive_posted();
}

fn description(element: &ElementHandle) -> String {
    element
        .accessible_description()
        .map(String::from)
        .unwrap_or_default()
}

/// The game requests Prism sent to prismd, oldest first.
fn game_requests(harness: &Harness) -> Vec<DaemonRequest> {
    harness
        .prismd
        .requests()
        .into_iter()
        .filter(|request| {
            matches!(
                request,
                DaemonRequest::SetAutomaticSwitching(_)
                    | DaemonRequest::SetGameProfile(_)
                    | DaemonRequest::RemoveGame(_)
                    | DaemonRequest::ListSteamApps
                    | DaemonRequest::AddSteamGame(_)
            )
        })
        .collect()
}

/// The group that links `game` to a profile of the tunable GPU.
fn link(harness: &Harness, game: &str) -> ElementHandle {
    let gpu = harness.app.core().games().games[0].links[0].gpu.clone();
    harness.find(Groupbox, &format!("{game} on {gpu}"))
}

/// The choice named `name` in `game`'s link on the tunable GPU.
fn link_choice(harness: &Harness, game: &str, name: &str) -> ElementHandle {
    link(harness, game)
        .query_descendants()
        .match_accessible_role(Button)
        .find_all()
        .into_iter()
        .find(|choice| choice.accessible_label().as_deref() == Some(name))
        .unwrap_or_else(|| panic!("{game}'s link has no choice {name}"))
}

fn switch(harness: &Harness) -> ElementHandle {
    harness.find(Switch, "Automatic switching")
}

#[test]
fn the_toggle_and_each_games_link_render_from_events() {
    let harness = connected();

    assert_eq!(switch(&harness).accessible_checked(), Some(false));
    assert_eq!(
        description(&harness.find(Groupbox, "Dota 2")),
        "Steam app 570"
    );
    assert_eq!(
        description(&harness.find(Groupbox, "Portal 2")),
        "Steam app 620 · Not installed"
    );
    assert_eq!(
        link_choice(&harness, "Dota 2", "Quiet").accessible_checked(),
        Some(true)
    );
    assert_eq!(
        link_choice(&harness, "Dota 2", "None").accessible_checked(),
        Some(false)
    );
    assert_eq!(
        link_choice(&harness, "Portal 2", "None").accessible_checked(),
        Some(true)
    );

    let mut changed = games(true);
    changed.games.remove(1);
    changed.games[0].profiles.insert(GPU.into(), "boost".into());
    push(&harness, DaemonEvent::GamesChanged(changed));

    assert_eq!(switch(&harness).accessible_checked(), Some(true));
    assert_eq!(harness.count(Groupbox, "Portal 2"), 0);
    assert_eq!(
        link_choice(&harness, "Dota 2", "Boost").accessible_checked(),
        Some(true)
    );

    push(&harness, DaemonEvent::GamesChanged(GamesState::default()));
    assert_eq!(harness.count(Groupbox, "Dota 2"), 0);
    assert!(harness.shows("No games yet. Add a Steam game to link it to your profiles."));
}

#[test]
fn the_toggle_linking_and_removing_send_their_requests() {
    let harness = connected();

    switch(&harness).invoke_accessible_default_action();
    link_choice(&harness, "Portal 2", "Boost").invoke_accessible_default_action();
    link_choice(&harness, "Dota 2", "None").invoke_accessible_default_action();
    harness
        .find(Button, "Remove Dota 2")
        .invoke_accessible_default_action();

    assert_eq!(
        game_requests(&harness),
        [
            DaemonRequest::SetAutomaticSwitching(SetAutomaticSwitchingRequest { enabled: true }),
            DaemonRequest::SetGameProfile(SetGameProfileRequest {
                game: GameId::Steam { app_id: 620 },
                gpu_id: GPU.into(),
                profile_id: Some("boost".into()),
            }),
            DaemonRequest::SetGameProfile(SetGameProfileRequest {
                game: DOTA,
                gpu_id: GPU.into(),
                profile_id: None,
            }),
            DaemonRequest::RemoveGame(GameRequest { game: DOTA }),
        ]
    );
    // Nothing changes until prismd pushes it.
    assert_eq!(switch(&harness).accessible_checked(), Some(false));
    assert_eq!(harness.count(Groupbox, "Dota 2"), 1);
}

#[test]
fn a_refused_change_says_why() {
    let harness = connected();
    harness
        .find(Button, "Remove Dota 2")
        .invoke_accessible_default_action();

    answer(
        &harness,
        DaemonResponse::Error(GpuCommandError::new(
            GpuCommandErrorCode::Io,
            "The tuning store can't be written.",
        )),
    );

    assert_eq!(
        harness.find(Text, "Problem").accessible_value().as_deref(),
        Some("Dota 2 not removed: The tuning store can't be written.")
    );
}

#[test]
fn add_steam_game_lists_the_callers_steam_apps_and_adds_the_one_chosen() {
    let harness = connected();

    harness
        .find(Button, "Add Steam game…")
        .invoke_accessible_default_action();
    assert_eq!(game_requests(&harness), [DaemonRequest::ListSteamApps]);
    assert_eq!(harness.count(Groupbox, "Add Steam game"), 1);
    assert!(harness.shows("Reading your Steam libraries…"));

    answer(
        &harness,
        DaemonResponse::SteamApps(vec![
            SteamApp {
                app_id: 570,
                name: "Dota 2".into(),
            },
            SteamApp {
                app_id: 1145360,
                name: "Hades".into(),
            },
        ]),
    );
    let dota = harness.find(Button, "Add Dota 2");
    assert_eq!(dota.accessible_enabled(), Some(false));
    assert_eq!(description(&dota), "Already added");

    harness.type_text("hds");
    assert_eq!(harness.count(Button, "Add Dota 2"), 0);
    harness
        .find(Button, "Add Hades")
        .invoke_accessible_default_action();
    assert_eq!(
        game_requests(&harness)[1..],
        [DaemonRequest::AddSteamGame(AddSteamGameRequest {
            app_id: 1145360
        })]
    );

    answer(&harness, DaemonResponse::Games(games(false)));
    assert_eq!(harness.count(Groupbox, "Add Steam game"), 0);
}

#[test]
fn without_tuning_access_games_are_read_only_with_the_reason() {
    let harness = connected_as(DaemonState::ReadOnly);
    let reason = harness.app.core().games().read_only.unwrap();
    assert!(reason.starts_with("Needs tuning access · "), "{reason}");

    let switch = switch(&harness);
    assert_eq!(switch.accessible_read_only(), Some(true));
    assert_eq!(description(&switch), reason);
    let add = harness.find(Button, "Add Steam game…");
    assert_eq!(add.accessible_enabled(), Some(false));
    assert_eq!(description(&add), reason);
    assert_eq!(
        harness.find(Button, "Remove Dota 2").accessible_enabled(),
        Some(false)
    );
    // The link shows only what it is, and why it can't change.
    let link = link(&harness, "Dota 2");
    assert_eq!(link.accessible_read_only(), Some(true));
    let shown = link
        .query_descendants()
        .match_accessible_role(Text)
        .find_first()
        .unwrap();
    assert_eq!(shown.accessible_value().as_deref(), Some("Quiet"));

    switch.invoke_accessible_default_action();
    assert_eq!(game_requests(&harness), []);
}

#[test]
fn recognition_unavailability_is_shown_with_its_reason() {
    let harness = connected();
    assert_eq!(harness.count(Text, "Recognition problem"), 0);

    push(
        &harness,
        DaemonEvent::RunningGamesChanged(RunningGamesState::Unavailable {
            reason: "A complete host process scan could not be read.".into(),
        }),
    );

    let problem = harness.find(Text, "Recognition problem");
    assert_eq!(
        problem.accessible_value().as_deref(),
        Some(
            "Prism can't recognize running games, so automatic switching can't follow them: A \
             complete host process scan could not be read."
        )
    );
}
