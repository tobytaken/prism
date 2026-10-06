//! Settings whose latest manual write reached the GPU but couldn't be recorded in its last
//! applied tuning values. The marker lives beside the tuning store rather than in it, because it
//! is needed exactly when the store can't be written.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Mutex;

use super::tuning_store::write_atomically;
use crate::protocol::ProfileSettingKey;

pub const POSSIBLY_STALE_FILE_NAME: &str = "last-applied-possibly-stale.json";

type StaleSettings = BTreeMap<String, BTreeSet<ProfileSettingKey>>;

pub(super) struct PossiblyStale {
    path: PathBuf,
    settings: Mutex<StaleSettings>,
}

impl PossiblyStale {
    pub fn open(path: PathBuf) -> Self {
        let settings = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| error.to_string()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(StaleSettings::new()),
            Err(error) => Err(error.to_string()),
        }
        .unwrap_or_else(|error| {
            eprintln!(
                "failed to read which last applied tuning values are possibly stale from {}: {error}",
                path.display()
            );
            StaleSettings::new()
        });
        Self {
            path,
            settings: Mutex::new(settings),
        }
    }

    /// GPU UUIDs with any possibly stale setting.
    pub fn gpus(&self) -> Vec<String> {
        self.settings.lock().unwrap().keys().cloned().collect()
    }

    /// The settings' latest manual writes reached the GPU without being recorded.
    pub fn mark(&self, uuid: &str, keys: impl IntoIterator<Item = ProfileSettingKey>) {
        self.change(|settings| {
            settings.entry(uuid.into()).or_default().extend(keys);
        });
    }

    /// The settings' latest manual writes are recorded.
    pub fn clear(&self, uuid: &str, keys: impl IntoIterator<Item = ProfileSettingKey>) {
        self.change(|settings| {
            if let Some(stale) = settings.get_mut(uuid) {
                for key in keys {
                    stale.remove(&key);
                }
                if stale.is_empty() {
                    settings.remove(uuid);
                }
            }
        });
    }

    /// Keeps the change in memory even when it can't be written, so this daemon still reports it.
    fn change(&self, change: impl FnOnce(&mut StaleSettings)) {
        let mut settings = self.settings.lock().unwrap();
        let before = settings.clone();
        change(&mut settings);
        if *settings == before {
            return;
        }
        let bytes = serde_json::to_vec_pretty(&*settings).unwrap();
        if let Err(error) = write_atomically(&self.path, &bytes) {
            eprintln!(
                "failed to record which last applied tuning values are possibly stale in {}: {error}",
                self.path.display()
            );
        }
    }
}
