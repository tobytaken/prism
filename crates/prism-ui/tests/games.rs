//! The Games section: the automatic switching toggle, the caller's games with a profile link per
//! GPU, removing a game and adding a Steam game or a program, all from prismd's events, read-only
//! without tuning access, with recognition problems explained, and the palette's game commands.

use std::collections::BTreeMap;
use std::time::Duration;

use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_gpu::{GpuCommandError, GpuCommandErrorCode, GpuTelemetryProvider};
use prism_protocol::{
    AddProgramRequest, AddSteamGameRequest, DaemonEvent, DaemonRequest, DaemonResponse,
    DaemonSnapshot, GameEntry, GameId, GameRequest, GamesState, ProfilesState, ProtocolVersion,
    RunningGamesState, RunningProgram, SessionMessage, SetAutomaticSwitchingRequest,
    SetGameProfileRequest, SteamApp, Welcome,
};
use prism_ui::{
    AddProgram, AddSteamGame, AppearanceMode, CommandId, Core, Dialog, Effect, GameRow,
    GamesSection, Input, Intent, ProgramFile, Section, SessionFailure,
};

const GPU: &str = "GPU-5a1b0000-0000-4000-8000-000000000001";
const DOTA: GameId = GameId::Steam { app_id: 570 };

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

fn game(game: GameId, name: &str, installed: bool, links: &[(&str, &str)]) -> GameEntry {
    GameEntry {
        game,
        name: name.into(),
        installed,
        profiles: links
            .iter()
            .map(|(gpu, profile)| ((*gpu).to_owned(), (*profile).to_owned()))
            .collect::<BTreeMap<_, _>>(),
    }
}

