//! Switching GPU tuning profiles: each GPU's profiles as prismd pushes them, applying one at once,
//! asking first when that would discard the GPU's draft, and the single review of the range
//! corrections an application needs (ADR 0001).

use prism_gpu::TuningOutcomeStatus;
use prism_protocol::{
    CorrectAndApplyProfileRequest, DaemonRequest, DaemonResponse, GpuTuningProfile,
    ProfileApplicationResponse, ProfileIdRequest, ProfileRangeCorrection, ProfileSettingKey,
};

use crate::cooling::cooling_text;
use crate::core::{CommandId, Core, Effect};
use crate::drafts::{DialogState, ReviewChange, not_applied, setting_name, setting_readout};

/// A profile application waiting for prismd's answer.
#[derive(Debug, Clone)]
pub(crate) struct ProfileApplying {
    command: CommandId,
    profile_id: String,
    /// The profile's name when it was chosen, for what its outcome says.
    name: String,
}

impl Core {
    /// The caller's profile with this id.
    pub(crate) fn profile(&self, profile_id: &str) -> Option<&GpuTuningProfile> {
        self.profiles
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
    }

    /// The caller's profiles of the GPU, in the order prismd lists them.
    pub(crate) fn profiles_of(&self, gpu_id: &str) -> impl Iterator<Item = &GpuTuningProfile> {
        self.profiles
            .profiles
            .iter()
            .filter(move |profile| profile.gpu_id == gpu_id)
    }

    /// Why the profile can't be applied now, when it can't. A profile of a GPU that isn't here
    /// stays stored, but can't be applied.
    pub(crate) fn profile_unavailable(&self, profile_id: &str) -> Option<String> {
        let Some(profile) = self.profile(profile_id) else {
            return Some("The profile no longer exists".into());
        };
        if !self.gpus().iter().any(|gpu| gpu.id == profile.gpu_id) {
            return Some("Its GPU isn't here".into());
        }
        if let Some(reason) = self.tuning_access().reason() {
            return Some(format!("Needs tuning access · {reason}"));
        }
        self.gpu_applying(&profile.gpu_id)
            .then(|| "Applying…".into())
    }

    /// Whether an Apply or a profile application on the GPU waits for prismd's answer.
    pub(crate) fn gpu_applying(&self, gpu_id: &str) -> bool {
        self.applying(gpu_id) || self.profile_applications.contains_key(gpu_id)
    }

    /// Applies the profile at once, or first asks whether to discard the unapplied tuning changes
    /// on its GPU.
    pub(crate) fn choose_profile(&mut self, profile_id: String) -> Vec<Effect> {
        if self.profile_unavailable(&profile_id).is_some() {
            return Vec::new();
        }
        let Some(gpu_id) = self
            .profile(&profile_id)
            .map(|profile| profile.gpu_id.clone())
        else {
            return Vec::new();
        };
        if self.drafts.by_gpu.contains_key(&gpu_id) {
            self.palette = None;
            self.dialog = Some(DialogState::DiscardForProfile { profile_id });
            return Vec::new();
        }
        self.apply_profile(profile_id, Vec::new())
    }

    /// Answers the open profile dialog: discards the draft and applies the profile, or adjusts the
    /// profile and applies it.
    pub(crate) fn confirm_profile(&mut self) -> Vec<Effect> {
        let (profile_id, corrections) = match self.dialog.take() {
            Some(DialogState::DiscardForProfile { profile_id }) => (profile_id, Vec::new()),
            Some(DialogState::ProfileCorrections {
                profile_id,
                corrections,
            }) => (profile_id, corrections),
            other => {
                self.dialog = other;
                return Vec::new();
            }
        };
        if self.profile_unavailable(&profile_id).is_some() {
            return Vec::new();
        }
        let Some(gpu_id) = self
            .profile(&profile_id)
            .map(|profile| profile.gpu_id.clone())
        else {
            return Vec::new();
        };
        // Sent before the edit hold ends, so automatic switching can't step in between.
        let mut effects = self.apply_profile(profile_id, corrections);
        if self.drafts.by_gpu.remove(&gpu_id).is_some() {
            effects.extend(self.sync_edit_holds());
        }
        effects
    }

    /// Sends the profile's application, adjusted by `corrections` when there are any.
    fn apply_profile(
        &mut self,
        profile_id: String,
        corrections: Vec<ProfileRangeCorrection>,
    ) -> Vec<Effect> {
        let Some(profile) = self.profile(&profile_id) else {
            return Vec::new();
        };
        let gpu_id = profile.gpu_id.clone();
        let name = profile.name.clone();
        let command = self.drafts.next_command();
        let request = if corrections.is_empty() {
            DaemonRequest::ApplyProfileById(ProfileIdRequest {
                profile_id: profile_id.clone(),
            })
        } else {
            DaemonRequest::CorrectAndApplyProfile(CorrectAndApplyProfileRequest {
                profile_id: profile_id.clone(),
                corrections,
            })
        };
        self.profile_outcomes.remove(&gpu_id);
        self.profile_applications.insert(
            gpu_id,
            ProfileApplying {
                command,
                profile_id,
                name,
            },
        );
        vec![Effect::CommandPrismd {
            id: command,
            request,
        }]
    }

