//! Staged tuning: each GPU's draft of unapplied tuning changes, the edit holds they keep, and
//! applying or discarding them, with the review a stale Apply opens and the confirmation closing
//! with drafts asks for.

use std::collections::BTreeMap;

use prism_gpu::{GpuCommandErrorCode, TuningOutcome, TuningOutcomeStatus};
use prism_protocol::{
    ApplyGpuProfileRequest, DaemonRequest, DaemonResponse, ProfileApplicationResponse,
    ProfileSetting, ProfileSettingKey, ProfileSettings, ReportedTuningValues, SetEditHoldsRequest,
};

use crate::core::{CommandId, Core, Effect, Gpu, Latest, Selection, TuningControl};
use crate::format::Readout;

/// A GPU's unapplied tuning changes.
#[derive(Debug, Clone)]
pub(crate) struct Draft {
    /// The GPU's tuning-state revision the changes are made against.
    pub(crate) revision: String,
    /// The current tuning values at that revision, which a review compares with.
    base: Option<ReportedTuningValues>,
    pub(crate) settings: ProfileSettings,
    /// Each staged setting the last Apply didn't put into effect, with why if prismd said, until
    /// it's staged again.
    pub(crate) failed: BTreeMap<ProfileSettingKey, Option<String>>,
    /// What the last Apply put into effect and what it didn't, and why, until the next edit.
    pub(crate) outcome: Option<String>,
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

/// A dialog over the whole window, which takes the keyboard until it's answered or dismissed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DialogState {
    /// prismd refused the GPU's Apply because its tuning state changed: what changed, to apply the
    /// draft again over it or discard it.
    Review {
        gpu_id: String,
        changes: Vec<ReviewChange>,
    },
    /// Closing with unapplied tuning changes: discard them and close, or keep editing.
    ConfirmClose,
}

/// A current tuning value that changed under a draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewChange {
    /// The setting, such as "Power limit".
    pub name: String,
    /// From what to what, such as "575 W → 500 W".
    pub change: String,
    /// What the draft stages for it, such as "450 W" or "default", if it stages anything.
    pub yours: Option<String>,
}

/// What a setting is called, such as where an Apply's outcome names it.
pub(crate) fn setting_name(key: ProfileSettingKey) -> &'static str {
    match key {
        ProfileSettingKey::PowerLimitWatts => "Power limit",
        ProfileSettingKey::GraphicsOffsetMhz => "Graphics clock offset",
        ProfileSettingKey::MemoryOffsetMhz => "Memory clock offset",
    }
}

/// The control that tunes a setting.
pub(crate) fn control_of(key: ProfileSettingKey) -> TuningControl {
    match key {
        ProfileSettingKey::PowerLimitWatts => TuningControl::PowerLimit,
        ProfileSettingKey::GraphicsOffsetMhz => TuningControl::GraphicsClockOffset,
        ProfileSettingKey::MemoryOffsetMhz => TuningControl::MemoryClockOffset,
    }
}

/// A setting's value in its unit, such as "450 W" or "+150 MHz".
pub(crate) fn setting_readout(key: ProfileSettingKey, value: Option<i64>) -> Readout {
    match key {
        ProfileSettingKey::PowerLimitWatts => {
            Readout::watts(value.and_then(|value| value.try_into().ok()))
        }
        ProfileSettingKey::GraphicsOffsetMhz | ProfileSettingKey::MemoryOffsetMhz => {
            Readout::offset_megahertz(value.and_then(|value| value.try_into().ok()))
        }
    }
}

/// A staged setting as shown, such as "450 W" or "default".
pub(crate) fn staged_text(key: ProfileSettingKey, setting: ProfileSetting) -> String {
    match setting {
        ProfileSetting::Value { value } => setting_readout(key, Some(value)).text(),
        ProfileSetting::Default {} => "default".into(),
    }
}

/// A setting's current tuning value as prismd reported it.
fn reported(values: &ReportedTuningValues, key: ProfileSettingKey) -> Option<i64> {
    match key {
        ProfileSettingKey::PowerLimitWatts => values.power_limit_watts,
        ProfileSettingKey::GraphicsOffsetMhz => values.graphics_offset_mhz,
        ProfileSettingKey::MemoryOffsetMhz => values.memory_offset_mhz,
    }
}

