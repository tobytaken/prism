//! Startup's boot-scoped record. Claim the boot durably before any GPU write.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::session::SessionPeer;
use super::tuning_store::write_atomically;
use prism_gpu::{GpuCommandError, GpuCommandErrorCode};
use prism_protocol::StartupOutcome;

pub(super) const STARTUP_IN_PROGRESS: &str = "Startup is in progress.";
const SCHEMA_VERSION: u64 = 1;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BootDocument {
    schema_version: u64,
    boot_id: String,
    startup_ran: bool,
    in_progress: BTreeSet<String>,
    outcomes: Vec<StartupOutcome>,
    dismissed: Vec<Dismissal>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Dismissal {
    uid: u32,
    username: String,
    gpu_id: String,
}

pub(super) struct BootState {
    path: PathBuf,
    state: Mutex<Result<BootDocument, String>>,
}

impl BootState {
    pub fn open(dir: &std::path::Path, boot_id: &str) -> Self {
        let path = dir.join("startup-state.json");
        let load = || -> Result<BootDocument, String> {
            if boot_id.is_empty() {
                return Err("The kernel boot id is unavailable.".into());
            }
            let fresh = || BootDocument {
                schema_version: SCHEMA_VERSION,
                boot_id: boot_id.into(),
                startup_ran: false,
                in_progress: BTreeSet::new(),
                outcomes: vec![],
                dismissed: vec![],
            };
            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(fresh()),
                Err(error) => return Err(error.to_string()),
            };
            let mut document: BootDocument =
                serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
            if document.schema_version != SCHEMA_VERSION {
                return Err("The boot state has an unsupported schema version.".into());
            }
            if document.boot_id != boot_id {
                return Ok(fresh());
            }
            // The marker already claimed these operations. An interrupted startup never retries
            // settings after a restart, even if the GPU arrives later in the same boot.
            for outcome in &mut document.outcomes {
                if document.in_progress.contains(&outcome.gpu_id) {
                    outcome.reason = Some("Startup was interrupted by a daemon restart; nothing further will be applied this boot.".into());
                }
            }
            document.in_progress.clear();
            Ok(document)
        };
        Self {
            state: Mutex::new(load()),
            path,
        }
    }

    /// Returns false when startup has already claimed this boot.
    pub fn begin(
        &self,
        outcomes: Vec<StartupOutcome>,
        in_progress: BTreeSet<String>,
    ) -> Result<bool, GpuCommandError> {
        self.change(|document| {
            if document.startup_ran {
                return false;
            }
            document.startup_ran = true;
            document.outcomes = outcomes;
            document.in_progress = in_progress;
            true
        })
    }

    pub fn outcomes(&self, peer: &SessionPeer) -> Result<Vec<StartupOutcome>, GpuCommandError> {
        let state = self.state.lock().unwrap();
        let document = state.as_ref().map_err(|error| unavailable(error))?;
        Ok(document
            .outcomes
            .iter()
            .filter(|outcome| {
                !document.dismissed.iter().any(|dismissal| {
                    dismissal.uid == peer.uid
                        && dismissal.username == peer.username
                        && dismissal.gpu_id == outcome.gpu_id
                })
            })
            .cloned()
            .collect())
    }

    pub fn record(&self, outcome: StartupOutcome) -> Result<(), GpuCommandError> {
        self.change(|document| {
            document.in_progress.remove(&outcome.gpu_id);
            document
                .outcomes
                .retain(|previous| previous.gpu_id != outcome.gpu_id);
            document.outcomes.push(outcome);
            document.outcomes.sort_by(|a, b| a.gpu_id.cmp(&b.gpu_id));
        })
    }

    pub fn dismiss(&self, peer: &SessionPeer, gpu_id: &str) -> Result<(), GpuCommandError> {
        self.change(|document| {
            if !document.dismissed.iter().any(|dismissal| {
                dismissal.uid == peer.uid
                    && dismissal.username == peer.username
                    && dismissal.gpu_id == gpu_id
            }) {
                document.dismissed.push(Dismissal {
                    uid: peer.uid,
                    username: peer.username.clone(),
                    gpu_id: gpu_id.into(),
                });
            }
        })
    }

    fn change<T>(&self, change: impl FnOnce(&mut BootDocument) -> T) -> Result<T, GpuCommandError> {
        let mut state = self.state.lock().unwrap();
        let document = state.as_ref().map_err(|error| unavailable(error))?;
        let mut changed = document.clone();
        let result = change(&mut changed);
        let bytes = serde_json::to_vec_pretty(&changed).unwrap();
        if let Err(error) = write_atomically(&self.path, &bytes) {
            *state = Err(error.to_string());
            return Err(unavailable(&error.to_string()));
        }
        *state = Ok(changed);
        Ok(result)
    }
}

fn unavailable(detail: &str) -> GpuCommandError {
    GpuCommandError::new(
        GpuCommandErrorCode::Io,
        format!("Startup is suspended because Prism's boot state can't be used: {detail}"),
    )
}
