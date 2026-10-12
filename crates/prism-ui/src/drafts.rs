//! Staged tuning: each GPU's draft of unapplied tuning changes, the edit holds they keep, and
//! applying or discarding them, with the review a stale Apply opens and the confirmation closing
//! with drafts asks for.

use std::collections::BTreeMap;

use prism_gpu::cooling::CoolingConfiguration;
use prism_gpu::{GpuCommandErrorCode, TuningOutcome, TuningOutcomeStatus};
use prism_protocol::{
    ApplyGpuProfileRequest, DaemonRequest, DaemonResponse, GpuTuningState,
    ProfileApplicationResponse, ProfileRangeCorrection, ProfileSetting, ProfileSettingKey,
    ProfileSettings, ReportedTuningValues, SetEditHoldsRequest,
};

use crate::cooling::{cooling_read, cooling_text};
use crate::core::{CommandId, Core, Effect, Gpu, Latest, Selection, TuningControl};
use crate::format::Readout;
use crate::games::SteamListing;

/// A GPU's unapplied tuning changes.
#[derive(Debug, Clone)]
pub(crate) struct Draft {
    /// The GPU's tuning-state revision the changes are made against.
    pub(crate) revision: String,
    /// The current tuning values at that revision, which a review compares with.
    base: Option<ReportedTuningValues>,
    pub(crate) settings: ProfileSettings,
    /// The GPU's fans as the user saw them when staging cooling, which prismd checks before it
    /// writes the cooling setting.
    fan_indexes: Vec<u32>,
    /// The cooling in effect as prismd reported it when the draft started staging cooling, or
    /// last moved onto a tuning state: a change from it is one the user hasn't reviewed.
    cooling_seen: Option<CoolingConfiguration>,
    /// Each staged setting the last Apply didn't put into effect, with why if prismd said, until
    /// it's staged again.
    pub(crate) failed: BTreeMap<Staged, Option<String>>,
    /// What the last Apply put into effect and what it didn't, and why, until the next edit.
    pub(crate) outcome: Option<String>,
    /// How the draft moves onto the next tuning state prismd pushes for its GPU, while it waits
    /// for one.
    follow: Option<Follow>,
}

/// Something a draft stages: a profile setting, or the cooling configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Staged {
    Setting(ProfileSettingKey),
    Cooling,
}

/// Why a draft waits for the next tuning state prismd pushes for its GPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Follow {
    /// An Apply put some of its settings into effect, and the state that write pushed hasn't
    /// arrived yet: the changes left are made against it.
    Quietly,
    /// prismd refused an Apply for review before the change that made it stale reached Prism: the
    /// review opens once it does.
    WithReview,
}

/// An Apply waiting for prismd's answer.
#[derive(Debug, Clone)]
pub(crate) struct Applying {
    command: CommandId,
    /// The GPU's tuning-state revision Prism had last heard of when it sent the Apply.
    seen: String,
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
    pub(crate) applying: BTreeMap<String, Applying>,
    /// The GPUs prismd last heard this session holds edits on.
    held: Vec<String>,
    next_command: u64,
}

impl Drafts {
    /// A new id for a command sent to prismd, for telling its answer apart.
    pub(crate) fn next_command(&mut self) -> CommandId {
        self.next_command += 1;
        CommandId(self.next_command)
    }
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
    /// Applying the profile would discard the unapplied tuning changes on its GPU: discard them
    /// and apply it, or keep editing.
    DiscardForProfile { profile_id: String },
    /// Applying the profile needs every one of these range corrections: adjust the profile and
    /// apply it, or cancel, which changes nothing.
    ProfileCorrections {
        profile_id: String,
        corrections: Vec<ProfileRangeCorrection>,
    },
    /// Naming a profile: saving the GPU's applied values as a new one, or renaming one. `error`
    /// is why prismd refused the name last sent, until it's edited; `sending` the command prismd
    /// hasn't answered yet; and `managing` the GPU whose Manage profiles it was opened from, which
    /// it goes back to.
    ProfileName {
        naming: Naming,
        name: String,
        error: Option<String>,
        sending: Option<CommandId>,
        managing: Option<String>,
    },
    /// Deleting the profile, once the user confirms it.
    DeleteProfile {
        profile_id: String,
        error: Option<String>,
        sending: Option<CommandId>,
        managing: Option<String>,
    },
    /// The GPU's profiles, to rename or delete any of them.
    ManageProfiles { gpu_id: String },
    /// The caller's Steam apps, searched by `filter`, to add one as a game. `adding` is the app,
    /// by id, being added under the command prismd hasn't answered yet, and `error` why prismd
    /// refused the last one.
    AddSteamGame {
        listing: SteamListing,
        filter: String,
        adding: Option<(CommandId, u32)>,
        error: Option<String>,
    },
}

