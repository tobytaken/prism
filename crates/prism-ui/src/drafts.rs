//! Staged tuning: each GPU's draft of unapplied tuning changes, the edit holds they keep, and
//! applying or discarding them.

use std::collections::BTreeMap;

use prism_gpu::{GpuCommandErrorCode, TuningOutcome, TuningOutcomeStatus};
use prism_protocol::{
    ApplyGpuProfileRequest, DaemonRequest, DaemonResponse, ProfileApplicationResponse,
    ProfileSetting, ProfileSettingKey, ProfileSettings, SetEditHoldsRequest,
};

use crate::core::{CommandId, Core, Effect, Selection, TuningControl};

/// A GPU's unapplied tuning changes.
#[derive(Debug, Clone)]
pub(crate) struct Draft {
    /// The GPU's tuning-state revision the changes are made against.
    pub(crate) revision: String,
    pub(crate) settings: ProfileSettings,
    /// Why the last Apply left changes unapplied, until the next edit.
    pub(crate) problem: Option<String>,
}

impl Draft {
    /// How many settings it changes.
    pub(crate) fn len(&self) -> usize {
        let numeric = ProfileSettingKey::ALL
            .into_iter()
            .filter(|key| self.settings.get(*key).is_some())
            .count();
        numeric + usize::from(self.settings.cooling.is_some())
    }
}

/// Every GPU's draft, kept while the user looks at another GPU, and the Applies prismd hasn't
/// answered yet.
#[derive(Debug, Default)]
pub(crate) struct Drafts {
    pub(crate) by_gpu: BTreeMap<String, Draft>,
    /// The Apply waiting for prismd's answer, on each GPU that has one.
    pub(crate) applying: BTreeMap<String, CommandId>,
    /// The GPUs prismd last heard this session holds edits on.
    held: Vec<String>,
    next_command: u64,
}

/// What a setting is called where an Apply's outcome names it.
fn setting_name(key: ProfileSettingKey) -> &'static str {
    match key {
        ProfileSettingKey::PowerLimitWatts => "Power limit",
        ProfileSettingKey::GraphicsOffsetMhz => "Graphics clock offset",
        ProfileSettingKey::MemoryOffsetMhz => "Memory clock offset",
    }
}

impl Core {
    /// The selected GPU's id, while the workspace shows one.
    fn selected_gpu(&self) -> Option<String> {
        match &self.selection {
            Some(Selection::Gpu(id)) => Some(id.clone()),
            _ => None,
        }
    }

    /// Stages `watts` as the selected GPU's power limit, kept within its allowed range. Staging the
    /// value in effect leaves the power limit unchanged.
    pub(crate) fn stage_power_limit(&mut self, watts: i64) -> Vec<Effect> {
        let Some(id) = self.selected_gpu() else {
            return Vec::new();
        };
        let Some(gpu) = self.gpus().iter().find(|gpu| gpu.id == id) else {
            return Vec::new();
        };
        let Some((min, max)) = gpu.power_limit_range() else {
            return Vec::new();
        };
        if !self.editable(&id, TuningControl::PowerLimit) || self.drafts.applying.contains_key(&id)
        {
            return Vec::new();
        }
        let watts = watts.clamp(min, max);
        let in_effect = self.power_limit_in_effect(gpu);
        let revision = self.revision_of(&id);
        let draft = self.drafts.by_gpu.entry(id.clone()).or_insert(Draft {
            revision,
            settings: ProfileSettings::default(),
            problem: None,
        });
        draft.settings.power_limit_watts =
            (Some(watts) != in_effect).then_some(ProfileSetting::Value { value: watts });
        draft.problem = None;
        if draft.len() == 0 {
            self.drafts.by_gpu.remove(&id);
        }
        self.sync_edit_holds()
    }

    /// Stages the power limit drawn at `y` on the selected GPU's power chart, from the top (0) to
    /// the bottom (1).
    pub(crate) fn drag_power_limit(&mut self, y: f32) -> Vec<Effect> {
        let Some(id) = self.selected_gpu() else {
            return Vec::new();
        };
        let Some(gpu) = self.gpus().iter().find(|gpu| gpu.id == id) else {
            return Vec::new();
        };
        let chart = self.power_chart(gpu);
        let watts = chart.max as f32 - y.clamp(0.0, 1.0) * (chart.max - chart.min) as f32;
        self.stage_power_limit(watts.round() as i64)
    }

    /// Sends the selected GPU's draft to prismd as one Apply.
    pub(crate) fn apply_changes(&mut self) -> Vec<Effect> {
        let Some(id) = self.selected_gpu() else {
            return Vec::new();
        };
        let Some(draft) = self.drafts.by_gpu.get(&id) else {
            return Vec::new();
        };
        if self.drafts.applying.contains_key(&id) || self.tuning_access().reason().is_some() {
            return Vec::new();
        }
        self.drafts.next_command += 1;
        let command = CommandId(self.drafts.next_command);
        let request = DaemonRequest::ApplyProfile(ApplyGpuProfileRequest {
            gpu_id: id.clone(),
            settings: draft.settings.clone(),
            expected_revision: draft.revision.clone(),
            fan_indexes: None,
        });
        self.drafts.applying.insert(id, command);
        vec![Effect::CommandPrismd {
            id: command,
            request,
        }]
    }

