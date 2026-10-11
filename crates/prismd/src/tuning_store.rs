use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use super::session::{DaemonSession, SessionEvents, SessionPeer, SessionPolicy};
use prism_protocol::{
    DaemonEvent, GpuTuningProfile, GpuTuningState, ProfileSettings, StartupBehavior,
    TuningStoreHealth, TuningStoreProblem,
};

pub const TUNING_STORE_FILE_NAME: &str = "tuning-state.json";

/// The schema version this daemon reads and writes.
pub const TUNING_STORE_SCHEMA_VERSION: u64 = 12;

/// Appended to the document's file name for the new contents of a write in progress.
pub const STAGED_FILE_SUFFIX: &str = ".tmp";

const SCHEMA_VERSION_KEY: &str = "schema_version";

/// The document as JSON, before it is read as a typed document.
type RawDocument = serde_json::Map<String, serde_json::Value>;

/// Upgrades a document by one schema version: the first entry upgrades version 1 to 2, and so on.
/// Bumping the schema version means adding its migration here.
const MIGRATIONS: [fn(RawDocument) -> Result<RawDocument, String>;
    TUNING_STORE_SCHEMA_VERSION as usize - 1] = [
    // Version 2 adds users' profiles and last applied tuning values, both empty until written.
    Ok,
    // Version 3 adds each GPU's current profile, none until a profile is applied or saved.
    Ok,
    // Version 4 adds host-wide startup choices, defaulting to Off with no designations.
    Ok,
    // Version 5 adds Restore last applied and the unclean-shutdown guard's record, absent until
    // a boot restores last applied tuning values.
    Ok,
    // Version 6 marked the one-time import of legacy configurations, which no longer exists.
    Ok,
    // Version 7 adds users' game associations and automatic switching opt-ins, none until set.
    Ok,
    // Version 8 adds each user's remembered fan curves per GPU, empty until manually applied.
    Ok,
    // Version 9 adds optional complete cooling configurations; old profiles omit cooling.
    Ok,
    // Version 10 adds manually identified programs to game associations; existing Steam links remain valid.
    Ok, // Version 11 adds explicit host-wide switching pauses per GPU, initially empty.
    Ok, // Version 12 records the boot of automatic failures for the next startup outcome.
    Ok,
];

/// The tuning store's contents.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TuningDocument {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub automatic_failures: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub switching_pauses: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "StartupBehavior::is_off")]
    pub startup_behavior: StartupBehavior,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub startup_profiles: BTreeMap<String, String>,
    /// Each user's own state. A user is a uid together with its username, so a reused uid with
    /// another username is a different user.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub users: Vec<UserState>,
    /// Last applied tuning values per GPU UUID: the latest successful manual instruction for
    /// each setting, keeping explicit default requests.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub last_applied: BTreeMap<String, ProfileSettings>,
    /// Each GPU's current profile per GPU UUID: the id of the profile most recently applied to
    /// or saved from it, whichever user owns it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub current_profiles: BTreeMap<String, String>,
    /// The boot that restored last applied tuning values, until the host shuts down cleanly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore_attempt: Option<RestoreAttempt>,
}

/// The unclean-shutdown guard's record: a boot attempted to restore last applied tuning values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreAttempt {
    pub boot_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserState {
    pub uid: u32,
    pub username: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub remembered_curves: BTreeMap<String, prism_gpu::cooling::CoolingConfiguration>,
    #[serde(default)]
    pub profiles: Vec<GpuTuningProfile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub games: Vec<super::games::GameAssociation>,
    /// Whether the user opted in to automatic switching; off by default.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub automatic_switching: bool,
}

impl TuningDocument {
    /// Whether this boot restored last applied tuning values and hasn't yet shut down cleanly.
    pub fn restored_in(&self, boot_id: &str) -> bool {
        self.restore_attempt
            .as_ref()
            .is_some_and(|attempt| attempt.boot_id == boot_id)
    }