impl Gpu {
    /// The values the GPU allows for a setting, as its latest reading or else its inventory says.
    pub(crate) fn range(&self, key: ProfileSettingKey) -> Option<(i64, i64)> {
        let device = match &self.latest {
            Latest::Read(device) => Some(device),
            Latest::Waiting | Latest::Missing { .. } => None,
        };
        let control = &self.device.control;
        let (min, max) = match key {
            ProfileSettingKey::PowerLimitWatts => {
                let power = device.map(|device| &device.power);
                (
                    power
                        .and_then(|power| power.min_limit_watts)
                        .or(control.power_limit.min_watts)
                        .map(i64::from),
                    power
                        .and_then(|power| power.max_limit_watts)
                        .or(control.power_limit.max_watts)
                        .map(i64::from),
                )
            }
            ProfileSettingKey::GraphicsOffsetMhz => {
                let offset = device.map(|device| &device.clocks.graphics);
                (
                    offset
                        .and_then(|offset| offset.min_offset_mhz)
                        .or(control.offsets.min_graphics_mhz)
                        .map(i64::from),
                    offset
                        .and_then(|offset| offset.max_offset_mhz)
                        .or(control.offsets.max_graphics_mhz)
                        .map(i64::from),
                )
            }
            ProfileSettingKey::MemoryOffsetMhz => {
                let offset = device.map(|device| &device.clocks.memory);
                (
                    offset
                        .and_then(|offset| offset.min_offset_mhz)
                        .or(control.offsets.min_memory_mhz)
                        .map(i64::from),
                    offset
                        .and_then(|offset| offset.max_offset_mhz)
                        .or(control.offsets.max_memory_mhz)
                        .map(i64::from),
                )
            }
        };
        let (min, max) = (min?, max?);
        (min <= max).then_some((min, max))
    }

