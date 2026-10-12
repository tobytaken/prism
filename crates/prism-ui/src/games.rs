//! The Games section: the caller's games as prismd pushes them, each linked to at most one of the
//! caller's profiles per GPU, the automatic switching toggle, removing a game, and adding a Steam
//! game from the caller's Steam libraries, which prismd recognizes natively or through Proton.
//! Games change only as prismd pushes them; answers only say how a change went (ADR 0010).

use prism_protocol::{
    AddSteamGameRequest, DaemonRequest, DaemonResponse, GameEntry, GameId, GameRequest,
    RunningGamesState, SetAutomaticSwitchingRequest, SetGameProfileRequest, SteamApp,
};

use crate::core::{CommandId, Core, Effect, Section, Selection};
use crate::drafts::DialogState;
use crate::palette::{Command, fuzzy_match};
use crate::view::{
    AddSteamGame, GameLink, GameRow, GamesSection, LinkChoice, SteamAppChoice, gpu_label,
};

/// A game change sent to prismd, waiting for its answer.
#[derive(Debug, Clone)]
pub(crate) enum GameCommand {
    SetSwitching(bool),
    /// Linking the game, by name, to the profile named, or to none.
    Link {
        game: String,
        profile: Option<String>,
    },
    Remove(String),
    /// Listing the caller's Steam apps for Add Steam game.
    ListSteamApps,
    /// Adding the Steam app named.
    Add(String),
}

impl GameCommand {
    /// What the Games section says once prismd refused it, such as "Dota 2 not removed: …".
    fn refused(&self, why: &str) -> String {
        match self {
            Self::SetSwitching(enabled) => {
                format!("Automatic switching not turned {}: {why}", on_off(*enabled))
            }
            Self::Link {
                game,
                profile: Some(profile),
            } => format!("{game} not linked to {profile}: {why}"),
            Self::Link {
                game,
                profile: None,
            } => format!("{game} not unlinked: {why}"),
            Self::Remove(game) => format!("{game} not removed: {why}"),
            Self::ListSteamApps => why.to_owned(),
            Self::Add(game) => format!("{game} not added: {why}"),
        }
    }
}

/// "on" or "off", as turning automatic switching on or off says it.
fn on_off(enabled: bool) -> &'static str {
    if enabled { "on" } else { "off" }
}

/// The caller's Steam apps, as Add Steam game lists them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SteamListing {
    /// prismd hasn't answered the listing sent as this command yet.
    Reading(CommandId),
    Listed(Vec<SteamApp>),
    /// Why prismd couldn't list them.
    Failed(String),
}

impl Core {
    /// The Games section as shown.
    pub fn games(&self) -> GamesSection {
        let connected = self.connected();
        GamesSection {
            automatic_switching: connected.then_some(self.games.automatic_switching),
            read_only: self.needs_tuning_access(),
            recognition_problem: match &self.running_games {
                Some(RunningGamesState::Unavailable { reason }) => Some(format!(
                    "Prism can't recognize running games, so automatic switching can't follow \
                     them: {reason}"
                )),
                _ => None,
            },
            outcome: self.games_outcome.clone(),
            games: self
                .games
                .games
                .iter()
                .map(|game| self.game_row(game))
                .collect(),
            empty: if connected {
                "No games yet. Add a Steam game to link it to your profiles."
            } else {
                "Your games show once Prism is connected to prismd."
            }
            .into(),
        }
    }

    fn game_row(&self, game: &GameEntry) -> GameRow {
        let (identity, missing) = match &game.game {
            GameId::Steam { app_id } => (format!("Steam app {app_id}"), "Not installed"),
            GameId::Program { path } => (path.clone(), "Not found"),
        };
        GameRow {
            game: game.game.clone(),
            name: game.name.clone(),
            detail: if game.installed {
                identity
            } else {
                format!("{identity} · {missing}")
            },
            // A link for each GPU here; the links of a GPU that isn't here stay stored.
            links: self
                .gpus()
                .iter()
                .enumerate()
                .map(|(index, gpu)| {
                    let linked = game.profiles.get(&gpu.id);
                    let choices: Vec<_> = [LinkChoice {
                        profile_id: None,
                        name: "None".into(),
                        linked: linked.is_none(),
                    }]
                    .into_iter()
                    .chain(self.profiles_of(&gpu.id).map(|profile| LinkChoice {
                        profile_id: Some(profile.id.clone()),
                        name: profile.name.clone(),
                        linked: linked == Some(&profile.id),
                    }))
                    .collect();
                    GameLink {
                        gpu_id: gpu.id.clone(),
                        gpu: gpu_label(index, gpu),
                        linked: choices
                            .iter()
                            .find(|choice| choice.linked)
                            .map_or_else(|| "Unknown profile".into(), |choice| choice.name.clone()),
                        choices,
                    }
                })
                .collect(),
        }
    }

