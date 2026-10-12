//! The Games section: the caller's games as prismd pushes them, each linked to at most one of the
//! caller's profiles per GPU, the automatic switching toggle, removing a game, adding a Steam game
//! from the caller's Steam libraries, which prismd recognizes natively or through Proton, and
//! adding a program, running or chosen in the file chooser, which prismd recognizes by its host
//! location. Games change only as prismd pushes them; answers only say how a change went (ADR
//! 0010).

use prism_protocol::{
    AddProgramRequest, AddSteamGameRequest, DaemonRequest, DaemonResponse, GameEntry, GameId,
    GameRequest, RunningGamesState, RunningProgram, SetAutomaticSwitchingRequest,
    SetGameProfileRequest,
};

use crate::core::{CommandId, Core, Effect, Section, Selection};
use crate::drafts::DialogState;
use crate::palette::{Command, fuzzy_match, on_off};
use crate::view::{
    AddProgram, AddSteamGame, GameLink, GameRow, GamesSection, LinkChoice, ProgramChoice,
    SteamAppChoice, gpu_label,
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
    /// Listing the caller's Steam apps or running programs for Add Steam game or Add program.
    List,
    /// Adding the Steam app or program named.
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
            Self::List => why.to_owned(),
            Self::Add(game) => format!("{game} not added: {why}"),
        }
    }
}

/// The caller's Steam apps or running programs, as Add Steam game or Add program lists them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Listing<T> {
    /// prismd hasn't answered the listing sent as this command yet.
    Reading(CommandId),
    Listed(Vec<T>),
    /// Why prismd couldn't list them.
    Failed(String),
}

impl<T: Clone> Listing<T> {
    /// Takes in prismd's answer to the listing sent as `command`, which `listed` reads the list
    /// from; says whether it was that one.
    fn answered(
        &mut self,
        command: CommandId,
        response: &DaemonResponse,
        refusal: Option<&str>,
        listed: impl FnOnce(&DaemonResponse) -> Option<&Vec<T>>,
    ) -> bool {
        if !matches!(self, Self::Reading(id) if *id == command) {
            return false;
        }
        *self = match (listed(response), refusal) {
            (Some(list), None) => Self::Listed(list.clone()),
            (_, why) => Self::Failed(why.unwrap_or_default().to_owned()),
        };
        true
    }
}

/// Takes in prismd's answer to adding a game, if `command` added the one `adding`: says whether
/// it was added, or why not in `error`.
fn adding_answered<K>(
    adding: &mut Option<(CommandId, K)>,
    error: &mut Option<String>,
    command: CommandId,
    sent: &GameCommand,
    refusal: Option<String>,
) -> Option<bool> {
    if !adding.as_ref().is_some_and(|(id, _)| *id == command) {
        return None;
    }
    *adding = None;
    *error = refusal.map(|why| sent.refused(&why));
    Some(error.is_none())
}

/// The program file the user chose in the file chooser, as its host location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgramFile {
    /// The host location of the file chosen, recovered from the document portal when the file
    /// chooser shared it through one.
    Chosen(String),
    /// The user closed the file chooser without choosing.
    Canceled,
    /// Why there's no host location to add, such as the document portal not knowing it.
    Failed(String),
}

