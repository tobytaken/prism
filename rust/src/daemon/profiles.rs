//! GPU tuning profiles in the tuning store: each user's own, keyed by GPU UUID, with names
//! unique per GPU regardless of letter case.

use std::fs::File;
use std::io::Read;

use super::session::SessionPeer;
use super::tuning_store::{TuningDocument, TuningStoreWriteError};
use crate::gpu::{
    ControlSupport, GpuCommandError, GpuCommandErrorCode, GpuPerformanceDevice,
    GpuPerformanceResponse,
};
use crate::protocol::{
    CurrentProfile, GpuTuningProfile, GpuTuningState, ProfileImportResponse,
    ProfileRangeCorrection, ProfileSetting, ProfileSettingKey, ProfileSettings, RangeBound,
    ReportedTuningValues,
};

impl From<TuningStoreWriteError> for GpuCommandError {
    fn from(error: TuningStoreWriteError) -> Self {
        match error {
            TuningStoreWriteError::Unhealthy => GpuCommandError::new(
                GpuCommandErrorCode::Io,
                "GPU tuning profiles are unavailable because Prism's tuning store can't be used.",
            ),
            TuningStoreWriteError::Io(error) => GpuCommandError::new(
                GpuCommandErrorCode::Io,
                format!("Failed to write Prism's tuning store: {error}"),
            ),
        }
    }
}

/// The GPU with this UUID in fresh readings, provided exactly one reports it.
pub(super) fn verified_gpu<'a>(
    performance: &'a GpuPerformanceResponse,
    uuid: &str,
) -> Result<&'a GpuPerformanceDevice, GpuCommandError> {
    let mut owners = performance
        .gpus
        .iter()
        .filter(|gpu| !uuid.trim().is_empty() && gpu.stable_id.as_deref() == Some(uuid));
    match (owners.next(), owners.next()) {
        (Some(gpu), None) => Ok(gpu),
        _ => Err(GpuCommandError::new(
            GpuCommandErrorCode::NotSupported,
            "Profiles are unavailable because this GPU's stable identity (UUID) could not be \
             verified. Refresh the GPU readings and try again.",
        )),
    }
}

/// The UUID of the GPU a live write addressed, if it reports one.
pub(super) fn uuid_of(performance: &GpuPerformanceResponse, gpu_id: &str) -> Option<String> {
    performance
        .gpus
        .iter()
        .find(|gpu| gpu.id == gpu_id)
        .and_then(|gpu| gpu.stable_id.clone())
        .filter(|uuid| !uuid.trim().is_empty())
}

struct Reading<'a> {
    label: &'static str,
    support: ControlSupport,
    current: Option<i64>,
    default: Option<i64>,
    error: Option<&'a str>,
}

fn reading(gpu: &GpuPerformanceDevice, key: ProfileSettingKey) -> Reading<'_> {
    fn offset<'a>(label: &'static str, info: &'a crate::gpu::ClockOffsetInfo) -> Reading<'a> {
        Reading {
            label,
            support: info.support,
            current: info.current_offset_mhz.map(i64::from),
            default: Some(0),
            error: info
                .current_value_error
                .as_deref()
                .or(info.reason.as_deref()),
        }
    }
    match key {
        ProfileSettingKey::PowerLimitWatts => Reading {
            label: "Power limit",
            support: gpu.power.support,
            current: gpu.power.current_limit_watts.map(i64::from),
            default: gpu.power.default_limit_watts.map(i64::from),
            error: gpu
                .power
                .current_value_error
                .as_deref()
                .or(gpu.power.reason.as_deref()),
        },
        ProfileSettingKey::GraphicsOffsetMhz => {
            offset("Graphics clock offset", &gpu.clocks.graphics)
        }
        ProfileSettingKey::MemoryOffsetMhz => offset("Memory clock offset", &gpu.clocks.memory),
    }
}

