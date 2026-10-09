//! Complete host process scans. An incomplete scan never means that a game ended.
use super::programs::{candidate_executable, is_wine_loader, mapped_images, FileIdentity};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use super::DaemonCore;
use crate::protocol::{DaemonEvent, GameId, RunningGame, RunningGamesState};

pub(super) type GamesByUser = BTreeMap<u32, Vec<RunningGame>>;

pub(super) struct Recognition(Result<GamesByUser, String>);

impl Default for Recognition {
    fn default() -> Self {
        Self(Err("The first process scan has not completed.".into()))
    }
}

impl Recognition {
    /// Each user's running games, or why the latest scan was incomplete.
    pub fn running(&self) -> &Result<GamesByUser, String> {
        &self.0
    }

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

    /// Runs one complete scan, then lets automatic switching react to it. The host adapter
    /// schedules this every 1–2 seconds; fixture hosts drive the same operation explicitly,
    /// without reading the real /proc.
    pub fn scan_running_games(&self) {
        {
            let mut current = self.recognition.lock().unwrap();
            let programs = self
                .tuning_store
                .read(|document| {
                    document
                        .users
                        .iter()
                        .filter(|user| {
                            self.dependencies.users.username(user.uid).as_deref()
                                == Some(user.username.as_str())
                        })
                        .flat_map(|user| {
                            user.games.iter().filter_map(|entry| {
                                let GameId::Program { path } = &entry.game else {
                                    return None;
                                };
                                let identity = FileIdentity::at(
                                    &self
                                        .dependencies
                                        .host_root
                                        .join(path.trim_start_matches('/')),
                                )
                                .ok()?;
                                Some((user.uid, entry.game.clone(), identity))
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let next = Recognition(scan(&self.dependencies.host_root, &programs).map_err(|_| {
                "A complete host process scan could not be read. Recognition will retry automatically.".into()
            }));
            self.tuning_store.publish_for_users(|peer| {
                let state = next.for_uid(peer.uid);
                (state != current.for_uid(peer.uid))
                    .then_some(DaemonEvent::RunningGamesChanged(state))
            });
            *current = next;
        }
        self.evaluate_switching();
    }
}

fn scan(root: &Path, programs: &[(u32, GameId, FileIdentity)]) -> io::Result<GamesByUser> {
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
        let observed = match scan_process(&path, pid, programs) {
            Ok(observed) => observed,
            // ENOENT/ESRCH alone do not prove exit: kernel tasks and an unreadable
            // userspace process can still have a live stat entry. Verify the process
            // directory itself disappeared before treating a read failure as exit.
            Err(error)
                if (error.kind() == io::ErrorKind::NotFound
                    || error.raw_os_error() == Some(libc::ESRCH))
                    && matches!(fs::symlink_metadata(&path), Err(missing) if missing.kind() == io::ErrorKind::NotFound) =>
            {
                continue;
            }
            Err(error) => return Err(error),
        };
        for (uid, recognized) in observed {
            let earliest = games
                .entry((uid, recognized.game.clone()))
                .or_insert(recognized.clone());
            if (recognized.start_time_ticks, pid) < (earliest.start_time_ticks, earliest.pid) {
                *earliest = recognized;
            }
        }
    }
    let mut users: GamesByUser = BTreeMap::new();
    for ((uid, _), game) in games {
        users.entry(uid).or_default().push(game);
    }
    Ok(users)
}

/// A pid may be reused between reads. Never combine one process's environment
/// with another process's owner or start time. Failed reads remain unknown.
fn scan_process(
    path: &Path,
    pid: u32,
    programs: &[(u32, GameId, FileIdentity)],
) -> io::Result<Vec<(u32, RunningGame)>> {
    let stat = fs::read_to_string(path.join("stat"))?;
    let started = start_time(&stat, pid)?;
    if non_userspace_task(&stat)? {
        return Ok(Vec::new());
    }
    let env = fs::read(path.join("environ"))?;
    let primary = env
        .split(|byte| *byte == 0)
        .find_map(|entry| entry.strip_prefix(b"SteamAppId=").and_then(app_id));
    let recognized_app = match primary {
        Some(id) => Some(id),
        None => reaper_app(&fs::read(path.join("cmdline"))?),
    };
    if recognized_app.is_none() && programs.is_empty() {
        return Ok(Vec::new());
    }
    let status = fs::read_to_string(path.join("status"))?;
    let uid = status
        .lines()
        .find_map(|line| {
            line.strip_prefix("Uid:")
                .and_then(|uids| uids.split_whitespace().next())
                .and_then(|uid| uid.parse::<u32>().ok())
        })
        .ok_or_else(invalid_process)?;
    let mut recognized: Vec<GameId> = recognized_app
        .map(|app_id| GameId::Steam { app_id })
        .into_iter()
        .collect();
    if programs.iter().any(|(owner, _, _)| *owner == uid) {
        let exe = match candidate_executable(
            &path.join("exe"),
            programs
                .iter()
                .filter(|(owner, _, _)| *owner == uid)
                .map(|(_, _, identity)| *identity),
        ) {
            Ok(identity) => identity,
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let wine = fs::read_link(path.join("exe"))
            .map(|executable| is_wine_loader(&executable))
            .unwrap_or(false);
        let images = if wine {
            mapped_images(path)?
        } else {
            Vec::new()
        };
        for (owner, game, identity) in programs {
            if *owner == uid
                && (exe == Some(*identity)
                    || images.iter().any(|image| image.identity == *identity))
            {
                recognized.push(game.clone());
            }
        }
    }
    let stat = fs::read_to_string(path.join("stat"))?;
    let start_time_ticks = start_time(&stat, pid)?;
    if started != start_time_ticks {
        return Err(invalid_process());
    }
    if non_userspace_task(&stat)? {
        return Ok(Vec::new());
    }
    Ok(recognized
        .into_iter()
        .map(|game| {
            (
                uid,
                RunningGame {
                    game,
                    pid,
                    start_time_ticks,
                },
            )
        })
        .collect())
}

fn non_userspace_task(stat: &str) -> io::Result<bool> {
    let fields: Vec<_> = stat
        .rsplit_once(')')
        .ok_or_else(invalid_process)?
        .1
        .split_whitespace()
        .collect();
    // Linux task flags, field 9: PF_KTHREAD identifies a task without a userspace
    // environment. A zombie/dead task has already exited, even before it is reaped.
    const PF_KTHREAD: u64 = 0x0020_0000;
    let flags = fields
        .get(6)
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(invalid_process)?;
    Ok(flags & PF_KTHREAD != 0 || matches!(fields.first(), Some(&"Z" | &"X" | &"x")))
}

fn app_id(bytes: &[u8]) -> Option<u32> {
    std::str::from_utf8(bytes)
        .ok()?
        .parse()
        .ok()
        .filter(|id| *id > 0)
}

pub(super) fn start_time(stat: &str, pid: u32) -> io::Result<u64> {
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