    pub fn user(&self, uid: u32, username: &str) -> Option<&UserState> {
        self.users
            .iter()
            .find(|user| user.uid == uid && user.username == username)
    }

    /// The user's state, added if the user has none yet.
    pub fn user_mut(&mut self, uid: u32, username: &str) -> &mut UserState {
        let index = match self
            .users
            .iter()
            .position(|user| user.uid == uid && user.username == username)
        {
            Some(index) => index,
            None => {
                self.users.push(UserState {
                    uid,
                    username: username.into(),
                    remembered_curves: BTreeMap::new(),
                    profiles: vec![],
                    games: vec![],
                    automatic_switching: false,
                });
                self.users.len() - 1
            }
        };
        &mut self.users[index]
    }
}

/// The daemon's tuning store: one versioned document. It fails closed: a document it can't use
/// is preserved untouched and never rewritten.
pub struct TuningStore {
    path: PathBuf,
    state: Mutex<StoreState>,
    events: Arc<SessionEvents>,
    /// The tuning state last pushed to sessions for each GPU, by UUID.
    pushed_tuning: Mutex<HashMap<String, GpuTuningState>>,
}

/// Pushes GPUs' changed tuning states to every session while it's held. Only one is held at a
/// time, so whatever reads the GPUs while holding it pushes the latest state last.
pub struct TuningPushes<'a> {
    events: &'a SessionEvents,
    pushed: MutexGuard<'a, HashMap<String, GpuTuningState>>,
}

impl TuningPushes<'_> {
    /// Pushes `state` unless it's the one last pushed for its GPU, or, before any was,
    /// `unchanged_from`.
    pub fn push(&mut self, state: GpuTuningState, unchanged_from: Option<&GpuTuningState>) {
        let last = self.pushed.get(&state.gpu_id).or(unchanged_from);
        if last != Some(&state) {
            self.events
                .publish(DaemonEvent::TuningStateChanged(state.clone()));
        }
        self.pushed.insert(state.gpu_id.clone(), state);
    }
}

enum StoreState {
    Healthy(TuningDocument),
    Unhealthy {
        problem: TuningStoreProblem,
        detail: String,
    },
}

impl TuningStore {
    pub fn open(path: PathBuf) -> Self {
        let state = match load(&path) {
            Ok(document) => StoreState::Healthy(document),
            Err((problem, detail)) => {
                eprintln!(
                    "Prism's tuning store {} is preserved untouched and won't be written: {detail}",
                    path.display()
                );
                StoreState::Unhealthy { problem, detail }
            }
        };
        Self {
            path,
            state: Mutex::new(state),
            events: Arc::default(),
            pushed_tuning: Mutex::default(),
        }
    }

    pub fn health(&self) -> TuningStoreHealth {
        self.health_of(&self.state.lock().unwrap())
    }

    pub fn held_gpus(&self) -> std::collections::BTreeSet<String> {
        self.events.held_gpus()
    }

    pub fn subscribe(&self, peer: SessionPeer, policy: SessionPolicy) -> DaemonSession {
        self.events.subscribe(peer, policy)
    }

    /// Builds a caller-scoped event for each session; None leaves that session unchanged.
    pub fn publish_for_users(&self, event: impl Fn(&SessionPeer) -> Option<DaemonEvent>) {
        self.events.publish_for_users(event);
    }

    /// Pushes the event to every open session.
    pub fn publish(&self, event: DaemonEvent) {
        self.events.publish(event);
    }

    /// Lets `observe` read GPUs' tuning states and push the changed ones; anything else pushing
    /// tuning states waits until it's done.
    pub fn push_tuning<T>(&self, observe: impl FnOnce(&mut TuningPushes) -> T) -> T {
        observe(&mut TuningPushes {
            events: &self.events,
            pushed: self.pushed_tuning.lock().unwrap(),
        })
    }