    /// Takes in prismd's answer to a profile application, if `command` was one: what it applied
    /// and left, or the corrections it needs reviewed first. Says whether it was one.
    pub(crate) fn profile_answered(
        &mut self,
        command: CommandId,
        response: &DaemonResponse,
    ) -> bool {
        let Some(gpu_id) = self
            .profile_applications
            .iter()
            .find(|(_, applying)| applying.command == command)
            .map(|(gpu_id, _)| gpu_id.clone())
        else {
            return false;
        };
        let Some(applying) = self.profile_applications.remove(&gpu_id) else {
            return false;
        };
        let name = &applying.name;
        let outcome = match response {
            DaemonResponse::ProfileApplication(application)
                if !application.corrections.is_empty() =>
            {
                // Nothing was written: the user decides on every correction at once.
                if self.dialog.is_none() {
                    self.palette = None;
                    self.dialog = Some(DialogState::ProfileCorrections {
                        profile_id: applying.profile_id,
                        corrections: application.corrections.clone(),
                    });
                }
                return true;
            }
            DaemonResponse::ProfileApplication(application) => {
                application_outcome(name, application)
            }
            DaemonResponse::CorrectedProfileApplication(corrected) => {
                application_outcome(name, &corrected.application)
            }
            DaemonResponse::Error(error) => {
                not_applied(&format!("{name} not applied"), Some(&error.message))
            }
            _ => format!("prismd answered applying {name} with something else. Apply it again."),
        };
        self.profile_outcomes.insert(gpu_id, outcome);
        true
    }

    /// The session ended: profile applications it didn't answer may or may not have happened.
    pub(crate) fn profiles_lost_session(&mut self) {
        self.profiles = Default::default();
        for (gpu_id, applying) in std::mem::take(&mut self.profile_applications) {
            self.profile_outcomes.insert(
                gpu_id,
                format!(
                    "The prismd connection ended before applying {} was answered. Check the \
                     values, then apply it again.",
                    applying.name
                ),
            );
        }
    }
}

/// What applying the profile named `name` put into effect and what it left, and why, such as
/// "Quiet applied" or "Quiet applied in part · Power limit not applied: …".
fn application_outcome(name: &str, application: &ProfileApplicationResponse) -> String {
    let mut applied = false;
    let mut left = Vec::new();
    for (key, outcome) in [
        (ProfileSettingKey::PowerLimitWatts, &application.power),
        (ProfileSettingKey::GraphicsOffsetMhz, &application.graphics),
        (ProfileSettingKey::MemoryOffsetMhz, &application.memory),
    ] {
        let Some(outcome) = outcome else { continue };
        if outcome.status == TuningOutcomeStatus::Succeeded {
            applied = true;
        } else {
            left.push(not_applied(
                &format!("{} not applied", setting_name(key)),
                outcome.error.as_deref(),
            ));
        }
    }
    if let Some(cooling) = &application.cooling {
        if cooling.outcome.status == TuningOutcomeStatus::Succeeded {
            applied = true;
        } else {
            left.push(not_applied(
                "Cooling not applied",
                cooling.outcome.error.as_deref(),
            ));
        }
    }
    let summary = match (applied, left.is_empty()) {
        (_, true) => return format!("{name} applied"),
        (true, false) => format!("{name} applied in part"),
        (false, false) => format!("{name} not applied"),
    };
    [summary]
        .into_iter()
        .chain(left)
        .collect::<Vec<_>>()
        .join(" · ")
}

/// A range correction as a review shows it: the setting, and its saved value to the one it would
/// become.
pub(crate) fn correction_change(correction: &ProfileRangeCorrection) -> ReviewChange {
    let (name, change) = match correction {
        ProfileRangeCorrection::Numeric(numeric) => (
            setting_name(numeric.setting),
            format!(
                "{} → {}",
                setting_readout(numeric.setting, Some(numeric.saved_value)).text(),
                setting_readout(numeric.setting, Some(numeric.proposed_value)).text()
            ),
        ),
        ProfileRangeCorrection::Cooling {
            saved_value,
            proposed_value,
            ..
        } => (
            "Cooling",
            format!(
                "{} → {}",
                cooling_text(saved_value),
                cooling_text(proposed_value)
            ),
        ),
    };
    ReviewChange {
        name: name.into(),
        change,
        yours: None,
    }
}