/// Captures every supported setting from the GPU's current values. A setting whose last
/// applied tuning value was a default request is saved as one while the GPU still reports
/// that default; a number equal to the default stays a number.
pub(super) fn capture_settings(
    gpu: &GpuPerformanceDevice,
    last_applied: Option<&ProfileSettings>,
) -> Result<ProfileSettings, GpuCommandError> {
    let blocked =
        |message: String| GpuCommandError::new(GpuCommandErrorCode::InvalidArgument, message);
    let refresh = "Refresh the GPU readings and try again.";
    let mut settings = ProfileSettings::default();
    for key in ProfileSettingKey::ALL {
        let reading = reading(gpu, key);
        match reading.support {
            ControlSupport::Unsupported => continue,
            ControlSupport::Unknown => {
                return Err(blocked(format!(
                    "{}: support is unknown. {}",
                    reading.label,
                    reading.error.unwrap_or(refresh)
                )))
            }
            ControlSupport::Supported => {}
        }
        let Some(current) = reading.current else {
            return Err(blocked(format!(
                "{}: current configured value is unavailable. {}",
                reading.label,
                reading.error.unwrap_or(refresh)
            )));
        };
        let follows_default =
            last_applied.and_then(|applied| applied.get(key)) == Some(ProfileSetting::Default {});
        settings.set(
            key,
            if follows_default && reading.default == Some(current) {
                ProfileSetting::Default {}
            } else {
                ProfileSetting::Value { value: current }
            },
        );
    }
    if settings.is_empty() {
        return Err(blocked(
            "This GPU has no supported profile settings (power limit or clock offsets).".into(),
        ));
    }
    Ok(settings)
}

/// A name with surrounding whitespace removed, unless nothing is left.
pub(super) fn profile_name(name: &str) -> Result<String, GpuCommandError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(GpuCommandError::new(
            GpuCommandErrorCode::InvalidArgument,
            "Enter a name for the profile.",
        ));
    }
    Ok(name.to_string())
}

fn name_taken(
    profiles: &[GpuTuningProfile],
    gpu_id: &str,
    name: &str,
    except_id: Option<&str>,
) -> bool {
    let name = name.to_lowercase();
    profiles.iter().any(|profile| {
        Some(profile.id.as_str()) != except_id
            && profile.gpu_id == gpu_id
            && profile.name.to_lowercase() == name
    })
}