    /// Takes these tuning states, as a session's snapshot gave them, for those sessions know,
    /// unless a state was already pushed for their GPU.
    pub fn known_tuning(&self, states: &[GpuTuningState]) {
        let mut pushed = self.pushed_tuning.lock().unwrap();
        for state in states {
            pushed
                .entry(state.gpu_id.clone())
                .or_insert_with(|| state.clone());
        }
    }

    /// Whether any open session subscribed to telemetry.
    pub fn telemetry_subscribed(&self) -> bool {
        self.events.telemetry_subscribed()
    }

    /// Pushes the event to every session that subscribed to telemetry.
    pub fn publish_telemetry(&self, event: DaemonEvent) {
        self.events.publish_telemetry(event);
    }

    fn health_of(&self, state: &StoreState) -> TuningStoreHealth {
        match state {
            StoreState::Healthy(_) => TuningStoreHealth::Healthy {},
            StoreState::Unhealthy { problem, detail } => TuningStoreHealth::Unhealthy {
                problem: *problem,
                path: self.path.display().to_string(),
                detail: detail.clone(),
            },
        }
    }
}

#[derive(Debug)]
pub enum TuningStoreWriteError {
    /// The store is unhealthy, so its document is never rewritten.
    Unhealthy,
    Io(io::Error),
}

impl std::fmt::Display for TuningStoreWriteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unhealthy => write!(formatter, "the tuning store is unhealthy"),
            Self::Io(error) => write!(formatter, "failed to write the tuning store: {error}"),
        }
    }
}

impl TuningStore {
    /// Reads the document, unless the store is unhealthy.
    pub fn read<T>(
        &self,
        read: impl FnOnce(&TuningDocument) -> T,
    ) -> Result<T, TuningStoreWriteError> {
        match &*self.state.lock().unwrap() {
            StoreState::Healthy(document) => Ok(read(document)),
            StoreState::Unhealthy { .. } => Err(TuningStoreWriteError::Unhealthy),
        }
    }

    /// Changes the document and writes it atomically at the current schema version. The change
    /// is kept only once it is on disk.
    pub fn update(
        &self,
        change: impl FnOnce(&mut TuningDocument),
    ) -> Result<(), TuningStoreWriteError> {
        self.try_update(|document| {
            change(document);
            Ok::<_, TuningStoreWriteError>(())
        })
    }

    /// Like `update`, but a change that fails leaves the document unchanged and unwritten.
    pub fn try_update<T, E: From<TuningStoreWriteError>>(
        &self,
        change: impl FnOnce(&mut TuningDocument) -> Result<T, E>,
    ) -> Result<T, E> {
        let mut state = self.state.lock().unwrap();
        let StoreState::Healthy(document) = &mut *state else {
            return Err(TuningStoreWriteError::Unhealthy.into());
        };
        let mut changed = document.clone();
        let result = change(&mut changed)?;
        if let Err(error) = write_atomically(&self.path, &serialize(&changed)) {
            *state = StoreState::Unhealthy {
                problem: TuningStoreProblem::Unreadable,
                detail: format!("Failed to write the tuning store: {error}"),
            };
            self.events
                .publish(DaemonEvent::StoreHealthChanged(self.health_of(&state)));
            return Err(TuningStoreWriteError::Io(error).into());
        }
        *document = changed;
        Ok(result)
    }
}

pub(super) fn serialize(document: &TuningDocument) -> Vec<u8> {
    let serde_json::Value::Object(mut contents) = serde_json::to_value(document).unwrap() else {
        unreachable!("the tuning document is a JSON object");
    };
    contents.insert(
        SCHEMA_VERSION_KEY.into(),
        TUNING_STORE_SCHEMA_VERSION.into(),
    );
    serde_json::to_vec_pretty(&contents).unwrap()
}

/// Replaces the file so that readers, and the daemon after a crash, see either the previous or
/// the new contents, never a mix.
pub(super) fn write_atomically(path: &Path, contents: &[u8]) -> io::Result<()> {
    StagedFile::write(path, contents)?.commit()
}

