//! Complete host process scans. An incomplete scan never means that a game ended.
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use super::DaemonCore;
use crate::protocol::{DaemonEvent, GameId, RunningGame, RunningGamesState};

type GamesByUser = BTreeMap<u32, Vec<RunningGame>>;

pub(super) struct Recognition(Result<GamesByUser, String>);

impl Default for Recognition {
    fn default() -> Self {
        Self(Err("The first process scan has not completed.".into()))
    }
}

impl Recognition {
    pub fn for_uid(&self, uid: u32) -> RunningGamesState {
        match &self.0 {
            Ok(users) => RunningGamesState::Available {
                games: users.get(&uid).cloned().unwrap_or_default(),
            },
            Err(reason) => RunningGamesState::Unavailable {
                reason: reason.clone(),
            },
        }
    }
}

impl DaemonCore {
    pub(super) fn start_recognition(self: &std::sync::Arc<Self>) -> io::Result<()> {
        let core = std::sync::Arc::downgrade(self);
        std::thread::Builder::new()
            .name("prismd-recognition".into())
            .spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_millis(1500));
                let Some(core) = core.upgrade() else {
                    break;
                };
                if core.stopping.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
                core.scan_running_games();
            })?;
        Ok(())
    }

    /// Runs one complete scan. The host adapter schedules this every 1–2 seconds; fixture
    /// hosts drive the same operation explicitly, without reading the real /proc.
    pub fn scan_running_games(&self) {
        let mut current = self.recognition.lock().unwrap();
        let next = Recognition(scan(&self.dependencies.host_root).map_err(|_| {
            "A complete host process scan could not be read. Recognition will retry automatically.".into()
        }));
        self.tuning_store.publish_for_users(|uid| {
            let state = next.for_uid(uid);
            (state != current.for_uid(uid)).then_some(DaemonEvent::RunningGamesChanged(state))
        });
        *current = next;
    }
}

fn scan(root: &Path) -> io::Result<GamesByUser> {
    let mut games: BTreeMap<(u32, GameId), RunningGame> = BTreeMap::new();
    for entry in fs::read_dir(root.join("proc"))? {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let path = entry.path();
        // A pid may be reused between reads. Never combine an old environment with a
        // different process's owner and start time.
        let started = start_time(&fs::read_to_string(path.join("stat"))?, pid)?;
        let env = fs::read(path.join("environ"))?;
        let primary = env
            .split(|byte| *byte == 0)
            .find_map(|entry| entry.strip_prefix(b"SteamAppId=").and_then(app_id));
        let recognized_app = match primary {
            Some(id) => Some(id),
            None => reaper_app(&fs::read(path.join("cmdline"))?),
        };
        let Some(app_id) = recognized_app else {
            continue;
        };
        let status = fs::read_to_string(path.join("status"))?;
        let uid = status
            .lines()
            .find_map(|line| {
                line.strip_prefix("Uid:")
                    .and_then(|uids| uids.split_whitespace().next())
                    .and_then(|uid| uid.parse::<u32>().ok())
            })
            .ok_or_else(invalid_process)?;
        let stat = fs::read_to_string(path.join("stat"))?;
        let start_time_ticks = start_time(&stat, pid)?;
        if started != start_time_ticks {
            return Err(invalid_process());
        }
        let game = GameId::Steam { app_id };
        let recognized = RunningGame {
            game: game.clone(),
            pid,
            start_time_ticks,
        };
        let earliest = games.entry((uid, game)).or_insert(recognized.clone());
        if (start_time_ticks, pid) < (earliest.start_time_ticks, earliest.pid) {
            *earliest = recognized;
        }
    }
    let mut users: GamesByUser = BTreeMap::new();
    for ((uid, _), game) in games {
        users.entry(uid).or_default().push(game);
    }
    Ok(users)
}

fn app_id(bytes: &[u8]) -> Option<u32> {
    std::str::from_utf8(bytes)
        .ok()?
        .parse()
        .ok()
        .filter(|id| *id > 0)
}

fn start_time(stat: &str, pid: u32) -> io::Result<u64> {
    if stat
        .split_once(" (")
        .and_then(|(id, _)| id.parse::<u32>().ok())
        != Some(pid)
    {
        return Err(invalid_process());
    }
    // comm is parenthesized and may itself contain spaces and parentheses.
    stat.rsplit_once(')')
        .and_then(|(_, fields)| fields.split_whitespace().nth(19))
        .and_then(|start| start.parse().ok())
        .ok_or_else(invalid_process)
}

fn invalid_process() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Incomplete process details")
}

/// Only the reaper's own SteamLaunch header is evidence; an arbitrary argument is not.
fn reaper_app(cmdline: &[u8]) -> Option<u32> {
    let args: Vec<_> = cmdline.split(|byte| *byte == 0).collect();
    if args.first()?.rsplit(|byte| *byte == b'/').next()? != b"reaper"
        || args.get(1)? != &b"SteamLaunch".as_slice()
    {
        return None;
    }
    let header: Vec<_> = args
        .iter()
        .skip(2)
        .take_while(|arg| **arg != b"--")
        .collect();
    if header.iter().any(|arg| ***arg == *b"Install=1") {
        return None;
    }
    header
        .into_iter()
        .find_map(|arg| arg.strip_prefix(b"AppId=").and_then(app_id))
}
