//! The schema-6 handoff from state.json to host-held startup profiles.
pub(super) const LEGACY_STATE_FILE_NAME: &str = "state.json";
use super::profiles::new_profile_id;
use super::session::SessionPeer;
use super::tuning_store::{
    serialize, write_atomically, TuningDocument, TuningStore, TuningStoreWriteError,
};
use crate::gpu::GpuPerformanceResponse;
use crate::protocol::{
    DroppedLegacyConfig, GpuTuningProfile, MigrationNotice, MigrationState, ProfileSetting,
    ProfileSettings, StartupBehavior,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyMigration {
    pub notice: MigrationNotice,
    pub offered_to: Option<MigrationUser>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MigrationUser {
    uid: u32,
    username: String,
}
impl MigrationUser {
    fn is(&self, peer: &SessionPeer) -> bool {
        self.uid == peer.uid && self.username == peer.username
    }
}

pub(super) fn upgrade(
    path: &Path,
    mut document: TuningDocument,
    inventory: impl FnOnce() -> GpuPerformanceResponse,
) -> Result<TuningDocument, String> {
    let legacy = path.with_file_name(LEGACY_STATE_FILE_NAME);
    let contents = match fs::read(&legacy) {
        Ok(contents) => Some(contents),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => return Err(format!("Could not read legacy settings: {error}")),
    };
    if let Some(contents) = contents {
        let saved: PersistedGpuState = serde_json::from_slice(&contents)
            .map_err(|error| format!("Could not read legacy settings: {error}"))?;
        let inventory = inventory();
        let mut dropped = vec![];
        // UUID entries win over aliases when several old keys address the same GPU.
        let mut resolved = BTreeMap::new();
        let mut entries: Vec<_> = saved.gpus.into_iter().collect();
        entries.sort_by_key(|(key, _)| !key.starts_with("GPU-"));
        for (key, record) in entries {
            let uuid = if key.starts_with("GPU-") {
                Some(key.clone())
            } else {
                let matches: Vec<_> = inventory
                    .gpus
                    .iter()
                    .filter(|gpu| {
                        gpu.id == key
                            || gpu.bus_id.as_deref() == Some(&key)
                            || gpu
                                .nvidia_index
                                .is_some_and(|index| format!("nvidia-{index}") == key)
                    })
                    .collect();
                if matches.len() == 1 {
                    matches[0]
                        .stable_id
                        .clone()
                        .filter(|id| !id.trim().is_empty())
                } else {
                    None
                }
            };
            let reason = match uuid.as_ref() {
                None => Some("No unique GPU with a UUID matched this key."),
                Some(uuid) if resolved.contains_key(uuid) => {
                    Some("Another legacy configuration already resolved to this GPU.")
                }
                _ => None,
            };
            if let Some(reason) = reason {
                dropped.push(DroppedLegacyConfig {
                    key,
                    values: serde_json::to_value(&record.desired).unwrap(),
                    reason: reason.into(),
                });
                continue;
            }
            let desired = record.desired;
            let value = |value| ProfileSetting::Value { value };
            let profile = GpuTuningProfile {
                id: new_profile_id().map_err(|error| error.message)?,
                name: "Previous startup settings".into(),
                gpu_id: uuid.unwrap(),
                settings: ProfileSettings {
                    power_limit_watts: desired.power_limit_watts.map(|v| value(i64::from(v))),
                    graphics_offset_mhz: desired.graphics_offset_mhz.map(|v| value(i64::from(v))),
                    memory_offset_mhz: desired.memory_offset_mhz.map(|v| value(i64::from(v))),
                },
            };
            resolved.insert(profile.gpu_id.clone(), profile);
        }
        if !resolved.is_empty() || !dropped.is_empty() {
            document.startup_behavior = StartupBehavior::StartupProfiles;
            document.legacy_migration = Some(LegacyMigration {
                notice: MigrationNotice { dropped },
                offered_to: None,
            });
        }
        for (uuid, profile) in resolved {
            document.startup_profiles.insert(uuid, profile.id.clone());
            document.ownerless_profiles.push(profile);
        }
    }
    // The schema version is the commit marker. An interrupted rename is retried without reimport.
    write_atomically(path, &serialize(&document)).map_err(|error| error.to_string())?;
    archive(path)?;
    Ok(document)
}

pub(super) fn archive(path: &Path) -> Result<(), String> {
    let legacy = path.with_file_name(LEGACY_STATE_FILE_NAME);
    let archived = legacy.with_extension("json.migrated");
    // A downgraded daemon can recreate state.json. The original archive is the completed
    // handoff: preserve both files instead of replacing it or importing the new file.
    match fs::symlink_metadata(&archived) {
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Could not inspect the legacy archive: {error}")),
    }
    match fs::rename(&legacy, &archived) {
        Ok(()) => fs::File::open(path.parent().unwrap())
            .and_then(|dir| dir.sync_all())
            .map_err(|error| error.to_string()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Could not archive legacy settings: {error}")),
    }
}

impl TuningStore {
    pub(super) fn reserve_migration_adoption(
        &self,
        peer: &SessionPeer,
    ) -> Result<(), TuningStoreWriteError> {
        if !self.read(|document| {
            document
                .legacy_migration
                .as_ref()
                .is_some_and(|migration| migration.offered_to.is_none())
        })? {
            return Ok(());
        }
        self.update(|document| {
            if let Some(migration) = &mut document.legacy_migration {
                migration.offered_to.get_or_insert_with(|| MigrationUser {
                    uid: peer.uid,
                    username: peer.username.clone(),
                });
            }
        })
    }
}

impl TuningDocument {
    pub(super) fn migration_state(&self, peer: &SessionPeer) -> MigrationState {
        MigrationState {
            profiles: self.ownerless_profiles.clone(),
            can_adopt: self
                .legacy_migration
                .as_ref()
                .and_then(|migration| migration.offered_to.as_ref())
                .is_some_and(|user| user.is(peer)),
            notice: self
                .legacy_migration
                .as_ref()
                .filter(|_| {
                    !self
                        .user(peer.uid, &peer.username)
                        .is_some_and(|user| user.migration_notice_dismissed)
                })
                .map(|migration| migration.notice.clone()),
        }
    }
}

impl TuningDocument {
    pub(super) fn adopt_migrated_profile(
        &mut self,
        peer: &SessionPeer,
        id: &str,
    ) -> Result<GpuTuningProfile, crate::gpu::GpuCommandError> {
        use crate::gpu::{GpuCommandError, GpuCommandErrorCode};
        if !self.migration_state(peer).can_adopt {
            return Err(GpuCommandError::new(
                GpuCommandErrorCode::NoPermission,
                "These profiles were offered to the first authorized user to open Prism.",
            ));
        }
        let index = self
            .ownerless_profiles
            .iter()
            .position(|profile| profile.id == id)
            .ok_or_else(|| {
                GpuCommandError::new(
                    GpuCommandErrorCode::NotFound,
                    "This profile has already been adopted or no longer exists.",
                )
            })?;
        let mut profile = self.ownerless_profiles.remove(index);
        // Keep the startup link and id; avoid a collision with the adopting user's names.
        let own = &self.user_mut(peer.uid, &peer.username).profiles;
        let base = profile.name.clone();
        let mut suffix = 2;
        while own.iter().any(|other| {
            other.gpu_id == profile.gpu_id
                && other.name.to_lowercase() == profile.name.to_lowercase()
        }) {
            profile.name = format!("{base} ({suffix})");
            suffix += 1;
        }
        self.add_profile(peer, profile)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DesiredGpuConfig {
    pub power_limit_watts: Option<u32>,
    pub gpu_clock_lock: Option<ClockRangeInput>,
    pub memory_clock_lock: Option<ClockRangeInput>,
    #[serde(rename = "graphicsOffsetMHz")]
    pub graphics_offset_mhz: Option<i32>,
    #[serde(rename = "memoryOffsetMHz")]
    pub memory_offset_mhz: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GpuApplyRecord {
    pub applied_at_unix_ms: u64,
    pub success: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PersistedGpuConfig {
    #[serde(default)]
    pub desired: DesiredGpuConfig,
    pub last_apply: Option<GpuApplyRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PersistedGpuState {
    #[serde(default)]
    pub gpus: BTreeMap<String, PersistedGpuConfig>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClockRangeInput {
    pub min_mhz: u32,
    pub max_mhz: u32,
}