    /// The caller's game, by what identifies it.
    fn game(&self, game: &GameId) -> Option<&GameEntry> {
        self.games.games.iter().find(|entry| entry.game == *game)
    }

    /// Why a game command can't run now, when it can't; none when `command` isn't one.
    pub(crate) fn game_command_unavailable(&self, command: &Command) -> Option<Option<String>> {
        match command {
            Command::SetAutomaticSwitching(_) | Command::AddSteamGame => {
                Some(self.needs_tuning_access())
            }
            _ => None,
        }
    }

    /// The palette's game commands: turning automatic switching on or off, as it isn't now, and
    /// adding a Steam game.
    pub(crate) fn game_palette_commands(&self) -> [(Command, String); 2] {
        let enable = !self.games.automatic_switching;
        [
            (
                Command::SetAutomaticSwitching(enable),
                format!("Turn automatic switching {}", on_off(enable)),
            ),
            (Command::AddSteamGame, "Add Steam game…".into()),
        ]
    }

    /// Sends a game change, with tuning access, clearing why the last one was refused; returns the
    /// command its answer comes back as, or none without tuning access.
    fn send_game_change(
        &mut self,
        change: GameCommand,
        request: DaemonRequest,
    ) -> Option<(CommandId, Vec<Effect>)> {
        if self.needs_tuning_access().is_some() {
            return None;
        }
        self.games_outcome = None;
        Some(self.command_prismd_for_games(change, request))
    }

    /// Sends a request of the Games section to prismd, noting what it was for its answer.
    fn command_prismd_for_games(
        &mut self,
        change: GameCommand,
        request: DaemonRequest,
    ) -> (CommandId, Vec<Effect>) {
        let id = self.drafts.next_command();
        self.game_commands.insert(id, change);
        (id, vec![Effect::CommandPrismd { id, request }])
    }

    pub(crate) fn set_automatic_switching(&mut self, enabled: bool) -> Vec<Effect> {
        self.send_game_change(
            GameCommand::SetSwitching(enabled),
            DaemonRequest::SetAutomaticSwitching(SetAutomaticSwitchingRequest { enabled }),
        )
        .map(|(_, effects)| effects)
        .unwrap_or_default()
    }

    /// Links the game to the profile on the GPU, or to none, unless it's linked to it already.
    pub(crate) fn link_game(
        &mut self,
        game: GameId,
        gpu_id: String,
        profile_id: Option<String>,
    ) -> Vec<Effect> {
        let Some(entry) = self.game(&game) else {
            return Vec::new();
        };
        if entry.profiles.get(&gpu_id) == profile_id.as_ref() {
            return Vec::new();
        }
        let profile = match &profile_id {
            Some(id) => match self.profile(id) {
                Some(profile) => Some(profile.name.clone()),
                None => return Vec::new(),
            },
            None => None,
        };
        let change = GameCommand::Link {
            game: entry.name.clone(),
            profile,
        };
        self.send_game_change(
            change,
            DaemonRequest::SetGameProfile(SetGameProfileRequest {
                game,
                gpu_id,
                profile_id,
            }),
        )
        .map(|(_, effects)| effects)
        .unwrap_or_default()
    }

    pub(crate) fn remove_game(&mut self, game: GameId) -> Vec<Effect> {
        let Some(entry) = self.game(&game) else {
            return Vec::new();
        };
        let change = GameCommand::Remove(entry.name.clone());
        self.send_game_change(change, DaemonRequest::RemoveGame(GameRequest { game }))
            .map(|(_, effects)| effects)
            .unwrap_or_default()
    }

    /// Shows Games and opens Add Steam game over it, listing the caller's Steam apps.
    pub(crate) fn open_add_steam_game(&mut self) -> Vec<Effect> {
        if self.needs_tuning_access().is_some() {
            return Vec::new();
        }
        self.selection = Some(Selection::Section(Section::Games));
        let (id, effects) =
            self.command_prismd_for_games(GameCommand::ListSteamApps, DaemonRequest::ListSteamApps);
        self.open_dialog(DialogState::AddSteamGame {
            listing: SteamListing::Reading(id),
            filter: String::new(),
            adding: None,
            error: None,
        });
        effects
    }

    pub(crate) fn filter_steam_apps(&mut self, query: String) {
        if let Some(DialogState::AddSteamGame { filter, .. }) = &mut self.dialog {
            *filter = query;
        }
    }

