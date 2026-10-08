//! Each user's game associations and automatic switching opt-in in the tuning store, with the
//! installed Steam apps and manually identified programs they are linked from.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::session::SessionPeer;
use super::steam_library::installed_steam_apps;
use super::tuning_store::TuningDocument;
use super::DaemonCore;
use crate::gpu::{GpuCommandError, GpuCommandErrorCode};
use crate::protocol::{
    DaemonResponse, GameEntry, GameId, GamesState, SetGameProfileRequest, SteamApp,
};

/// A user's link from a game to at most one of their profiles per GPU.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GameAssociation {
    pub game: GameId,
    /// The name when the game was added, shown while it isn't installed.
    pub name: String,
    /// The linked profile id per GPU UUID.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, String>,
}

impl DaemonCore {
    /// The caller's associations, with whether each game is installed now.
    pub(super) fn games(&self, peer: &SessionPeer) -> Result<DaemonResponse, GpuCommandError> {
        let installed = self.steam_apps(peer)?;
        Ok(DaemonResponse::Games(self.tuning_store.read(
            |document| document.games_state(peer, &installed, &self.dependencies.host_root),
        )?))
    }

    pub(super) fn steam_apps(&self, peer: &SessionPeer) -> Result<Vec<SteamApp>, GpuCommandError> {
        installed_steam_apps(&self.dependencies.host_root, peer)
    }

    /// Adds an installed Steam app, named from its manifest. Adding it again keeps its links.
    pub(super) fn add_steam_game(
        &self,
        peer: &SessionPeer,
        app_id: u32,
    ) -> Result<DaemonResponse, GpuCommandError> {
        let app = self
            .steam_apps(peer)?
            .into_iter()
            .find(|app| app.app_id == app_id)
            .ok_or_else(|| {
                GpuCommandError::new(
                    GpuCommandErrorCode::NotFound,
                    format!("Steam app {app_id} isn't installed in your Steam libraries."),
                )
            })?;
        self.update_games(peer, |user_games| {
            if !user_games
                .iter()
                .any(|game| game.game == GameId::Steam { app_id })
            {
                user_games.push(GameAssociation {
                    game: GameId::Steam { app_id },
                    name: app.name,
                    profiles: BTreeMap::new(),
                });
            }
            Ok(())
        })
    }

    pub(super) fn add_program(
        &self,
        peer: &SessionPeer,
        path: String,
    ) -> Result<DaemonResponse, GpuCommandError> {
        let location = std::path::Path::new(&path);
        if !location.is_absolute()
            || path.contains('\0')
            || location
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(GpuCommandError::new(
                GpuCommandErrorCode::InvalidArgument,
                "Choose an absolute host program location.",
            ));
        }
        if !self
            .dependencies
            .host_root
            .join(location.strip_prefix("/").unwrap())
            .is_file()
        {
            return Err(GpuCommandError::new(
                GpuCommandErrorCode::NotFound,
                "Program not found at this host location.",
            ));
        }
        let name = location.file_name().unwrap().to_string_lossy().into_owned();
        self.update_games(peer, |games| {
            let game = GameId::Program { path };
            if !games.iter().any(|entry| entry.game == game) {
                games.push(GameAssociation {
                    game,
                    name,
                    profiles: BTreeMap::new(),
                });
            }
            Ok(())
        })
    }

    pub(super) fn set_game_profile(
        &self,
        peer: &SessionPeer,
        request: SetGameProfileRequest,
    ) -> Result<DaemonResponse, GpuCommandError> {
        self.update_document(peer, |document| {
            if let Some(profile_id) = &request.profile_id {
                let profile = document.profile_of(peer, profile_id)?;
                if profile.gpu_id != request.gpu_id {
                    return Err(GpuCommandError::new(
                        GpuCommandErrorCode::InvalidArgument,
                        format!("The profile \"{}\" belongs to another GPU.", profile.name),
                    ));
                }
            }
            let association = document
                .user_mut(peer.uid, &peer.username)
                .games
                .iter_mut()
                .find(|game| game.game == request.game)
                .ok_or_else(|| {
                    GpuCommandError::new(
                        GpuCommandErrorCode::NotFound,
                        "This game is no longer on your Games page.",
                    )
                })?;
            match request.profile_id {
                Some(profile_id) => association.profiles.insert(request.gpu_id, profile_id),
                None => association.profiles.remove(&request.gpu_id),
            };
            Ok(())
        })
    }

    pub(super) fn remove_game(
        &self,
        peer: &SessionPeer,
        game: GameId,
    ) -> Result<DaemonResponse, GpuCommandError> {
        self.update_games(peer, |user_games| {
            user_games.retain(|association| association.game != game);
            Ok(())
        })
    }

    pub(super) fn set_automatic_switching(
        &self,
        peer: &SessionPeer,
        enabled: bool,
    ) -> Result<DaemonResponse, GpuCommandError> {
        self.update_document(peer, |document| {
            document
                .user_mut(peer.uid, &peer.username)
                .automatic_switching = enabled;
            Ok(())
        })
    }

    fn update_games(
        &self,
        peer: &SessionPeer,
        change: impl FnOnce(&mut Vec<GameAssociation>) -> Result<(), GpuCommandError>,
    ) -> Result<DaemonResponse, GpuCommandError> {
        self.update_document(peer, |document| {
            change(&mut document.user_mut(peer.uid, &peer.username).games)
        })
    }

    /// Changes the document and responds with the caller's resulting games.
    fn update_document(
        &self,
        peer: &SessionPeer,
        change: impl FnOnce(&mut TuningDocument) -> Result<(), GpuCommandError>,
    ) -> Result<DaemonResponse, GpuCommandError> {
        self.tuning_store.try_update(change)?;
        self.games(peer)
    }
}

impl TuningDocument {
    /// The user's associations by name, each marked installed when it's among the installed
    /// apps, which also give its current name.
    pub fn games_state(
        &self,
        peer: &SessionPeer,
        installed: &[SteamApp],
        host_root: &std::path::Path,
    ) -> GamesState {
        let user = self.user(peer.uid, &peer.username);
        let mut games: Vec<_> = user
            .map(|user| user.games.as_slice())
            .unwrap_or_default()
            .iter()
            .map(|association| {
                let app = match &association.game {
                    GameId::Steam { app_id } => installed.iter().find(|app| app.app_id == *app_id),
                    GameId::Program { .. } => None,
                };
                GameEntry {
                    game: association.game.clone(),
                    name: app.map_or(association.name.clone(), |app| app.name.clone()),
                    installed: match &association.game {
                        GameId::Steam { .. } => app.is_some(),
                        GameId::Program { path } => {
                            host_root.join(path.trim_start_matches('/')).is_file()
                        }
                    },
                    profiles: association.profiles.clone(),
                }
            })
            .collect();
        games.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.game.cmp(&b.game))
        });
        GamesState {
            automatic_switching: user.is_some_and(|user| user.automatic_switching),
            games,
        }
    }

    /// Removes every link to the profile.
    pub fn unlink_profile(&mut self, profile_id: &str) {
        for association in self.users.iter_mut().flat_map(|user| &mut user.games) {
            association.profiles.retain(|_, id| id != profile_id);
        }
    }
}