/// A program's name, as prismd names it: the last part of its location.
fn program_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
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
                "No games yet. Add a Steam game or a program to link it to your profiles."
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
            Command::SetAutomaticSwitching(_) | Command::AddSteamGame | Command::AddProgram => {
                Some(self.needs_tuning_access())
            }
            _ => None,
        }
    }

    /// The palette's game commands: turning automatic switching on or off, as it isn't now, and
    /// adding a Steam game or a program.
    pub(crate) fn game_palette_commands(&self) -> [(Command, String); 3] {
        let enable = !self.games.automatic_switching;
        [
            (
                Command::SetAutomaticSwitching(enable),
                format!("Turn automatic switching {}", on_off(enable)),
            ),
            (Command::AddSteamGame, "Add Steam game…".into()),
            (Command::AddProgram, "Add program…".into()),
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
        self.select(Selection::Section(Section::Games));
        let (id, effects) =
            self.command_prismd_for_games(GameCommand::List, DaemonRequest::ListSteamApps);
        self.open_dialog(DialogState::AddSteamGame {
            listing: Listing::Reading(id),
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
            listing: Listing::Listed(apps),
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

    /// Shows Games and opens Add program over it, listing the caller's running programs.
    pub(crate) fn open_add_program(&mut self) -> Vec<Effect> {
        if self.needs_tuning_access().is_some() {
            return Vec::new();
        }
        self.select(Selection::Section(Section::Games));
        let (id, effects) =
            self.command_prismd_for_games(GameCommand::List, DaemonRequest::ListRunningPrograms);
        self.open_dialog(DialogState::AddProgram {
            listing: Listing::Reading(id),
            filter: String::new(),
            adding: None,
            error: None,
        });
        effects
    }

    pub(crate) fn filter_programs(&mut self, query: String) {
        if let Some(DialogState::AddProgram { filter, .. }) = &mut self.dialog {
            *filter = query;
        }
    }

    /// Adds the listed running program at this host location, unless it's added already, or
    /// another is being added or chosen in the file chooser.
    pub(crate) fn add_running_program(&mut self, path: String) -> Vec<Effect> {
        let Some(DialogState::AddProgram {
            listing: Listing::Listed(programs),
            adding: None,
            ..
        }) = &self.dialog
        else {
            return Vec::new();
        };
        if self.choosing_program_file
            || !programs
                .iter()
                .any(|program| program.path.as_ref() == Some(&path))
            || self.game(&GameId::Program { path: path.clone() }).is_some()
        {
            return Vec::new();
        }
        self.add_program(path)
    }

    /// Opens the file chooser for the program file to add, unless it's open already or a
    /// program is being added.
    pub(crate) fn browse_for_program(&mut self) -> Vec<Effect> {
        if self.needs_tuning_access().is_some() {
            return Vec::new();
        }
        if self.choosing_program_file {
            return Vec::new();
        }
        let Some(DialogState::AddProgram {
            adding: None,
            error,
            ..
        }) = &mut self.dialog
        else {
            return Vec::new();
        };
        *error = None;
        self.choosing_program_file = true;
        vec![Effect::ChooseProgramFile]
    }

    /// Takes in the file chooser's answer, adding the program file chosen, while Add program is
    /// open to wait for it.
    pub(crate) fn program_file_chosen(&mut self, file: ProgramFile) -> Vec<Effect> {
        self.choosing_program_file = false;
        let Some(DialogState::AddProgram {
            adding: None,
            error,
            ..
        }) = &mut self.dialog
        else {
            return Vec::new();
        };
        match file {
            ProgramFile::Chosen(path) => self.add_program(path),
            ProgramFile::Canceled => Vec::new(),
            ProgramFile::Failed(why) => {
                *error = Some(why);
                Vec::new()
            }
        }
    }

    /// Asks prismd to add the program at this host location, from Add program.
    fn add_program(&mut self, path: String) -> Vec<Effect> {
        let change = GameCommand::Add(program_name(&path).to_owned());
        let Some((id, effects)) = self.send_game_change(
            change,
            DaemonRequest::AddProgram(AddProgramRequest { path: path.clone() }),
        ) else {
            return Vec::new();
        };
        if let Some(DialogState::AddProgram { adding, error, .. }) = &mut self.dialog {
            *adding = Some((id, path));
            *error = None;
        }
        effects
    }

    /// Takes in prismd's answer to a game change, if `command` was one: Add Steam game's or Add
    /// program's listing, whether the game they add was added, or why a change was refused. Says
    /// whether it was one.
    pub(crate) fn game_answered(&mut self, command: CommandId, response: &DaemonResponse) -> bool {
        let Some(sent) = self.game_commands.remove(&command) else {
            return false;
        };
        let refusal = match response {
            DaemonResponse::Error(error) => Some(error.message.clone()),
            DaemonResponse::Games(_)
            | DaemonResponse::SteamApps(_)
            | DaemonResponse::RunningPrograms(_) => None,
            _ => Some("prismd answered with something else. Try again.".into()),
        };
        let added =
            match &mut self.dialog {
                Some(DialogState::AddSteamGame {
                    listing,
                    adding,
                    error,
                    ..
                }) => {
                    if listing.answered(command, response, refusal.as_deref(), |response| {
                        match response {
                            DaemonResponse::SteamApps(apps) => Some(apps),
                            _ => None,
                        }
                    }) {
                        return true;
                    }
                    adding_answered(adding, error, command, &sent, refusal.clone())
                }
                Some(DialogState::AddProgram {
                    listing,
                    adding,
                    error,
                    ..
                }) => {
                    if listing.answered(command, response, refusal.as_deref(), |response| {
                        match response {
                            DaemonResponse::RunningPrograms(programs) => Some(programs),
                            _ => None,
                        }
                    }) {
                        return true;
                    }
                    adding_answered(adding, error, command, &sent, refusal.clone())
                }
                _ => None,
            };
        match added {
            Some(true) => self.dialog = None,
            Some(false) => {}
            None => {
                if let (Some(why), false) = (refusal, matches!(sent, GameCommand::List)) {
                    self.games_outcome = Some(sent.refused(&why));
                }
            }
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
            Listing::Listed(apps) => apps
                .iter()
                .find(|app| app.app_id == app_id)
                .map(|app| format!("Adding {}…", app.name)),
            _ => None,
        });
        let (apps, status) = match listing {
            Listing::Reading(_) => (Vec::new(), Some("Reading your Steam libraries…".into())),
            Listing::Failed(why) => (Vec::new(), Some(why.clone())),
            Listing::Listed(apps) if apps.is_empty() => (
                Vec::new(),
                Some("No installed Steam games were found in your Steam libraries.".into()),
            ),
            Listing::Listed(apps) => (
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

    /// Add program as shown, if `dialog` is it.
    pub(crate) fn add_program_dialog(&self, dialog: &DialogState) -> Option<AddProgram> {
        let DialogState::AddProgram {
            listing,
            filter,
            adding,
            error,
        } = dialog
        else {
            return None;
        };
        // Such as "Adding Celeste…" while a program is being added, which keeps the others from
        // being added, as the file chooser does while it's open.
        let busy = match (adding, self.choosing_program_file) {
            (Some((_, path)), _) => Some(format!("Adding {}…", program_name(path))),
            (None, true) => Some("Choose the program file in the file chooser…".to_owned()),
            (None, false) => None,
        };
        let (programs, status) = match listing {
            Listing::Reading(_) => (Vec::new(), Some("Reading your running programs…".into())),
            Listing::Failed(why) => (Vec::new(), Some(why.clone())),
            Listing::Listed(programs) if programs.is_empty() => (
                Vec::new(),
                Some(
                    "None of your programs are running. Browse for the program file instead."
                        .into(),
                ),
            ),
            Listing::Listed(programs) => {
                (self.program_choices(programs, filter, busy.as_ref()), None)
            }
        };
        Some(AddProgram {
            title: "Add program".into(),
            message: "Prism recognizes a program by its location on the host. Choose one that's \
                      running, or browse for its program file."
                .into(),
            filter: filter.clone(),
            programs,
            status: busy.or(status),
            can_browse: adding.is_none() && !self.choosing_program_file,
            error: error.clone(),
        })
    }

    /// The running programs that match `filter`, each process of the same program once, with why
    /// each can't be added now, when it can't: added already, at no known location, or `busy`.
    fn program_choices(
        &self,
        programs: &[RunningProgram],
        filter: &str,
        busy: Option<&String>,
    ) -> Vec<ProgramChoice> {
        let mut choices: Vec<ProgramChoice> = Vec::new();
        for program in programs {
            if !fuzzy_match(filter, &program.name)
                || choices
                    .iter()
                    .any(|choice| choice.path == program.path && choice.name == program.name)
            {
                continue;
            }
            let added = program
                .path
                .as_ref()
                .is_some_and(|path| self.game(&GameId::Program { path: path.clone() }).is_some());
            choices.push(ProgramChoice {
                path: program.path.clone(),
                name: program.name.clone(),
                detail: program
                    .path
                    .clone()
                    .unwrap_or_else(|| "Host location unknown".into()),
                warning: program.recognition_warning.clone(),
                unavailable: if added {
                    Some("Already added".into())
                } else if program.path.is_none() {
                    Some("Location unknown".into())
                } else {
                    busy.cloned()
                },
            });
        }
        choices
    }

    /// The session ended: games and their changes go with it, and nothing listed can be added.
    pub(crate) fn games_lost_session(&mut self) {
        self.games = Default::default();
        self.running_games = None;
        self.game_commands.clear();
        self.games_outcome = None;
        if matches!(
            self.dialog,
            Some(DialogState::AddSteamGame { .. } | DialogState::AddProgram { .. })
        ) {
            self.dialog = None;
        }
    }
}
