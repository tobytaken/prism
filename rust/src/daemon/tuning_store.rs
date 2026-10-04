use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::gpu::{TuningStoreHealth, TuningStoreProblem};

pub const TUNING_STORE_FILE_NAME: &str = "tuning-state.json";

/// The schema version this daemon reads and writes.
pub const TUNING_STORE_SCHEMA_VERSION: u64 = 1;

/// Appended to the document's file name for the new contents of a write in progress.
pub const STAGED_FILE_SUFFIX: &str = ".tmp";

const SCHEMA_VERSION_KEY: &str = "schemaVersion";

/// The document as JSON, before it is read as a typed document.
type RawDocument = serde_json::Map<String, serde_json::Value>;

/// Upgrades a document by one schema version: the first entry upgrades version 1 to 2, and so on.
/// Bumping the schema version means adding its migration here.
const MIGRATIONS: [fn(RawDocument) -> Result<RawDocument, String>;
    TUNING_STORE_SCHEMA_VERSION as usize - 1] = [];

/// The persistent tuning state. It holds no user data yet.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TuningDocument {}

/// The daemon's persistent tuning state: one versioned document, separate from the legacy saved
/// configurations. It fails closed: a document it can't use is preserved untouched and never
/// rewritten.
pub struct TuningStore {
    path: PathBuf,
    state: Mutex<StoreState>,
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
        }
    }

    pub fn health(&self) -> TuningStoreHealth {
        match &*self.state.lock().unwrap() {
            StoreState::Healthy(_) => TuningStoreHealth::Healthy,
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
    /// Changes the document and writes it atomically at the current schema version. The change
    /// is kept only once it is on disk.
    // Nothing writes tuning state yet; profiles and startup choices will.
    #[allow(dead_code)]
    pub fn update(
        &self,
        change: impl FnOnce(&mut TuningDocument),
    ) -> Result<(), TuningStoreWriteError> {
        let mut state = self.state.lock().unwrap();
        let StoreState::Healthy(document) = &mut *state else {
            return Err(TuningStoreWriteError::Unhealthy);
        };
        let mut changed = document.clone();
        change(&mut changed);
        write_atomically(&self.path, &serialize(&changed)).map_err(TuningStoreWriteError::Io)?;
        *document = changed;
        Ok(())
    }
}

fn serialize(document: &TuningDocument) -> Vec<u8> {
    let serde_json::Value::Object(mut contents) = serde_json::to_value(document).unwrap() else {
        unreachable!("the tuning document is a JSON object");
    };
    contents.insert("schemaVersion".into(), TUNING_STORE_SCHEMA_VERSION.into());
    serde_json::to_vec_pretty(&contents).unwrap()
}

/// Replaces the file so that readers, and the daemon after a crash, see either the previous or
/// the new contents, never a mix.
fn write_atomically(path: &Path, contents: &[u8]) -> io::Result<()> {
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
    fn the_first_write_creates_the_document_at_the_current_schema_version() {
        let dir = tempfile::tempdir().unwrap();
        let store = TuningStore::open(store_path(&dir));

        store.update(|_| {}).unwrap();

        let written: serde_json::Value =
            serde_json::from_slice(&fs::read(store_path(&dir)).unwrap()).unwrap();
        assert_eq!(written, serde_json::json!({ "schemaVersion": 1 }));
        assert_eq!(
            TuningStore::open(store_path(&dir)).health(),
            TuningStoreHealth::Healthy
        );
    }

    #[test]
    fn an_unhealthy_store_refuses_every_write_and_keeps_its_bytes() {
        for preserved in [
            &b"{\"schemaVersion\": 1, \"profi"[..],
            b"{\"schemaVersion\": 7}",
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
    fn a_write_interrupted_before_the_rename_leaves_the_previous_document_intact() {
        let dir = tempfile::tempdir().unwrap();
        let store = TuningStore::open(store_path(&dir));
        store.update(|_| {}).unwrap();
        let previous = fs::read(store_path(&dir)).unwrap();

        let path = store_path(&dir);
        let interrupted = StagedFile::write(&path, b"{\"schemaVersion\": 1, \"half").unwrap();
        drop(interrupted);

        assert_eq!(fs::read(store_path(&dir)).unwrap(), previous);
        let reopened = TuningStore::open(store_path(&dir));
        assert_eq!(reopened.health(), TuningStoreHealth::Healthy);
        reopened.update(|_| {}).unwrap();
        assert_eq!(fs::read(store_path(&dir)).unwrap(), previous);
    }
}