/// Dota 2 linked to Quiet on the tunable GPU, Portal 2 not installed, and a program; switching
/// off.
fn games() -> GamesState {
    GamesState {
        automatic_switching: false,
        games: vec![
            game(DOTA, "Dota 2", true, &[(GPU, "quiet")]),
            game(GameId::Steam { app_id: 620 }, "Portal 2", false, &[]),
            game(
                GameId::Program {
                    path: "/opt/games/factorio/bin/factorio".into(),
                },
                "factorio",
                true,
                &[],
            ),
        ],
    }
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

/// Prism connected to prismd with the tunable GPU, whose snapshot has the caller's profiles and
/// games, showing Games.
fn connected_with(can_tune: bool, change: impl FnOnce(&mut DaemonSnapshot)) -> Core {
    let mut core = Core::default();
    core.handle(Input::Launched {
        appearance: AppearanceMode::System,
        started_unix_ms: 0,
    });
    let mut snapshot = snapshot();
    snapshot.profiles = profiles();
    snapshot.games = games();
    change(&mut snapshot);
    core.handle(Input::PrismdConnected {
        welcome: welcome(can_tune),
        snapshot,
        socket: "/run/prism/prismd.sock".into(),
    });
    core.handle(Input::PrismdEvent {
        event: DaemonEvent::TelemetrySampled(
            SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO).performance(),
        ),
        received_at: Duration::from_secs(1),
    });
    intent(&mut core, Intent::GoTo(Section::Games));
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

/// The one command sent to prismd among `effects`, with the id its answer comes back with.
fn sent_once(effects: &[Effect]) -> (CommandId, DaemonRequest) {
    let sent: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::CommandPrismd { id, request } => Some((*id, request.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(sent.len(), 1, "{effects:?}");
    sent[0].clone()
}

fn answer(core: &mut Core, id: CommandId, response: DaemonResponse) {
    core.handle(Input::PrismdAnswered { id, response });
}

fn refused(message: &str) -> DaemonResponse {
    DaemonResponse::Error(GpuCommandError::new(
        GpuCommandErrorCode::InvalidArgument,
        message,
    ))
}

fn section(core: &Core) -> GamesSection {
    core.games()
}

fn row(core: &Core, name: &str) -> GameRow {
    section(core)
        .games
        .into_iter()
        .find(|row| row.name == name)
        .unwrap_or_else(|| panic!("no game named {name}"))
}

/// The names of the profile choices for the game on the tunable GPU, the linked one in brackets.
fn link_choices(core: &Core, name: &str) -> Vec<String> {
    let row = row(core, name);
    assert_eq!(row.links.len(), 1, "{row:?}");
    row.links[0]
        .choices
        .iter()
        .map(|choice| {
            if choice.linked {
                format!("[{}]", choice.name)
            } else {
                choice.name.clone()
            }
        })
        .collect()
}

fn add_steam_game_dialog(core: &Core) -> AddSteamGame {
    match core.dialog() {
        Some(Dialog::AddSteamGame(dialog)) => dialog,
        other => panic!("expected Add Steam game, got {other:?}"),
    }
}

fn add_program_dialog(core: &Core) -> AddProgram {
    match core.dialog() {
        Some(Dialog::AddProgram(dialog)) => dialog,
        other => panic!("expected Add program, got {other:?}"),
    }
}

/// One of the caller's running programs, at its host location if known.
fn program(pid: u32, name: &str, path: Option<&str>, warning: Option<&str>) -> RunningProgram {
    RunningProgram {
        pid,
        name: name.into(),
        path: path.map(Into::into),
        launcher: None,
        environment: Vec::new(),
        helper: false,
        recognition_warning: warning.map(Into::into),
    }
}

const UNRECOVERED: &str =
    "The host program location could not be recovered; browse for the program file instead.";
const SCRIPTS: &str = "Scripts, virtual machines and emulators identify the runtime, not the \
                       individual game; recognition may be unreliable.";

fn palette_titles(core: &mut Core, query: &str) -> Vec<(String, Option<String>)> {
    intent(core, Intent::OpenPalette);
    intent(core, Intent::FilterPalette(query.into()));
    let entries = core.palette().unwrap().entries;
    intent(core, Intent::Dismiss);
    entries
        .into_iter()
        .map(|entry| (entry.title, entry.unavailable))
        .collect()
}

#[test]
fn games_show_from_the_snapshot_with_the_toggle_and_each_gpus_linked_profile() {
    let core = connected();

    let section = section(&core);
    assert_eq!(section.automatic_switching, Some(false));
    assert_eq!(
        section.empty,
        "No games yet. Add a Steam game or a program to link it to your profiles."
    );
    assert_eq!(section.read_only, None);
    assert_eq!(section.recognition_problem, None);
    let names: Vec<_> = section.games.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(names, ["Dota 2", "Portal 2", "factorio"]);
    assert_eq!(row(&core, "Dota 2").detail, "Steam app 570");
    assert_eq!(
        row(&core, "Portal 2").detail,
        "Steam app 620 · Not installed"
    );
    assert_eq!(
        row(&core, "factorio").detail,
        "/opt/games/factorio/bin/factorio"
    );
    // Each GPU here gets a link, from its own profiles; Old card's GPU isn't here.
    let dota = row(&core, "Dota 2");
    assert_eq!(dota.links[0].gpu_id, GPU);
    assert!(
        dota.links[0].gpu.starts_with("GPU 0"),
        "{}",
        dota.links[0].gpu
    );
    assert_eq!(dota.links[0].linked, "Quiet");
    assert_eq!(link_choices(&core, "Dota 2"), ["None", "[Quiet]", "Boost"]);
    assert_eq!(
        link_choices(&core, "Portal 2"),
        ["[None]", "Quiet", "Boost"]
    );
    assert_eq!(row(&core, "Portal 2").links[0].linked, "None");
}

#[test]
fn games_follow_pushed_changes_and_not_the_requests_sent() {
    let mut core = connected();

    let effects = intent(&mut core, Intent::SetAutomaticSwitching(true));
    assert_eq!(
        requests(&effects),
        [DaemonRequest::SetAutomaticSwitching(
            SetAutomaticSwitchingRequest { enabled: true }
        )]
    );
    // Only prismd's push changes what shows.
    assert_eq!(section(&core).automatic_switching, Some(false));

    let mut changed = games();
    changed.automatic_switching = true;
    changed.games.remove(2);
    changed.games[0].profiles.insert(GPU.into(), "boost".into());
    push(&mut core, DaemonEvent::GamesChanged(changed));

    assert_eq!(section(&core).automatic_switching, Some(true));
    assert_eq!(section(&core).games.len(), 2);
    assert_eq!(link_choices(&core, "Dota 2"), ["None", "Quiet", "[Boost]"]);

    let effects = intent(&mut core, Intent::SetAutomaticSwitching(false));
    assert_eq!(
        requests(&effects),
        [DaemonRequest::SetAutomaticSwitching(
            SetAutomaticSwitchingRequest { enabled: false }
        )]
    );
}

#[test]
fn linking_a_profile_or_none_and_removing_a_game_send_their_requests() {
    let mut core = connected();

    let effects = intent(
        &mut core,
        Intent::LinkGame {
            game: GameId::Steam { app_id: 620 },
            gpu_id: GPU.into(),
            profile_id: Some("boost".into()),
        },
    );
    assert_eq!(
        requests(&effects),
        [DaemonRequest::SetGameProfile(SetGameProfileRequest {
            game: GameId::Steam { app_id: 620 },
            gpu_id: GPU.into(),
            profile_id: Some("boost".into()),
        })]
    );

    let effects = intent(
        &mut core,
        Intent::LinkGame {
            game: DOTA,
            gpu_id: GPU.into(),
            profile_id: None,
        },
    );
    assert_eq!(
        requests(&effects),
        [DaemonRequest::SetGameProfile(SetGameProfileRequest {
            game: DOTA,
            gpu_id: GPU.into(),
            profile_id: None,
        })]
    );

    // Choosing the profile already linked sends nothing.
    let effects = intent(
        &mut core,
        Intent::LinkGame {
            game: DOTA,
            gpu_id: GPU.into(),
            profile_id: Some("quiet".into()),
        },
    );
    assert_eq!(requests(&effects), []);

    let effects = intent(&mut core, Intent::RemoveGame(DOTA));
    assert_eq!(
        requests(&effects),
        [DaemonRequest::RemoveGame(GameRequest { game: DOTA })]
    );
    // It stays until prismd pushes the games without it.
    assert_eq!(section(&core).games.len(), 3);
}

#[test]
fn a_refused_game_change_says_why_until_the_next_one() {
    let mut core = connected();

    let (id, _) = sent_once(&intent(&mut core, Intent::RemoveGame(DOTA)));
    answer(&mut core, id, refused("The tuning store can't be written."));
    assert_eq!(
        section(&core).outcome.as_deref(),
        Some("Dota 2 not removed: The tuning store can't be written.")
    );

    let (id, _) = sent_once(&intent(
        &mut core,
        Intent::LinkGame {
            game: DOTA,
            gpu_id: GPU.into(),
            profile_id: Some("boost".into()),
        },
    ));
    assert_eq!(section(&core).outcome, None);
    answer(
        &mut core,
        id,
        refused("The profile belongs to another GPU."),
    );
    assert_eq!(
        section(&core).outcome.as_deref(),
        Some("Dota 2 not linked to Boost: The profile belongs to another GPU.")
    );

    let (id, _) = sent_once(&intent(&mut core, Intent::SetAutomaticSwitching(true)));
    answer(&mut core, id, refused("No."));
    assert_eq!(
        section(&core).outcome.as_deref(),
        Some("Automatic switching not turned on: No.")
    );

    // An accepted change says nothing: the push shows it.
    let (id, _) = sent_once(&intent(&mut core, Intent::SetAutomaticSwitching(true)));
    answer(&mut core, id, DaemonResponse::Games(games()));
    assert_eq!(section(&core).outcome, None);
}

#[test]
fn add_steam_game_lists_the_callers_steam_apps_and_adds_the_one_chosen() {
    let mut core = connected();

    let (listing, request) = sent_once(&intent(&mut core, Intent::AddSteamGame));
    assert_eq!(request, DaemonRequest::ListSteamApps);
    let dialog = add_steam_game_dialog(&core);
    assert_eq!(dialog.title, "Add Steam game");
    assert_eq!(
        dialog.status.as_deref(),
        Some("Reading your Steam libraries…")
    );
    assert_eq!(dialog.apps, []);

    answer(
        &mut core,
        listing,
        DaemonResponse::SteamApps(vec![
            SteamApp {
                app_id: 1086940,
                name: "Baldur's Gate 3".into(),
            },
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
    let dialog = add_steam_game_dialog(&core);
    assert_eq!(dialog.status, None);
    let apps: Vec<_> = dialog
        .apps
        .iter()
        .map(|app| (app.name.as_str(), app.unavailable.as_deref()))
        .collect();
    assert_eq!(
        apps,
        [
            ("Baldur's Gate 3", None),
            ("Dota 2", Some("Already added")),
            ("Hades", None),
        ]
    );

    intent(&mut core, Intent::FilterSteamApps("hds".into()));
    let dialog = add_steam_game_dialog(&core);
    assert_eq!(dialog.filter, "hds");
    let names: Vec<_> = dialog.apps.iter().map(|app| app.name.as_str()).collect();
    assert_eq!(names, ["Hades"]);

    // One that's already added can't be added again.
    assert_eq!(requests(&intent(&mut core, Intent::AddSteamApp(570))), []);

    let (adding, request) = sent_once(&intent(&mut core, Intent::AddSteamApp(1145360)));
    assert_eq!(
        request,
        DaemonRequest::AddSteamGame(AddSteamGameRequest { app_id: 1145360 })
    );
    assert_eq!(
        add_steam_game_dialog(&core).status.as_deref(),
        Some("Adding Hades…")
    );
    // Only one at a time.
    assert_eq!(
        requests(&intent(&mut core, Intent::AddSteamApp(1086940))),
        []
    );

    answer(&mut core, adding, DaemonResponse::Games(games()));
    assert!(!core.dialog_open());
}

#[test]
fn add_steam_game_says_why_listing_or_adding_failed() {
    let mut core = connected();
    let (listing, _) = sent_once(&intent(&mut core, Intent::AddSteamGame));
    answer(
        &mut core,
        listing,
        refused("Prism couldn't read your Steam libraries as you: denied"),
    );
    assert_eq!(
        add_steam_game_dialog(&core).status.as_deref(),
        Some("Prism couldn't read your Steam libraries as you: denied")
    );
    intent(&mut core, Intent::Dismiss);
    assert!(!core.dialog_open());

    let (listing, _) = sent_once(&intent(&mut core, Intent::AddSteamGame));
    answer(&mut core, listing, DaemonResponse::SteamApps(vec![]));
    assert_eq!(
        add_steam_game_dialog(&core).status.as_deref(),
        Some("No installed Steam games were found in your Steam libraries.")
    );
    intent(&mut core, Intent::Dismiss);

    let (listing, _) = sent_once(&intent(&mut core, Intent::AddSteamGame));
    answer(
        &mut core,
        listing,
        DaemonResponse::SteamApps(vec![SteamApp {
            app_id: 1145360,
            name: "Hades".into(),
        }]),
    );
    let (adding, _) = sent_once(&intent(&mut core, Intent::AddSteamApp(1145360)));
    answer(
        &mut core,
        adding,
        refused("Steam app 1145360 isn't installed in your Steam libraries."),
    );
    let dialog = add_steam_game_dialog(&core);
    assert_eq!(dialog.status, None);
    assert_eq!(
        dialog.error.as_deref(),
        Some("Hades not added: Steam app 1145360 isn't installed in your Steam libraries.")
    );
    assert_eq!(dialog.apps[0].unavailable, None);
}

#[test]
fn without_tuning_access_games_are_read_only_with_the_reason() {
    let mut core = connected_with(false, |_| {});

    let section = section(&core);
    assert_eq!(
        section.read_only.as_deref(),
        Some("Needs tuning access · bob isn't in group wheel")
    );
    assert_eq!(section.games.len(), 3);
    assert_eq!(row(&core, "Dota 2").links[0].linked, "Quiet");

    for change in [
        Intent::SetAutomaticSwitching(true),
        Intent::LinkGame {
            game: DOTA,
            gpu_id: GPU.into(),
            profile_id: None,
        },
        Intent::RemoveGame(DOTA),
        Intent::AddSteamGame,
        Intent::AddProgram,
    ] {
        assert_eq!(
            requests(&intent(&mut core, change.clone())),
            [],
            "{change:?}"
        );
    }
    assert!(!core.dialog_open());

    let reason = Some("Needs tuning access · bob isn't in group wheel".to_owned());
    assert_eq!(
        palette_titles(&mut core, "turn automatic switching"),
        [("Turn automatic switching on".to_owned(), reason.clone())]
    );
    assert_eq!(
        palette_titles(&mut core, "add steam game"),
        [("Add Steam game…".to_owned(), reason.clone())]
    );
    assert_eq!(
        palette_titles(&mut core, "add program"),
        [("Add program…".to_owned(), reason)]
    );
}

#[test]
fn without_a_session_games_need_prismd_and_show_none() {
    for add in [Intent::AddSteamGame, Intent::AddProgram] {
        let mut core = connected();
        intent(&mut core, add.clone());
        core.handle(Input::PrismdNotConnected(SessionFailure::Unavailable {
            message: "gone".into(),
        }));
        // Nothing it lists can be added any more.
        assert!(!core.dialog_open(), "{add:?}");
    }

    let mut core = connected();
    core.handle(Input::PrismdNotConnected(SessionFailure::Unavailable {
        message: "gone".into(),
    }));

    let section = section(&core);
    assert_eq!(section.games, []);
    // Whether switching is on, and which games there are, is unknown without prismd.
    assert_eq!(section.automatic_switching, None);
    assert_eq!(
        section.empty,
        "Your games show once Prism is connected to prismd."
    );
    assert_eq!(
        section.read_only.as_deref(),
        Some("Needs tuning access · not running")
    );
}

#[test]
fn recognition_problems_are_explained_with_their_reason() {
    let unavailable = |reason: &str| RunningGamesState::Unavailable {
        reason: reason.into(),
    };
    let mut core = connected_with(true, |snapshot| {
        snapshot.running_games = unavailable("The first process scan has not completed.");
    });
    assert_eq!(
        section(&core).recognition_problem.as_deref(),
        Some(
            "Prism can't recognize running games, so automatic switching can't follow them: The \
             first process scan has not completed."
        )
    );

    push(
        &mut core,
        DaemonEvent::RunningGamesChanged(RunningGamesState::Available { games: vec![] }),
    );
    assert_eq!(section(&core).recognition_problem, None);

    push(
        &mut core,
        DaemonEvent::RunningGamesChanged(unavailable(
            "A complete host process scan could not be read.",
        )),
    );
    assert!(
        section(&core)
            .recognition_problem
            .unwrap()
            .ends_with("A complete host process scan could not be read.")
    );
}

#[test]
fn the_palette_turns_automatic_switching_on_or_off_and_adds_a_steam_game_or_a_program() {
    let mut core = connected();
    intent(&mut core, Intent::GoTo(Section::Settings));

    assert_eq!(
        palette_titles(&mut core, "turn automatic switching"),
        [("Turn automatic switching on".to_owned(), None)]
    );
    intent(&mut core, Intent::OpenPalette);
    intent(
        &mut core,
        Intent::FilterPalette("turn automatic switching".into()),
    );
    assert_eq!(
        requests(&intent(&mut core, Intent::RunHighlighted)),
        [DaemonRequest::SetAutomaticSwitching(
            SetAutomaticSwitchingRequest { enabled: true }
        )]
    );

    let mut on = games();
    on.automatic_switching = true;
    push(&mut core, DaemonEvent::GamesChanged(on));
    assert_eq!(
        palette_titles(&mut core, "turn automatic switching"),
        [("Turn automatic switching off".to_owned(), None)]
    );

    intent(&mut core, Intent::OpenPalette);
    intent(&mut core, Intent::FilterPalette("add steam game".into()));
    let effects = intent(&mut core, Intent::RunHighlighted);
    assert_eq!(requests(&effects), [DaemonRequest::ListSteamApps]);
    // It shows Games under the dialog, where the game it adds appears.
    assert_eq!(core.content(), prism_ui::Content::Section(Section::Games));
    assert!(matches!(core.dialog(), Some(Dialog::AddSteamGame(_))));

    intent(&mut core, Intent::Dismiss);
    intent(&mut core, Intent::GoTo(Section::Settings));
    intent(&mut core, Intent::OpenPalette);
    intent(&mut core, Intent::FilterPalette("add program".into()));
    let effects = intent(&mut core, Intent::RunHighlighted);
    assert_eq!(requests(&effects), [DaemonRequest::ListRunningPrograms]);
    assert_eq!(core.content(), prism_ui::Content::Section(Section::Games));
    assert!(matches!(core.dialog(), Some(Dialog::AddProgram(_))));
}

#[test]
fn add_program_lists_the_callers_running_programs_and_adds_the_one_chosen() {
    let mut core = connected();

    let (listing, request) = sent_once(&intent(&mut core, Intent::AddProgram));
    assert_eq!(request, DaemonRequest::ListRunningPrograms);
    let dialog = add_program_dialog(&core);
    assert_eq!(dialog.title, "Add program");
    assert_eq!(
        dialog.status.as_deref(),
        Some("Reading your running programs…")
    );
    assert_eq!(dialog.programs, []);
    assert!(dialog.can_browse);

    answer(
        &mut core,
        listing,
        DaemonResponse::RunningPrograms(vec![
            program(
                4100,
                "Balatro.exe",
                Some("/games/Balatro/Balatro.exe"),
                None,
            ),
            // Another process of the same program.
            program(
                4101,
                "Balatro.exe",
                Some("/games/Balatro/Balatro.exe"),
                None,
            ),
            program(
                2001,
                "factorio",
                Some("/opt/games/factorio/bin/factorio"),
                None,
            ),
            program(3000, "game.exe", None, Some(UNRECOVERED)),
            program(3100, "python3", Some("/usr/bin/python3"), Some(SCRIPTS)),
        ]),
    );
    let dialog = add_program_dialog(&core);
    assert_eq!(dialog.status, None);
    let programs: Vec<_> = dialog
        .programs
        .iter()
        .map(|program| {
            (
                program.name.as_str(),
                program.detail.as_str(),
                program.unavailable.as_deref(),
                program.warning.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        programs,
        [
            ("Balatro.exe", "/games/Balatro/Balatro.exe", None, None),
            (
                "factorio",
                "/opt/games/factorio/bin/factorio",
                Some("Already added"),
                None
            ),
            (
                "game.exe",
                "Host location unknown",
                Some("Location unknown"),
                Some(UNRECOVERED)
            ),
            ("python3", "/usr/bin/python3", None, Some(SCRIPTS)),
        ]
    );

    intent(&mut core, Intent::FilterPrograms("blt".into()));
    let dialog = add_program_dialog(&core);
    assert_eq!(dialog.filter, "blt");
    let names: Vec<_> = dialog
        .programs
        .iter()
        .map(|program| program.name.as_str())
        .collect();
    assert_eq!(names, ["Balatro.exe"]);

    // One that's already added, or that isn't listed, can't be added from the list.
    for path in ["/opt/games/factorio/bin/factorio", "/usr/bin/bash"] {
        assert_eq!(
            requests(&intent(&mut core, Intent::AddRunningProgram(path.into()))),
            []
        );
    }

    let (adding, request) = sent_once(&intent(
        &mut core,
        Intent::AddRunningProgram("/games/Balatro/Balatro.exe".into()),
    ));
    assert_eq!(
        request,
        DaemonRequest::AddProgram(AddProgramRequest {
            path: "/games/Balatro/Balatro.exe".into()
        })
    );
    let dialog = add_program_dialog(&core);
    assert_eq!(dialog.status.as_deref(), Some("Adding Balatro.exe…"));
    assert!(!dialog.can_browse);
    // Only one at a time.
    assert_eq!(
        requests(&intent(
            &mut core,
            Intent::AddRunningProgram("/usr/bin/python3".into())
        )),
        []
    );
    assert_eq!(intent(&mut core, Intent::BrowseForProgram), []);

    answer(&mut core, adding, DaemonResponse::Games(games()));
    assert!(!core.dialog_open());
}

#[test]
fn browsing_adds_the_program_file_at_the_host_location_chosen() {
    let mut core = connected();
    let (listing, _) = sent_once(&intent(&mut core, Intent::AddProgram));
    answer(&mut core, listing, DaemonResponse::RunningPrograms(vec![]));
    assert_eq!(
        add_program_dialog(&core).status.as_deref(),
        Some("None of your programs are running. Browse for the program file instead.")
    );

    assert_eq!(
        intent(&mut core, Intent::BrowseForProgram),
        [Effect::ChooseProgramFile]
    );
    let dialog = add_program_dialog(&core);
    assert!(!dialog.can_browse);
    assert_eq!(
        dialog.status.as_deref(),
        Some("Choose the program file in the file chooser…")
    );
    // One file chooser at a time.
    assert_eq!(intent(&mut core, Intent::BrowseForProgram), []);

    // Canceling it changes nothing.
    assert_eq!(
        core.handle(Input::ProgramFileChosen(ProgramFile::Canceled)),
        []
    );
    let dialog = add_program_dialog(&core);
    assert!(dialog.can_browse);
    assert_eq!(dialog.error, None);

    intent(&mut core, Intent::BrowseForProgram);
    let (adding, request) = sent_once(&core.handle(Input::ProgramFileChosen(ProgramFile::Chosen(
        "/home/alice/Games/Celeste/Celeste".into(),
    ))));
    assert_eq!(
        request,
        DaemonRequest::AddProgram(AddProgramRequest {
            path: "/home/alice/Games/Celeste/Celeste".into()
        })
    );
    assert_eq!(
        add_program_dialog(&core).status.as_deref(),
        Some("Adding Celeste…")
    );
    answer(&mut core, adding, DaemonResponse::Games(games()));
    assert!(!core.dialog_open());

    // A file chosen once Add program is closed adds nothing.
    intent(&mut core, Intent::AddProgram);
    intent(&mut core, Intent::BrowseForProgram);
    intent(&mut core, Intent::Dismiss);
    assert_eq!(
        core.handle(Input::ProgramFileChosen(ProgramFile::Chosen(
            "/games/late".into()
        ))),
        []
    );
}

#[test]
fn reopened_add_program_waits_for_the_file_chooser_still_open() {
    let mut core = connected();
    intent(&mut core, Intent::AddProgram);
    intent(&mut core, Intent::BrowseForProgram);
    intent(&mut core, Intent::Dismiss);

    // The file chooser opened before is still open, so another doesn't open over it.
    intent(&mut core, Intent::AddProgram);
    let dialog = add_program_dialog(&core);
    assert!(!dialog.can_browse);
    assert_eq!(
        dialog.status.as_deref(),
        Some("Choose the program file in the file chooser…")
    );
    assert_eq!(intent(&mut core, Intent::BrowseForProgram), []);

    // What it chooses is what Add program waits for.
    let (_, request) = sent_once(&core.handle(Input::ProgramFileChosen(ProgramFile::Chosen(
        "/games/Celeste".into(),
    ))));
    assert_eq!(
        request,
        DaemonRequest::AddProgram(AddProgramRequest {
            path: "/games/Celeste".into()
        })
    );
}

#[test]
fn add_program_says_why_listing_choosing_or_adding_failed() {
    let mut core = connected();
    let (listing, _) = sent_once(&intent(&mut core, Intent::AddProgram));
    answer(
        &mut core,
        listing,
        refused("Running programs could not be listed: denied"),
    );
    let dialog = add_program_dialog(&core);
    assert_eq!(
        dialog.status.as_deref(),
        Some("Running programs could not be listed: denied")
    );
    // Browsing still works.
    assert!(dialog.can_browse);

    // A file the document portal shared without its host location isn't sent to prismd.
    intent(&mut core, Intent::BrowseForProgram);
    let missing = "Prism couldn't find where game.exe is on the host: the document portal \
                   doesn't know it.";
    assert_eq!(
        core.handle(Input::ProgramFileChosen(ProgramFile::Failed(
            missing.into()
        ))),
        []
    );
    let dialog = add_program_dialog(&core);
    assert_eq!(dialog.error.as_deref(), Some(missing));
    assert!(dialog.can_browse);

    // prismd takes only an absolute host location.
    intent(&mut core, Intent::BrowseForProgram);
    let (adding, _) = sent_once(&core.handle(Input::ProgramFileChosen(ProgramFile::Chosen(
        "games/Celeste".into(),
    ))));
    // Trying again clears why the last one failed.
    assert_eq!(add_program_dialog(&core).error, None);
    answer(
        &mut core,
        adding,
        refused("Choose an absolute host program location."),
    );
    let dialog = add_program_dialog(&core);
    assert_eq!(
        dialog.status.as_deref(),
        Some("Running programs could not be listed: denied")
    );
    assert_eq!(
        dialog.error.as_deref(),
        Some("Celeste not added: Choose an absolute host program location.")
    );
    assert!(dialog.can_browse);
}