    /// What a default instruction for the setting resolves to, as far as Prism knows: the GPU's
    /// reported default power limit, or no offset (ADR 0001).
    pub(crate) fn default_of(&self, key: ProfileSettingKey) -> Option<i64> {
        match key {
            ProfileSettingKey::PowerLimitWatts => match &self.latest {
                Latest::Read(device) => device.power.default_limit_watts,
                Latest::Waiting | Latest::Missing { .. } => None,
            }
            .or(self.device.control.power_limit.default_watts)
            .map(i64::from),
            ProfileSettingKey::GraphicsOffsetMhz | ProfileSettingKey::MemoryOffsetMhz => Some(0),
        }
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

    /// The GPU whose draft Apply and Discard act on: the reviewed one while a review is open, or
    /// else the selected one.
    fn changes_gpu(&self) -> Option<String> {
        match &self.dialog {
            Some(DialogState::Review { gpu_id, .. }) => Some(gpu_id.clone()),
            _ => self.selected_gpu(),
        }
    }

    /// The setting's current tuning value on the GPU: as prismd last pushed its tuning state, or
    /// else as the latest reading says.
    pub(crate) fn in_effect(&self, gpu: &Gpu, key: ProfileSettingKey) -> Option<i64> {
        self.tuning
            .get(&gpu.id)
            .and_then(|state| reported(&state.reported, key))
            .or(match &gpu.latest {
                Latest::Read(device) => match key {
                    ProfileSettingKey::PowerLimitWatts => {
                        device.power.current_limit_watts.map(i64::from)
                    }
                    ProfileSettingKey::GraphicsOffsetMhz => {
                        device.clocks.graphics.current_offset_mhz.map(i64::from)
                    }
                    ProfileSettingKey::MemoryOffsetMhz => {
                        device.clocks.memory.current_offset_mhz.map(i64::from)
                    }
                },
                Latest::Waiting | Latest::Missing { .. } => None,
            })
    }

    /// Stages `value` for the selected GPU's setting, kept within its allowed range. Staging the
    /// value in effect leaves the setting unchanged.
    pub(crate) fn stage(&mut self, key: ProfileSettingKey, value: i64) -> Vec<Effect> {
        let Some(gpu) = self.stageable(key) else {
            return Vec::new();
        };
        let Some((min, max)) = gpu.range(key) else {
            return Vec::new();
        };
        let value = value.clamp(min, max);
        let setting =
            (Some(value) != self.in_effect(gpu, key)).then_some(ProfileSetting::Value { value });
        let id = gpu.id.clone();
        self.put(id, key, setting)
    }

    /// Stages an explicit request for the selected GPU's setting's default, which keeps its
    /// meaning even when the value in effect is the default already.
    pub(crate) fn use_default(&mut self, key: ProfileSettingKey) -> Vec<Effect> {
        let Some(gpu) = self.stageable(key) else {
            return Vec::new();
        };
        let id = gpu.id.clone();
        self.put(id, key, Some(ProfileSetting::Default {}))
    }

    /// The selected GPU, while its setting can be staged: with tuning access, the GPU control
    /// capability for it and no Apply on its way.
    fn stageable(&self, key: ProfileSettingKey) -> Option<&Gpu> {
        let id = self.selected_gpu()?;
        let gpu = self.gpus().iter().find(|gpu| gpu.id == id)?;
        (self.editable(&id, control_of(key)) && !self.drafts.applying.contains_key(&id))
            .then_some(gpu)
    }

    /// Stages `setting` for the GPU's `key`, or none to leave it unchanged.
    fn put(
        &mut self,
        id: String,
        key: ProfileSettingKey,
        setting: Option<ProfileSetting>,
    ) -> Vec<Effect> {
        let (revision, base) = match self.tuning.get(&id) {
            Some(state) => (state.revision.clone(), Some(state.reported.clone())),
            None => (String::new(), None),
        };
        let draft = self.drafts.by_gpu.entry(id.clone()).or_insert(Draft {
            revision,
            base,
            settings: ProfileSettings::default(),
            failed: BTreeMap::new(),
            outcome: None,
        });
        match setting {
            Some(setting) => draft.settings.set(key, setting),
            None => draft.settings.clear(key),
        }
        draft.failed.remove(&key);
        draft.outcome = None;
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
        self.stage(ProfileSettingKey::PowerLimitWatts, watts.round() as i64)
    }

    /// Sends the draft to prismd as one Apply, which answers any review of it.
    pub(crate) fn apply_changes(&mut self) -> Vec<Effect> {
        let Some(id) = self.changes_gpu() else {
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
        self.close_review();
        vec![Effect::CommandPrismd {
            id: command,
            request,
        }]
    }

    /// Drops the draft, which releases its edit hold and answers any review of it.
    pub(crate) fn discard_changes(&mut self) -> Vec<Effect> {
        let Some(id) = self.changes_gpu() else {
            return Vec::new();
        };
        if self.drafts.applying.contains_key(&id) || self.drafts.by_gpu.remove(&id).is_none() {
            return Vec::new();
        }
        self.close_review();
        self.sync_edit_holds()
    }

    fn close_review(&mut self) {
        if let Some(DialogState::Review { .. }) = self.dialog {
            self.dialog = None;
        }
    }

    /// Takes in prismd's answer to a command: an Apply's outcome decides which changes stay
    /// unapplied, and why, and a refusal for review opens one. The values in effect come only from
    /// prismd's events.
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
        let latest = self.tuning.get(&id).cloned();
        let Some(draft) = self.drafts.by_gpu.get_mut(&id) else {
            return Vec::new();
        };
        // The changes left stay made against the state they started from: an answer can overtake
        // the event its own write pushed, so only a refusal for review moves them, onto the state
        // the user is now shown.
        let mut review = None;
        match response {
            DaemonResponse::ProfileApplication(application) => settle(draft, &application),
            DaemonResponse::Error(error) => {
                if error.code == GpuCommandErrorCode::StaleRevision {
                    let now = latest.as_ref().map(|state| &state.reported);
                    review = Some(review_changes(draft.base.as_ref(), now, &draft.settings));
                    draft.revision = latest
                        .as_ref()
                        .map(|state| state.revision.clone())
                        .unwrap_or_default();
                    draft.base = now.cloned();
                }
                draft.outcome = Some(error.message);
            }
            _ => {
                draft.outcome =
                    Some("prismd answered the Apply with something else. Apply again.".into());
            }
        }
        if draft.len() == 0 {
            self.drafts.by_gpu.remove(&id);
        }
        // Nothing opens over a dialog the user hasn't answered; the strip still says why.
        if let Some(changes) = review
            && self.dialog.is_none()
        {
            self.palette = None;
            self.dialog = Some(DialogState::Review {
                gpu_id: id,
                changes,
            });
        }
        self.sync_edit_holds()
    }

    /// The session ended: Applies it didn't answer stay unapplied, and prismd let go of every
    /// edit hold.
    pub(crate) fn drafts_lost_session(&mut self) {
        for id in std::mem::take(&mut self.drafts.applying).into_keys() {
            if let Some(draft) = self.drafts.by_gpu.get_mut(&id) {
                draft.outcome = Some(
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

    /// Asks before quitting while any GPU has unapplied tuning changes; quits at once otherwise.
    pub(crate) fn close(&mut self) -> Vec<Effect> {
        if self.drafts.by_gpu.is_empty() {
            return self.quit();
        }
        self.palette = None;
        self.dialog = Some(DialogState::ConfirmClose);
        Vec::new()
    }

    /// How many unapplied tuning changes every GPU has together.
    pub(crate) fn unapplied_count(&self) -> usize {
        self.drafts.by_gpu.values().map(Draft::len).sum()
    }

    /// The selected GPU's draft, if it has one.
    pub(crate) fn selected_draft(&self) -> Option<&Draft> {
        self.drafts.by_gpu.get(&self.selected_gpu()?)
    }

    /// The draft Apply and Discard act on, if there is one.
    pub(crate) fn changes_draft(&self) -> Option<&Draft> {
        self.drafts.by_gpu.get(&self.changes_gpu()?)
    }

    /// Whether the Apply of the draft Apply and Discard act on is waiting for prismd's answer.
    pub(crate) fn changes_applying(&self) -> bool {
        self.changes_gpu()
            .is_some_and(|id| self.drafts.applying.contains_key(&id))
    }

    /// Whether the GPU's Apply is waiting for prismd's answer.
    pub(crate) fn applying(&self, gpu_id: &str) -> bool {
        self.drafts.applying.contains_key(gpu_id)
    }

    /// Whether any GPU has unapplied tuning changes, which put it under an edit hold.
    pub fn edit_hold(&self) -> bool {
        !self.drafts.by_gpu.is_empty()
    }
}

/// Removes the settings the application put into effect, marks the ones it didn't, and words
/// both in the draft's outcome.
fn settle(draft: &mut Draft, application: &ProfileApplicationResponse) {
    let mut outcome = Vec::new();
    for (key, result) in [
        (ProfileSettingKey::PowerLimitWatts, &application.power),
        (ProfileSettingKey::GraphicsOffsetMhz, &application.graphics),
        (ProfileSettingKey::MemoryOffsetMhz, &application.memory),
    ] {
        if draft.settings.get(key).is_none() {
            continue;
        }
        let name = setting_name(key);
        if succeeded(result) {
            draft.settings.clear(key);
            outcome.push(format!("{name} applied"));
            continue;
        }
        let why = result.as_ref().and_then(|result| result.error.clone());
        outcome.push(not_applied(&format!("{name} not applied"), why.as_deref()));
        draft.failed.insert(key, why);
    }
    if let Some(cooling) = &application.cooling
        && draft.settings.cooling.is_some()
    {
        if cooling.outcome.status == TuningOutcomeStatus::Succeeded {
            draft.settings.cooling = None;
            outcome.push("Cooling applied".into());
        } else {
            outcome.push(not_applied(
                "Cooling not applied",
                cooling.outcome.error.as_deref(),
            ));
        }
    }
    // Only a partial Apply leaves a draft to say this on.
    draft.outcome = Some(outcome.join(" · "));
}

fn succeeded(outcome: &Option<TuningOutcome>) -> bool {
    outcome
        .as_ref()
        .is_some_and(|outcome| outcome.status == TuningOutcomeStatus::Succeeded)
}

/// That `what` wasn't applied, and why if prismd said, such as "Power limit not applied: …".
pub(crate) fn not_applied(what: &str, why: Option<&str>) -> String {
    match why {
        Some(why) => format!("{what}: {why}"),
        None => what.into(),
    }
}

/// The current tuning values that changed from `base` to `now`, with what the draft stages for
/// each.
fn review_changes(
    base: Option<&ReportedTuningValues>,
    now: Option<&ReportedTuningValues>,
    settings: &ProfileSettings,
) -> Vec<ReviewChange> {
    let mut changes: Vec<_> = ProfileSettingKey::ALL
        .into_iter()
        .filter_map(|key| {
            let was = base.and_then(|values| reported(values, key));
            let is = now.and_then(|values| reported(values, key));
            (was != is).then(|| ReviewChange {
                name: setting_name(key).into(),
                change: format!(
                    "{} → {}",
                    setting_readout(key, was).text(),
                    setting_readout(key, is).text()
                ),
                yours: settings.get(key).map(|setting| staged_text(key, setting)),
            })
        })
        .collect();
    let cooling = |values: Option<&ReportedTuningValues>| values.and_then(|v| v.cooling.clone());
    if cooling(base) != cooling(now) {
        changes.push(ReviewChange {
            name: "Cooling".into(),
            change: "Changed".into(),
            yours: settings.cooling.as_ref().map(|_| "changed".into()),
        });
    }
    changes
}