/// FNV-1a: a stable digest, so a revision means the same state across daemon restarts.
fn digest(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

impl TuningDocument {
    /// The GPU's tuning state, read from the GPU's fresh readings. Its revision covers the
    /// reported values that saving captures, the recorded last applied tuning values and the
    /// current profile, so it changes whenever any of them does.
    pub fn tuning_state(&self, gpu: &GpuPerformanceDevice) -> GpuTuningState {
        let uuid = gpu.stable_id.clone().unwrap_or_default();
        let reported: Vec<_> = ProfileSettingKey::ALL
            .into_iter()
            .map(|key| {
                let reading = reading(gpu, key);
                (reading.support, reading.current, reading.default)
            })
            .collect();
        let current_id = self.current_profiles.get(&uuid);
        let current_profile = current_id
            .and_then(|id| self.any_profile(id))
            .map(|profile| CurrentProfile {
                id: profile.id.clone(),
                name: profile.name.clone(),
            });
        let state = serde_json::json!({
            "reported": reported,
            "lastApplied": self.last_applied.get(&uuid),
            "currentProfile": current_profile,
        });
        GpuTuningState {
            revision: format!("{:016x}", digest(state.to_string().as_bytes())),
            reported: ReportedTuningValues {
                power_limit_watts: reading(gpu, ProfileSettingKey::PowerLimitWatts).current,
                graphics_offset_mhz: reading(gpu, ProfileSettingKey::GraphicsOffsetMhz).current,
                memory_offset_mhz: reading(gpu, ProfileSettingKey::MemoryOffsetMhz).current,
            },
            current_profile,
            gpu_id: uuid,
        }
    }

    /// Rejects a save unless the caller saw the GPU's tuning state as it is now.
    pub fn check_revision(
        &self,
        gpu: &GpuPerformanceDevice,
        seen: &str,
    ) -> Result<(), GpuCommandError> {
        if self.tuning_state(gpu).revision == seen {
            return Ok(());
        }
        Err(GpuCommandError::new(
            GpuCommandErrorCode::InvalidArgument,
            "The GPU's values changed since you last looked. Review them and save again.",
        ))
    }

    /// Overwrites the GPU's current profile with these settings, provided the caller owns it.
    pub fn save_to_current(
        &mut self,
        peer: &SessionPeer,
        uuid: &str,
        settings: ProfileSettings,
    ) -> Result<GpuTuningProfile, GpuCommandError> {
        let no_current = || {
            GpuCommandError::new(
                GpuCommandErrorCode::InvalidArgument,
                "This GPU has no current profile. Save as a new profile instead.",
            )
        };
        let id = self
            .current_profiles
            .get(uuid)
            .ok_or_else(no_current)?
            .clone();
        let current = self
            .any_profile(&id)
            .filter(|profile| profile.gpu_id == uuid)
            .ok_or_else(no_current)?
            .name
            .clone();
        let own = self
            .users
            .iter_mut()
            .filter(|user| user.uid == peer.uid && user.username == peer.username)
            .flat_map(|user| user.profiles.iter_mut())
            .find(|profile| profile.id == id);
        let Some(profile) = own else {
            return Err(GpuCommandError::new(
                GpuCommandErrorCode::NoPermission,
                format!(
                    "The current profile \"{current}\" belongs to another user, so you can't save \
                     over it. Save as a new profile instead."
                ),
            ));
        };
        profile.settings = settings;
        Ok(profile.clone())
    }

    /// Any user's profile with this id; ids are unique across users.
    fn any_profile(&self, id: &str) -> Option<&GpuTuningProfile> {
        self.users
            .iter()
            .flat_map(|user| &user.profiles)
            .find(|profile| profile.id == id)
    }

    pub fn profiles_of(&self, peer: &SessionPeer) -> Vec<GpuTuningProfile> {
        self.user(peer.uid, &peer.username)
            .map(|user| user.profiles.clone())
            .unwrap_or_default()
    }

    pub fn profile_of(
        &self,
        peer: &SessionPeer,
        profile_id: &str,
    ) -> Result<GpuTuningProfile, GpuCommandError> {
        self.user(peer.uid, &peer.username)
            .and_then(|user| {
                user.profiles
                    .iter()
                    .find(|profile| profile.id == profile_id)
            })
            .cloned()
            .ok_or_else(profile_missing)
    }

    pub fn add_profile(
        &mut self,
        peer: &SessionPeer,
        profile: GpuTuningProfile,
    ) -> Result<GpuTuningProfile, GpuCommandError> {
        let profiles = &mut self.user_mut(peer.uid, &peer.username).profiles;
        if name_taken(profiles, &profile.gpu_id, &profile.name, None) {
            return Err(GpuCommandError::new(
                GpuCommandErrorCode::InvalidArgument,
                format!(
                    "A profile named \"{}\" already exists for this GPU. Choose another name.",
                    profile.name
                ),
            ));
        }
        profiles.push(profile.clone());
        Ok(profile)
    }

    /// Renames only the caller's profile, preserving its id and every reference to it.
    pub fn rename_profile(
        &mut self,
        peer: &SessionPeer,
        profile_id: &str,
        name: &str,
    ) -> Result<GpuTuningProfile, GpuCommandError> {
        let profile = self.profile_of(peer, profile_id)?;
        let name = profile_name(name)?;
        let profiles = &mut self.user_mut(peer.uid, &peer.username).profiles;
        if name_taken(profiles, &profile.gpu_id, &name, Some(profile_id)) {
            return Err(GpuCommandError::new(
                GpuCommandErrorCode::InvalidArgument,
                format!(
                    "A profile named \"{name}\" already exists for this GPU. Choose another name."
                ),
            ));
        }
        let profile = profiles
            .iter_mut()
            .find(|profile| profile.id == profile_id)
            .unwrap();
        profile.name = name;
        Ok(profile.clone())
    }

    /// Removes only the caller's profile and its current-profile references, leaving all
    /// current and last applied tuning values unchanged.
    pub fn delete_profile(
        &mut self,
        peer: &SessionPeer,
        profile_id: &str,
    ) -> Result<(), GpuCommandError> {
        self.profile_of(peer, profile_id)?;
        self.user_mut(peer.uid, &peer.username)
            .profiles
            .retain(|profile| profile.id != profile_id);
        self.current_profiles.retain(|_, id| id != profile_id);
        Ok(())
    }

    /// Adds the profiles under the caller, keeping their ids. A profile whose id already exists
    /// is skipped, and a name already used on the same GPU gets the first free " (n)" suffix.
    pub fn import_profiles(
        &mut self,
        peer: &SessionPeer,
        profiles: Vec<GpuTuningProfile>,
    ) -> Result<ProfileImportResponse, GpuCommandError> {
        let invalid = |message: &str| {
            GpuCommandError::new(
                GpuCommandErrorCode::InvalidArgument,
                format!("The profiles to import are invalid: {message}"),
            )
        };
        for profile in &profiles {
            if profile.id.trim().is_empty() {
                return Err(invalid("a profile has no id."));
            }
            if profile.gpu_id.trim().is_empty() {
                return Err(invalid("a profile has no owning GPU."));
            }
            if profile.settings.is_empty() {
                return Err(invalid("a profile has no settings."));
            }
            profile_name(&profile.name).map_err(|_| invalid("a profile has no name."))?;
        }
        let mut response = ProfileImportResponse {
            imported: vec![],
            skipped: vec![],
        };
        for mut profile in profiles {
            if self.any_profile(&profile.id).is_some() {
                response.skipped.push(profile.id);
                continue;
            }
            let own = &mut self.user_mut(peer.uid, &peer.username).profiles;
            let base = profile.name.trim().to_string();
            profile.name = base.clone();
            let mut suffix = 2;
            while name_taken(own, &profile.gpu_id, &profile.name, None) {
                profile.name = format!("{base} ({suffix})");
                suffix += 1;
            }
            own.push(profile.clone());
            response.imported.push(profile);
        }
        Ok(response)
    }

    /// Moves each reviewed saved value to the bound it exceeded, provided the profile still
    /// holds exactly the values that were reviewed.
    pub fn correct_profile(
        &mut self,
        peer: &SessionPeer,
        profile_id: &str,
        corrections: &[ProfileRangeCorrection],
    ) -> Result<GpuTuningProfile, GpuCommandError> {
        let reviewed = |correction: &&ProfileRangeCorrection| match correction.bound {
            RangeBound::Maximum => correction.proposed_value < correction.saved_value,
            RangeBound::Minimum => correction.proposed_value > correction.saved_value,
        };
        let distinct = corrections.iter().enumerate().all(|(index, correction)| {
            corrections[..index]
                .iter()
                .all(|earlier| earlier.setting != correction.setting)
        });
        if corrections.is_empty() || !distinct || !corrections.iter().all(|c| reviewed(&c)) {
            return Err(GpuCommandError::new(
                GpuCommandErrorCode::InvalidArgument,
                "The profile corrections are invalid. No GPU settings were changed.",
            ));
        }
        let profile = self
            .user_mut(peer.uid, &peer.username)
            .profiles
            .iter_mut()
            .find(|profile| profile.id == profile_id)
            .ok_or_else(profile_missing)?;
        for correction in corrections {
            if profile.settings.get(correction.setting)
                != Some(ProfileSetting::Value {
                    value: correction.saved_value,
                })
            {
                return Err(GpuCommandError::new(
                    GpuCommandErrorCode::InvalidArgument,
                    "The saved profile changed while its corrections were being reviewed. Load it \
                     again to review current corrections. No GPU settings were changed.",
                ));
            }
            profile.settings.set(
                correction.setting,
                ProfileSetting::Value {
                    value: correction.proposed_value,
                },
            );
        }
        Ok(profile.clone())
    }

    /// Records successfully applied manual instructions as the GPU's last applied tuning values.
    pub fn record_applied(&mut self, uuid: &str, applied: &[(ProfileSettingKey, ProfileSetting)]) {
        let settings = self.last_applied.entry(uuid.to_string()).or_default();
        for (key, setting) in applied {
            settings.set(*key, *setting);
        }
    }
}

fn profile_missing() -> GpuCommandError {
    GpuCommandError::new(
        GpuCommandErrorCode::NotFound,
        "The GPU tuning profile no longer exists. No GPU settings were changed.",
    )
}

/// A random, opaque profile id in the UUID format the app has always used.
pub(super) fn new_profile_id() -> Result<String, GpuCommandError> {
    let mut bytes = [0u8; 16];
    File::open("/dev/urandom")
        .and_then(|mut random| random.read_exact(&mut bytes))
        .map_err(|error| {
            GpuCommandError::new(
                GpuCommandErrorCode::Io,
                format!("Failed to create a profile id: {error}"),
            )
        })?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}