/// New contents written and flushed beside the file they will replace. Until committed, the file
/// itself is untouched.
struct StagedFile<'a> {
    path: &'a Path,
    staged_path: PathBuf,
}

impl<'a> StagedFile<'a> {
    fn write(path: &'a Path, contents: &[u8]) -> io::Result<Self> {
        let parent = parent_dir(path);
        fs::create_dir_all(parent)?;
        let mut staged_path = path.as_os_str().to_owned();
        staged_path.push(STAGED_FILE_SUFFIX);
        let staged_path = PathBuf::from(staged_path);
        // A file left by an interrupted write is replaced, so the new one gets its own mode.
        match fs::remove_file(&staged_path) {
            Err(error) if error.kind() != ErrorKind::NotFound => return Err(error),
            _ => {}
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&staged_path)?;
        file.write_all(contents)?;
        file.sync_all()?;
        Ok(Self { path, staged_path })
    }

    fn commit(self) -> io::Result<()> {
        fs::rename(&self.staged_path, self.path)?;
        File::open(parent_dir(self.path))?.sync_all()
    }
}

fn parent_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

type LoadError = (TuningStoreProblem, String);

/// Reads the document, treating a missing one as empty.
fn load(path: &Path) -> Result<TuningDocument, LoadError> {
    let unreadable = |detail: String| (TuningStoreProblem::Unreadable, detail);
    let contents = match fs::read(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(TuningDocument::default()),
        Err(error) => return Err(unreadable(error.to_string())),
    };
    let mut document: RawDocument =
        serde_json::from_slice(&contents).map_err(|error| unreadable(error.to_string()))?;
    let version = document
        .remove(SCHEMA_VERSION_KEY)
        .ok_or_else(|| unreadable("the document has no schema version".into()))?;
    let version = version
        .as_u64()
        .ok_or_else(|| unreadable(format!("the schema version {version} is not a version")))?;
    if version > TUNING_STORE_SCHEMA_VERSION {
        return Err((
            TuningStoreProblem::NewerVersion,
            format!(
                "it was written by schema version {version}, and this daemon understands up to \
                 version {TUNING_STORE_SCHEMA_VERSION}"
            ),
        ));
    }
    if version == 0 {
        return Err(unreadable("unknown schema version 0".into()));
    }
    for migrate in &MIGRATIONS[version as usize - 1..] {
        document = migrate(document).map_err(unreadable)?;
    }
    serde_json::from_value(serde_json::Value::Object(document))
        .map_err(|error| unreadable(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_path(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("state").join(TUNING_STORE_FILE_NAME)
    }

    #[test]
    fn a_write_failure_pushes_the_new_health_to_every_session_and_preserves_the_document() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(&dir);
        let store = TuningStore::open(path.clone());
        store.update(|_| {}).unwrap();
        let before = fs::read(&path).unwrap();
        let sessions: Vec<_> = [(1000, "alice"), (1001, "bob")]
            .into_iter()
            .map(|(uid, username)| {
                store.subscribe(
                    SessionPeer {
                        uid,
                        username: username.into(),
                        groups: vec![],
                        primary_group: "users".into(),
                        home: format!("/home/{username}").into(),
                    },
                    SessionPolicy {
                        admin_group: "admins".into(),
                        admin_user: None,
                    },
                )
            })
            .collect();
        fs::create_dir(path.with_file_name("tuning-state.json.tmp")).unwrap();
        assert!(store.update(|_| {}).is_err());
        for session in &sessions {
            let DaemonEvent::StoreHealthChanged(health) = session
                .recv_timeout(std::time::Duration::from_secs(1))
                .unwrap()
            else {
                panic!("expected the store's health")
            };
            assert!(matches!(
                health,
                TuningStoreHealth::Unhealthy {
                    problem: TuningStoreProblem::Unreadable,
                    ..
                }
            ));
            assert_eq!(health, store.health());
        }
        assert_eq!(fs::read(path).unwrap(), before);
    }

    #[test]
    fn the_first_write_creates_the_document_at_the_current_schema_version() {
        let dir = tempfile::tempdir().unwrap();
        let store = TuningStore::open(store_path(&dir));

        store.update(|_| {}).unwrap();

        let written: serde_json::Value =
            serde_json::from_slice(&fs::read(store_path(&dir)).unwrap()).unwrap();
        assert_eq!(
            written,
            serde_json::json!({ "schema_version": TUNING_STORE_SCHEMA_VERSION })
        );
        assert_eq!(
            TuningStore::open(store_path(&dir)).health(),
            TuningStoreHealth::Healthy {}
        );
    }

    #[test]
    fn version_eleven_pauses_survive_adding_failure_notes_and_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(&dir);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            br#"{"schema_version":11,"switching_pauses":["GPU-paused"]}"#,
        )
        .unwrap();
        let store = TuningStore::open(path.clone());
        store
            .update(|document| {
                document
                    .automatic_failures
                    .insert("GPU-failed".into(), "boot-1".into());
            })
            .unwrap();

        let reopened = TuningStore::open(path.clone());
        reopened
            .read(|document| {
                assert!(document.switching_pauses.contains("GPU-paused"));
                assert_eq!(document.automatic_failures["GPU-failed"], "boot-1");
            })
            .unwrap();
        let written: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(written["schema_version"], TUNING_STORE_SCHEMA_VERSION);
    }

    #[test]
    fn an_unhealthy_store_refuses_every_write_and_keeps_its_bytes() {
        for preserved in [
            &b"{\"schema_version\": 1, \"profi"[..],
            b"{\"schema_version\": 99}",
        ] {
            let dir = tempfile::tempdir().unwrap();
            fs::create_dir_all(store_path(&dir).parent().unwrap()).unwrap();
            fs::write(store_path(&dir), preserved).unwrap();
            let store = TuningStore::open(store_path(&dir));

            assert!(matches!(
                store.update(|_| {}),
                Err(TuningStoreWriteError::Unhealthy)
            ));
            assert_eq!(fs::read(store_path(&dir)).unwrap(), preserved);
        }
    }

    #[test]
    fn a_store_written_with_camel_case_keys_is_unhealthy_and_preserved() {
        // Stores from before the keys matched Rust field names aren't migrated.
        for preserved in [
            &br#"{"schemaVersion": 12, "startupBehavior": "restoreLastApplied"}"#[..],
            // Even under the current schema version key, camelCase fields are unknown.
            br#"{"schema_version": 12, "startupBehavior": "restoreLastApplied"}"#,
        ] {
            let dir = tempfile::tempdir().unwrap();
            fs::create_dir_all(store_path(&dir).parent().unwrap()).unwrap();
            fs::write(store_path(&dir), preserved).unwrap();
            let store = TuningStore::open(store_path(&dir));

            assert!(matches!(
                store.health(),
                TuningStoreHealth::Unhealthy {
                    problem: TuningStoreProblem::Unreadable,
                    ..
                }
            ));
            assert!(matches!(
                store.update(|_| {}),
                Err(TuningStoreWriteError::Unhealthy)
            ));
            assert_eq!(fs::read(store_path(&dir)).unwrap(), preserved);
        }
    }

    #[test]
    fn a_write_interrupted_before_the_rename_leaves_the_previous_document_intact() {
        let dir = tempfile::tempdir().unwrap();
        let store = TuningStore::open(store_path(&dir));
        store.update(|_| {}).unwrap();
        let previous = fs::read(store_path(&dir)).unwrap();

        let path = store_path(&dir);
        let interrupted = StagedFile::write(&path, b"{\"schema_version\": 1, \"half").unwrap();
        drop(interrupted);

        assert_eq!(fs::read(store_path(&dir)).unwrap(), previous);
        let reopened = TuningStore::open(store_path(&dir));
        assert_eq!(reopened.health(), TuningStoreHealth::Healthy {});
        reopened.update(|_| {}).unwrap();
        assert_eq!(fs::read(store_path(&dir)).unwrap(), previous);
    }
}
