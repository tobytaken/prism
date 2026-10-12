//! The daemon's Activity: what it did this boot, such as profile applications, automatic switches
//! and startup outcomes, so a Prism opened later can show what happened while it was closed. It
//! lives with the other boot-scoped state, so it survives a daemon restart and is cleared at
//! reboot. Losing it loses only history, so a record that can't be read starts over.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use super::automatic_tuning::ProfileApplicationOutcomes;
use super::host::Clock;
use super::session::SessionPeer;
use super::tuning_store::{TuningStore, write_atomically};
use prism_gpu::TuningOutcomeStatus;
use prism_protocol::{
    ActivityEntry, ActivityEvent, DaemonEvent, ProfileApplicationResponse, trim_activity,
};

pub(super) const ACTIVITY_FILE_NAME: &str = "activity.json";
const SCHEMA_VERSION: u64 = 1;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActivityDocument {
    schema_version: u64,
    boot_id: String,
    /// The id of the latest entry, kept when it's dropped, so ids never repeat this boot.
    last_id: u64,
    entries: Vec<StoredEntry>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredEntry {
    /// The user who may see the entry, or none for every user.
    user: Option<ActivityUser>,
    entry: ActivityEntry,
}

/// A user, by uid and the username their session opened with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ActivityUser {
    pub uid: u32,
    pub username: String,
}

impl ActivityUser {
    pub fn of(peer: &SessionPeer) -> Self {
        Self {
            uid: peer.uid,
            username: peer.username.clone(),
        }
    }

    fn is(&self, peer: &SessionPeer) -> bool {
        self.uid == peer.uid && self.username == peer.username
    }
}

pub(super) struct Activity {
    /// None when the boot id is unknown, so a record couldn't be told apart from a previous
    /// boot's: Activity is then kept in memory only.
    path: Option<PathBuf>,
    clock: Arc<dyn Clock>,
    /// Pushes each new entry to the sessions that may see it.
    store: Arc<TuningStore>,
    document: Mutex<ActivityDocument>,
}

impl Activity {
    pub fn open(dir: &Path, boot_id: &str, clock: Arc<dyn Clock>, store: Arc<TuningStore>) -> Self {
        let path = (!boot_id.is_empty()).then(|| dir.join(ACTIVITY_FILE_NAME));
        let fresh = ActivityDocument {
            schema_version: SCHEMA_VERSION,
            boot_id: boot_id.into(),
            last_id: 0,
            entries: vec![],
        };
        let document = match path.as_deref().map(fs::read) {
            None => fresh,
            Some(Err(error)) if error.kind() == ErrorKind::NotFound => fresh,
            Some(bytes) => bytes
                .map_err(|error| error.to_string())
                .and_then(|bytes| {
                    serde_json::from_slice::<ActivityDocument>(&bytes)
                        .map_err(|error| error.to_string())
                })
                .and_then(|document| {
                    if document.schema_version == SCHEMA_VERSION {
                        Ok(document)
                    } else {
                        Err("it has an unsupported schema version".into())
                    }
                })
                .map(|document| {
                    if document.boot_id == boot_id {
                        document
                    } else {
                        fresh.clone()
                    }
                })
                .unwrap_or_else(|error| {
                    eprintln!("Activity starts over because it can't be read: {error}");
                    fresh.clone()
                }),
        };
        Self {
            path,
            clock,
            store,
            document: Mutex::new(document),
        }
    }

    /// Records that this happened to the GPU, visible to `user` or with none to everyone, and
    /// pushes it to the sessions that may see it.
    pub fn record(&self, user: Option<ActivityUser>, gpu_id: &str, event: ActivityEvent) {
        let mut document = self.document.lock().unwrap();
        document.last_id += 1;
        let entry = ActivityEntry {
            id: document.last_id,
            at_unix_ms: self.clock.now_unix_ms(),
            gpu_id: gpu_id.into(),
            event,
        };
        document.entries.push(StoredEntry {
            user: user.clone(),
            entry: entry.clone(),
        });
        trim_activity(&mut document.entries, |stored| &stored.entry);
        // Kept in memory even when it can't be written: only a daemon restart loses it then.
        if let Some(path) = &self.path
            && let Err(error) =
                write_atomically(path, &serde_json::to_vec_pretty(&*document).unwrap())
        {
            eprintln!("failed to record Activity: {error}");
        }
        // Pushed while still held, so sessions get entries in the order of their ids.
        self.store.publish_for_users(|peer| {
            user.as_ref()
                .is_none_or(|user| user.is(peer))
                .then(|| DaemonEvent::ActivityRecorded(entry.clone()))
        });
    }

    /// The entries the caller may see, oldest first.
    pub fn entries_for(&self, peer: &SessionPeer) -> Vec<ActivityEntry> {
        self.document
            .lock()
            .unwrap()
            .entries
            .iter()
            .filter(|stored| stored.user.as_ref().is_none_or(|user| user.is(peer)))
            .map(|stored| stored.entry.clone())
            .collect()
    }
}

/// Whether the application tried to change any setting; one that wrote nothing, such as one
/// waiting for range corrections, isn't worth recording.
pub(super) fn attempted(application: &ProfileApplicationResponse) -> bool {
    application
        .outcomes()
        .any(|outcome| outcome.status != TuningOutcomeStatus::Unattempted)
}