    /// Drops the selected GPU's draft, which releases its edit hold.
    pub(crate) fn discard_changes(&mut self) -> Vec<Effect> {
        let Some(id) = self.selected_gpu() else {
            return Vec::new();
        };
        if self.drafts.applying.contains_key(&id) || self.drafts.by_gpu.remove(&id).is_none() {
            return Vec::new();
        }
        self.sync_edit_holds()
    }

    /// Takes in prismd's answer to a command: an Apply's outcome decides which changes stay
    /// unapplied, and why. The values in effect come only from prismd's events.
    pub(crate) fn answered(&mut self, command: CommandId, response: DaemonResponse) -> Vec<Effect> {
        let Some(id) = self
            .drafts
            .applying
            .iter()
            .find(|(_, applying)| **applying == command)
            .map(|(id, _)| id.clone())
        else {
            return Vec::new();
        };
        self.drafts.applying.remove(&id);
        let latest = self.revision_of(&id);
        let Some(draft) = self.drafts.by_gpu.get_mut(&id) else {
            return Vec::new();
        };
        // The changes left stay made against the state they started from: an answer can overtake
        // the event its own write pushed, so only a refusal for review moves them, onto the state
        // the user has now been shown.
        draft.problem = match response {
            DaemonResponse::ProfileApplication(application) => {
                unapplied(&mut draft.settings, &application)
            }
            DaemonResponse::Error(error) => {
                if error.code == GpuCommandErrorCode::StaleRevision {
                    draft.revision = latest;
                }
                Some(error.message)
            }
            _ => Some("prismd answered the Apply with something else. Apply again.".into()),
        };
        if draft.len() == 0 {
            self.drafts.by_gpu.remove(&id);
        }
        self.sync_edit_holds()
    }

    /// The session ended: Applies it didn't answer stay unapplied, and prismd let go of every
    /// edit hold.
    pub(crate) fn drafts_lost_session(&mut self) {
        for id in std::mem::take(&mut self.drafts.applying).into_keys() {
            if let Some(draft) = self.drafts.by_gpu.get_mut(&id) {
                draft.problem = Some(
                    "The prismd connection ended before the Apply was answered. Check the values, \
                     then apply again."
                        .into(),
                );
            }
        }
        self.drafts.held.clear();
    }

    /// Tells prismd which GPUs have drafts, when that changed or it hasn't heard yet this session.
    pub(crate) fn sync_edit_holds(&mut self) -> Vec<Effect> {
        let gpu_ids: Vec<_> = self.drafts.by_gpu.keys().cloned().collect();
        if gpu_ids == self.drafts.held || !self.connected() {
            return Vec::new();
        }
        self.drafts.held = gpu_ids.clone();
        vec![Effect::SendToPrismd(DaemonRequest::SetEditHolds(
            SetEditHoldsRequest { gpu_ids },
        ))]
    }

    /// The GPU's tuning-state revision as prismd last pushed it, or none without one.
    fn revision_of(&self, gpu_id: &str) -> String {
        self.tuning
            .get(gpu_id)
            .map(|state| state.revision.clone())
            .unwrap_or_default()
    }

    /// The selected GPU's draft, if it has one.
    pub(crate) fn selected_draft(&self) -> Option<&Draft> {
        self.drafts.by_gpu.get(&self.selected_gpu()?)
    }

    /// Whether the selected GPU's Apply is waiting for prismd's answer.
    pub(crate) fn selected_applying(&self) -> bool {
        self.selected_gpu()
            .is_some_and(|id| self.drafts.applying.contains_key(&id))
    }

    /// Whether any GPU has unapplied tuning changes, which put it under an edit hold.
    pub fn edit_hold(&self) -> bool {
        !self.drafts.by_gpu.is_empty()
    }
}

/// Removes the settings the application put into effect, and says why the others weren't.
fn unapplied(
    settings: &mut ProfileSettings,
    application: &ProfileApplicationResponse,
) -> Option<String> {
    let succeeded = |outcome: &Option<TuningOutcome>| {
        outcome
            .as_ref()
            .is_some_and(|outcome| outcome.status == TuningOutcomeStatus::Succeeded)
    };
    let mut problems = Vec::new();
    for (key, outcome) in [
        (ProfileSettingKey::PowerLimitWatts, &application.power),
        (ProfileSettingKey::GraphicsOffsetMhz, &application.graphics),
        (ProfileSettingKey::MemoryOffsetMhz, &application.memory),
    ] {
        if settings.get(key).is_none() {
            continue;
        }
        if succeeded(outcome) {
            settings.clear(key);
            continue;
        }
        let why = outcome
            .as_ref()
            .and_then(|outcome| outcome.error.clone())
            .unwrap_or_else(|| "not applied".into());
        problems.push(format!("{}: {why}", setting_name(key)));
    }
    if let Some(cooling) = &application.cooling
        && settings.cooling.is_some()
    {
        if cooling.outcome.status == TuningOutcomeStatus::Succeeded {
            settings.cooling = None;
        } else {
            let why = cooling.outcome.error.as_deref().unwrap_or("not applied");
            problems.push(format!("Cooling: {why}"));
        }
    }
    (!problems.is_empty()).then(|| problems.join("\n"))
}