    /// Adds the listed Steam app, unless it's added already or another is being added.
    pub(crate) fn add_steam_app(&mut self, app_id: u32) -> Vec<Effect> {
        let Some(DialogState::AddSteamGame {
            listing: SteamListing::Listed(apps),
            adding: None,
            ..
        }) = &self.dialog
        else {
            return Vec::new();
        };
        let Some(app) = apps.iter().find(|app| app.app_id == app_id) else {
            return Vec::new();
        };
        if self.game(&GameId::Steam { app_id }).is_some() {
            return Vec::new();
        }
        let name = app.name.clone();
        let Some((id, effects)) = self.send_game_change(
            GameCommand::Add(name),
            DaemonRequest::AddSteamGame(AddSteamGameRequest { app_id }),
        ) else {
            return Vec::new();
        };
        if let Some(DialogState::AddSteamGame { adding, error, .. }) = &mut self.dialog {
            *adding = Some((id, app_id));
            *error = None;
        }
        effects
    }

    /// Takes in prismd's answer to a game change, if `command` was one: Add Steam game's listing,
    /// or why a change was refused. Says whether it was one.
    pub(crate) fn game_answered(&mut self, command: CommandId, response: &DaemonResponse) -> bool {
        let Some(sent) = self.game_commands.remove(&command) else {
            return false;
        };
        let refusal = match response {
            DaemonResponse::Error(error) => Some(error.message.clone()),
            DaemonResponse::Games(_) | DaemonResponse::SteamApps(_) => None,
            _ => Some("prismd answered with something else. Try again.".into()),
        };
        match &mut self.dialog {
            Some(DialogState::AddSteamGame { listing, .. })
                if *listing == SteamListing::Reading(command) =>
            {
                *listing = match (response, refusal) {
                    (DaemonResponse::SteamApps(apps), None) => SteamListing::Listed(apps.clone()),
                    (_, why) => SteamListing::Failed(why.unwrap_or_default()),
                };
                return true;
            }
            Some(DialogState::AddSteamGame { adding, error, .. })
                if adding.is_some_and(|(id, _)| id == command) =>
            {
                *adding = None;
                match refusal {
                    Some(why) => *error = Some(sent.refused(&why)),
                    None => self.dialog = None,
                }
                return true;
            }
            _ => {}
        }
        if let (Some(why), false) = (refusal, matches!(sent, GameCommand::ListSteamApps)) {
            self.games_outcome = Some(sent.refused(&why));
        }
        true
    }

    /// Add Steam game as shown, if `dialog` is it.
    pub(crate) fn add_steam_game_dialog(&self, dialog: &DialogState) -> Option<AddSteamGame> {
        let DialogState::AddSteamGame {
            listing,
            filter,
            adding,
            error,
        } = dialog
        else {
            return None;
        };
        // Such as "Adding Hades…", while an app is being added.
        let adding_status = adding.and_then(|(_, app_id)| match listing {
            SteamListing::Listed(apps) => apps
                .iter()
                .find(|app| app.app_id == app_id)
                .map(|app| format!("Adding {}…", app.name)),
            _ => None,
        });
        let (apps, status) = match listing {
            SteamListing::Reading(_) => (Vec::new(), Some("Reading your Steam libraries…".into())),
            SteamListing::Failed(why) => (Vec::new(), Some(why.clone())),
            SteamListing::Listed(apps) if apps.is_empty() => (
                Vec::new(),
                Some("No installed Steam games were found in your Steam libraries.".into()),
            ),
            SteamListing::Listed(apps) => (
                apps.iter()
                    .filter(|app| fuzzy_match(filter, &app.name))
                    .map(|app| SteamAppChoice {
                        app_id: app.app_id,
                        name: app.name.clone(),
                        unavailable: if self.game(&GameId::Steam { app_id: app.app_id }).is_some() {
                            Some("Already added".into())
                        } else {
                            adding_status.clone()
                        },
                    })
                    .collect(),
                adding_status.clone(),
            ),
        };
        Some(AddSteamGame {
            title: "Add Steam game".into(),
            message: "Installed games from your Steam libraries. Prism recognizes a Steam game \
                      whether it runs natively or through Proton."
                .into(),
            filter: filter.clone(),
            apps,
            status,
            error: error.clone(),
        })
    }

    /// The session ended: games and their changes go with it, and nothing listed can be added.
    pub(crate) fn games_lost_session(&mut self) {
        self.games = Default::default();
        self.running_games = None;
        self.game_commands.clear();
        self.games_outcome = None;
        if matches!(self.dialog, Some(DialogState::AddSteamGame { .. })) {
            self.dialog = None;
        }
    }
}