/// What a profile name dialog names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Naming {
    /// A new profile of the GPU's applied values.
    SaveAsNew { gpu_id: String },
    /// The profile, by id.
    Rename { profile_id: String },
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
pub(crate) fn reported(values: &ReportedTuningValues, key: ProfileSettingKey) -> Option<i64> {
    match key {
        ProfileSettingKey::PowerLimitWatts => values.power_limit_watts,
        ProfileSettingKey::GraphicsOffsetMhz => values.graphics_offset_mhz,
        ProfileSettingKey::MemoryOffsetMhz => values.memory_offset_mhz,
    }
}

impl Core {
    /// The selected GPU's id, while the workspace shows one.
    pub(crate) fn selected_gpu(&self) -> Option<String> {
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
        let Some(gpu) = self.stageable(control_of(key)) else {
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
        let Some(gpu) = self.stageable(control_of(key)) else {
            return Vec::new();
        };
        let id = gpu.id.clone();
        self.put(id, key, Some(ProfileSetting::Default {}))
    }

    /// The selected GPU, while its control can be staged: with tuning access, the GPU control
    /// capability for it and no Apply on its way.
    pub(crate) fn stageable(&self, control: TuningControl) -> Option<&Gpu> {
        let id = self.selected_gpu()?;
        let gpu = self.gpus().iter().find(|gpu| gpu.id == id)?;
        (self.editable(&id, control) && !self.drafts.applying.contains_key(&id)).then_some(gpu)
    }

    /// Stages `setting` for the GPU's `key`, or none to leave it unchanged.
    fn put(
        &mut self,
        id: String,
        key: ProfileSettingKey,
        setting: Option<ProfileSetting>,
    ) -> Vec<Effect> {
        self.edit_draft(id, Staged::Setting(key), |draft| match setting {
            Some(setting) => draft.settings.set(key, setting),
            None => draft.settings.clear(key),
        })
    }

    /// Stages `configuration` for the GPU's fans as they are now, or none to leave cooling
    /// unchanged.
    pub(crate) fn put_cooling(
        &mut self,
        id: String,
        configuration: Option<CoolingConfiguration>,
    ) -> Vec<Effect> {
        let fan_indexes = self
            .gpus()
            .iter()
            .find(|gpu| gpu.id == id)
            .and_then(cooling_read)
            .map(|cooling| cooling.fans.iter().map(|fan| fan.index).collect())
            .unwrap_or_default();
        let reported = self
            .tuning
            .get(&id)
            .and_then(|state| state.reported.cooling.clone());
        self.edit_draft(id, Staged::Cooling, |draft| {
            if draft.settings.cooling.is_none() {
                draft.cooling_seen = reported;
            }
            draft.settings.cooling = configuration;
            draft.fan_indexes = fan_indexes;
        })
    }

    /// Changes what the GPU's draft stages for `staged` with `edit`, starting a draft against the
    /// GPU's tuning state if it has none, and dropping it once it stages nothing.
    fn edit_draft(
        &mut self,
        id: String,
        staged: Staged,
        edit: impl FnOnce(&mut Draft),
    ) -> Vec<Effect> {
        // What a profile application said no longer holds once the GPU is edited again.
        self.profile_outcomes.remove(&id);
        let (revision, base) = match self.tuning.get(&id) {
            Some(state) => (state.revision.clone(), Some(state.reported.clone())),
            None => (String::new(), None),
        };
        let draft = self.drafts.by_gpu.entry(id.clone()).or_insert(Draft {
            revision,
            base,
            settings: ProfileSettings::default(),
            fan_indexes: Vec::new(),
            cooling_seen: None,
            failed: BTreeMap::new(),
            outcome: None,
            follow: None,
        });
        edit(draft);
        draft.failed.remove(&staged);
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
        if !self.drafts.by_gpu.contains_key(&id) {
            return Vec::new();
        }
        if self.drafts.applying.contains_key(&id)
            || self.tuning_access().reason().is_some()
            || !self.curve_problems(&id).is_empty()
        {
            return Vec::new();
        }
        // Cooling changed under the draft replaces that change only once the user saw it: Apply
        // opens the review, and Apply in the review moves the draft onto the change.
        if self.cooling_changed_under(&id) {
            let reviewing =
                matches!(&self.dialog, Some(DialogState::Review { gpu_id, .. }) if *gpu_id == id);
            let Some(state) = self.tuning.get(&id).cloned() else {
                return Vec::new();
            };
            let Some(draft) = self.drafts.by_gpu.get_mut(&id) else {
                return Vec::new();
            };
            if !reviewing {
                let changes =
                    review_changes(draft.base.as_ref(), Some(&state.reported), &draft.settings);
                self.open_review(id, changes);
                return Vec::new();
            }
            follow(draft, &state);
        }
        let command = self.drafts.next_command();
        let Some(draft) = self.drafts.by_gpu.get(&id) else {
            return Vec::new();
        };
        let request = DaemonRequest::ApplyProfile(ApplyGpuProfileRequest {
            gpu_id: id.clone(),
            settings: draft.settings.clone(),
            expected_revision: draft.revision.clone(),
            fan_indexes: draft
                .settings
                .cooling
                .is_some()
                .then(|| draft.fan_indexes.clone()),
        });
        let seen = self.revision_of(&id);
        self.drafts.applying.insert(id, Applying { command, seen });
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
        if self.profile_answered(command, &response)
            || self.profile_command_answered(command, &response)
            || self.game_answered(command, &response)
        {
            return Vec::new();
        }
        let Some(id) = self
            .drafts
            .applying
            .iter()
            .find(|(_, applying)| applying.command == command)
            .map(|(id, _)| id.clone())
        else {
            return Vec::new();
        };
        let Some(applying) = self.drafts.applying.remove(&id) else {
            return Vec::new();
        };
        let latest = self.tuning.get(&id).cloned();
        let Some(draft) = self.drafts.by_gpu.get_mut(&id) else {
            return Vec::new();
        };
        // An answer can overtake the event its own write pushed, or the one that made it stale:
        // the changes left move onto a state only once Prism has heard of it.
        let heard_since = latest
            .as_ref()
            .is_some_and(|state| state.revision != applying.seen);
        let mut review = None;
        match response {
            DaemonResponse::ProfileApplication(application) => {
                // A write that changed nothing leaves the state the changes are made against.
                if settle(draft, &application) {
                    match &latest {
                        Some(state) if heard_since => {
                            follow(draft, state);
                        }
                        _ => draft.follow = Some(Follow::Quietly),
                    }
                }
            }
            DaemonResponse::Error(error) => {
                if error.code == GpuCommandErrorCode::StaleRevision {
                    match &latest {
                        Some(state) if state.revision != draft.revision => {
                            review = Some(follow(draft, state));
                        }
                        _ => draft.follow = Some(Follow::WithReview),
                    }
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
        if let Some(changes) = review {
            self.open_review(id, changes);
        }
        self.sync_edit_holds()
    }

    /// prismd pushed the GPU's tuning state: a draft waiting for it moves onto it, opening the
    /// review it waited to open.
    pub(crate) fn tuning_pushed(&mut self, gpu_id: &str) {
        let Some(state) = self.tuning.get(gpu_id).cloned() else {
            return;
        };
        let competing = self.cooling_changed_under(gpu_id);
        let Some(draft) = self.drafts.by_gpu.get_mut(gpu_id) else {
            return;
        };
        // An open review of cooling changed under the draft shows the latest change.
        if competing
            && let Some(DialogState::Review {
                gpu_id: reviewed,
                changes,
            }) = &mut self.dialog
            && reviewed == gpu_id
        {
            *changes = review_changes(draft.base.as_ref(), Some(&state.reported), &draft.settings);
        }
        let Some(how) = draft.follow else {
            return;
        };
        let changes = follow(draft, &state);
        if how == Follow::WithReview {
            self.open_review(gpu_id.to_owned(), changes);
        }
    }

    /// Opens the review of what changed under the GPU's draft, unless a dialog the user hasn't
    /// answered is open; the strip still says why.
    fn open_review(&mut self, gpu_id: String, changes: Vec<ReviewChange>) {
        if self.dialog.is_none() {
            self.open_dialog(DialogState::Review { gpu_id, changes });
        }
    }

    /// The GPU's tuning-state revision as prismd last pushed it, or none without one.
    pub(crate) fn revision_of(&self, gpu_id: &str) -> String {
        self.tuning
            .get(gpu_id)
            .map(|state| state.revision.clone())
            .unwrap_or_default()
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
    pub(crate) fn close_or_ask(&mut self) -> Vec<Effect> {
        if self.drafts.by_gpu.is_empty() {
            return self.quit();
        }
        self.open_dialog(DialogState::ConfirmClose);
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

    /// The first thing keeping the fan curve the draft Apply acts on stages from being applied,
    /// if it stages one that can't be.
    pub(crate) fn changes_curve_problem(&self) -> Option<String> {
        let id = self.changes_gpu()?;
        self.curve_problems(&id).into_iter().next()
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

    /// Whether the cooling in effect on the GPU changed under the cooling its draft stages, such as
    /// in another session: the draft replaces that change only once the user reviewed it (ADR
    /// 0006). A curve's changing output isn't a change to the cooling in effect, cooling prismd
    /// can't report, such as after a cooling fault, can't be compared, and while an Apply waits
    /// for prismd's answer, any change is its own write.
    pub(crate) fn cooling_changed_under(&self, gpu_id: &str) -> bool {
        let Some(draft) = self.drafts.by_gpu.get(gpu_id) else {
            return false;
        };
        if draft.settings.cooling.is_none() || draft.follow.is_some() || self.applying(gpu_id) {
            return false;
        }
        let now = self
            .tuning
            .get(gpu_id)
            .and_then(|state| state.reported.cooling.as_ref());
        match (&draft.cooling_seen, now) {
            (Some(seen), Some(now)) => seen != now,
            _ => false,
        }
    }

    /// Whether any GPU has unapplied tuning changes, which put it under an edit hold.
    pub fn edit_hold(&self) -> bool {
        !self.drafts.by_gpu.is_empty()
    }
}

/// Moves the draft onto `state`, returning the current tuning values that changed under it.
fn follow(draft: &mut Draft, state: &GpuTuningState) -> Vec<ReviewChange> {
    let changes = review_changes(draft.base.as_ref(), Some(&state.reported), &draft.settings);
    draft.revision = state.revision.clone();
    draft.base = Some(state.reported.clone());
    draft.cooling_seen = state.reported.cooling.clone();
    draft.follow = None;
    changes
}

/// Removes the settings the application put into effect, marks the ones it didn't, and words
/// both in the draft's outcome. Says whether it put any into effect.
fn settle(draft: &mut Draft, application: &ProfileApplicationResponse) -> bool {
    let mut outcome = Vec::new();
    let mut applied = false;
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
            applied = true;
            continue;
        }
        let why = result.as_ref().and_then(|result| result.error.clone());
        outcome.push(not_applied(&format!("{name} not applied"), why.as_deref()));
        draft.failed.insert(Staged::Setting(key), why);
    }
    if let Some(cooling) = &application.cooling
        && draft.settings.cooling.is_some()
    {
        if cooling.outcome.status == TuningOutcomeStatus::Succeeded {
            draft.settings.cooling = None;
            outcome.push("Cooling applied".into());
            applied = true;
        } else {
            // A fan's own error says why when the whole application doesn't.
            let why = cooling.outcome.error.clone().or_else(|| {
                cooling
                    .fans
                    .iter()
                    .find_map(|fan| fan.outcome.error.clone())
            });
            outcome.push(not_applied("Cooling not applied", why.as_deref()));
            draft.failed.insert(Staged::Cooling, why);
            // Fans that took their setting changed the cooling in effect all the same.
            applied |= cooling
                .fans
                .iter()
                .any(|fan| fan.outcome.status == TuningOutcomeStatus::Succeeded);
        }
    }
    // Only a partial Apply leaves a draft to say this on.
    draft.outcome = Some(outcome.join(" · "));
    applied
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
    fn cooling(values: Option<&ReportedTuningValues>) -> Option<&CoolingConfiguration> {
        values?.cooling.as_ref()
    }
    if cooling(base) != cooling(now) {
        let text = |cooling: Option<&CoolingConfiguration>| {
            cooling.map_or_else(|| "unknown".into(), cooling_text)
        };
        changes.push(ReviewChange {
            name: "Cooling".into(),
            change: format!("{} → {}", text(cooling(base)), text(cooling(now))),
            yours: settings.cooling.as_ref().map(cooling_text),
        });
    }
    changes
}
