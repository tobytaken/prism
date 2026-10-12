//! Managing GPU tuning profiles from the workspace header's `⋯` menu and the palette: saving the
//! GPU's applied values to its current profile or as a new one, renaming, deleting after a
//! confirmation, Manage profiles and the startup profile. Saving is blocked while what it would
//! capture isn't known or isn't applied yet, and names are unique per GPU regardless of letter
//! case (ADR 0001). Profiles change only as prismd pushes them; answers only say how a change went.

use prism_gpu::ControlSupport;
use prism_protocol::{
    DaemonCommandResponse, DaemonRequest, DaemonResponse, GpuTuningProfile, ProfileIdRequest,
    ProfileSetting, ProfileSettingKey, RenameProfileRequest, SaveProfileAsNewRequest,
    SaveToCurrentProfileRequest, SetStartupProfileRequest,
};

use crate::core::{CommandId, Core, Effect, Latest};
use crate::drafts::{DialogState, Naming, reported, setting_name};
use crate::palette::Command;
use crate::view::{
    ConfirmDelete, Dialog, ManageProfiles, ManagedProfile, MenuEntry, ProfileMenu,
    ProfileNameDialog, gpu_label,
};

/// A profile change sent to prismd, waiting for its answer.
#[derive(Debug, Clone)]
pub(crate) struct ProfileCommand {
    /// The GPU whose change strip says how it went, when no dialog does.
    gpu_id: String,
    change: ProfileChange,
}

/// What a profile change does, with the profile's name as the user knew it then.
#[derive(Debug, Clone)]
enum ProfileChange {
    SaveTo(String),
    SaveAs(String),
    Rename(String),
    Delete(String),
    Startup(String),
}

impl ProfileChange {
    fn saves(&self) -> bool {
        matches!(self, Self::SaveTo(_) | Self::SaveAs(_))
    }

    /// What it does, as a sentence goes on, such as "saving to Quiet", and how to try it again.
    fn doing(&self) -> (String, &'static str) {
        match self {
            Self::SaveTo(name) => (format!("saving to {name}"), "save again"),
            Self::SaveAs(name) => (format!("saving {name}"), "save again"),
            Self::Rename(name) => (format!("renaming {name}"), "rename it again"),
            Self::Delete(name) => (format!("deleting {name}"), "delete it again"),
            Self::Startup(name) => (
                format!("making {name} the startup profile"),
                "choose it again",
            ),
        }
    }

    /// What the change strip says once prismd answered, if anything: how a save went, or why
    /// another change was refused when no dialog said so.
    fn outcome(&self, refusal: Option<&str>, said_in_dialog: bool) -> Option<String> {
        Some(match (self, refusal) {
            (Self::SaveTo(name), None) => format!("Saved to {name}"),
            (Self::SaveAs(name), None) => format!("Saved as {name}"),
            (_, Some(_)) if said_in_dialog => return None,
            (Self::SaveTo(name), Some(why)) => format!("Not saved to {name}: {why}"),
            (Self::SaveAs(name), Some(why)) => format!("{name} not saved: {why}"),
            (Self::Rename(name), Some(why)) => format!("{name} not renamed: {why}"),
            (Self::Delete(name), Some(why)) => format!("{name} not deleted: {why}"),
            (Self::Startup(name), Some(why)) => {
                format!("{name} not made the startup profile: {why}")
            }
            (Self::Rename(_) | Self::Delete(_) | Self::Startup(_), None) => return None,
        })
    }
}

impl Core {
    /// The selected GPU's profile actions, as the `⋯` menu and the palette list them.
    pub(crate) fn profile_actions(&self, gpu_id: &str) -> Vec<(Command, String)> {
        let startup = match self.profiles.current_profiles.get(gpu_id) {
            Some(current) => format!("Use {} as startup profile", current.name),
            None => "Use current profile as startup profile".into(),
        };
        vec![
            (
                Command::SaveToCurrentProfile,
                "Save to current profile".into(),
            ),
            (Command::SaveAsNewProfile, "Save as new profile…".into()),
            (Command::RenameProfile, "Rename profile…".into()),
            (Command::DeleteProfile, "Delete profile…".into()),
            (Command::UseAsStartupProfile, startup),
            (Command::ManageProfiles, "Manage profiles…".into()),
        ]
    }

    /// The open `⋯` profile menu, if it is open.
    pub fn profile_menu(&self) -> Option<ProfileMenu> {
        if !self.profile_menu_open {
            return None;
        }
        let gpu_id = self.selected_gpu()?;
        Some(ProfileMenu {
            entries: self
                .profile_actions(&gpu_id)
                .into_iter()
                .map(|(command, title)| MenuEntry {
                    unavailable: self.unavailable(&command),
                    title,
                })
                .collect(),
        })
    }

    /// Whether the `⋯` profile menu is open.
    pub fn profile_menu_open(&self) -> bool {
        self.profile_menu().is_some()
    }

    /// Opens the `⋯` profile menu on the selected GPU's workspace, or closes it again.
    pub(crate) fn toggle_profile_menu(&mut self) {
        if self.profile_menu_open {
            self.profile_menu_open = false;
        } else if self.selected_gpu().is_some() && self.dialog.is_none() {
            self.palette = None;
            self.profile_menu_open = true;
        }
    }

    /// The command of the open profile menu's entry at `index`.
    pub(crate) fn profile_menu_command(&self, index: usize) -> Option<Command> {
        let gpu_id = self.selected_gpu()?;
        self.profile_actions(&gpu_id)
            .into_iter()
            .nth(index)
            .map(|(command, _)| command)
    }

    /// Why a profile action can't run now, when it can't; none when `command` isn't one.
    pub(crate) fn profile_action_unavailable(&self, command: &Command) -> Option<Option<String>> {
        let gpu_id = match command {
            Command::SaveToCurrentProfile
            | Command::SaveAsNewProfile
            | Command::RenameProfile
            | Command::DeleteProfile
            | Command::UseAsStartupProfile
            | Command::ManageProfiles => self.selected_gpu(),
            _ => return None,
        };
        let Some(gpu_id) = gpu_id else {
            return Some(Some("No GPU selected".into()));
        };
        if let Some(reason) = self.needs_tuning_access() {
            return Some(Some(reason));
        }
        Some(match command {
            Command::SaveAsNewProfile => self.save_blocked(&gpu_id),
            Command::SaveToCurrentProfile => self
                .own_current_profile(&gpu_id)
                .err()
                .or_else(|| self.save_blocked(&gpu_id)),
            Command::RenameProfile | Command::DeleteProfile => {
                self.own_current_profile(&gpu_id).err()
            }
            Command::UseAsStartupProfile => match self.own_current_profile(&gpu_id) {
                Err(reason) => Some(reason),
                Ok(profile) => self
                    .profiles
                    .startup_profiles
                    .get(&gpu_id)
                    .is_some_and(|startup| startup.id == profile.id)
                    .then(|| "Already the startup profile".into()),
            },
            _ => self
                .profiles_of(&gpu_id)
                .next()
                .is_none()
                .then(|| "No profiles for this GPU".into()),
        })
    }

    /// Why profiles can't be changed without tuning access, such as "Needs tuning access · not
    /// running", or none with it.
    pub(crate) fn needs_tuning_access(&self) -> Option<String> {
        self.tuning_access()
            .reason()
            .map(|reason| format!("Needs tuning access · {reason}"))
    }

    /// The GPU's current profile, while it's one of the caller's own, or why not.
    fn own_current_profile(&self, gpu_id: &str) -> Result<&GpuTuningProfile, String> {
        let current = self
            .profiles
            .current_profiles
            .get(gpu_id)
            .ok_or("No current profile")?;
        self.profile(&current.id)
            .ok_or_else(|| "The current profile belongs to another user".into())
    }

    /// Why saving the GPU's applied values to a profile is blocked now, when it is: it would
    /// capture values the GPU hasn't accepted yet, or values Prism doesn't know (ADR 0001).
    /// Pending cooling edits don't block it.
    fn save_blocked(&self, gpu_id: &str) -> Option<String> {
        if self.gpu_applying(gpu_id) {
            return Some("Applying…".into());
        }
        if self
            .profile_commands
            .values()
            .any(|sent| sent.gpu_id == gpu_id && sent.change.saves())
        {
            return Some("Saving…".into());
        }
        let edited = self.drafts.by_gpu.get(gpu_id).is_some_and(|draft| {
            ProfileSettingKey::ALL
                .into_iter()
                .any(|key| draft.settings.get(key).is_some())
        });
        if edited {
            return Some("Apply or discard the unapplied changes first".into());
        }
        let Some(gpu) = self.gpus().iter().find(|gpu| gpu.id == gpu_id) else {
            return Some("Its GPU isn't here".into());
        };
        let Latest::Read(device) = &gpu.latest else {
            return Some("Waiting for the GPU's readings".into());
        };
        let mut any = device.cooling.is_supported;
        for key in ProfileSettingKey::ALL {
            let support = match key {
                ProfileSettingKey::PowerLimitWatts => device.power.support,
                ProfileSettingKey::GraphicsOffsetMhz => device.clocks.graphics.support,
                ProfileSettingKey::MemoryOffsetMhz => device.clocks.memory.support,
            };
            match support {
                ControlSupport::Unsupported => continue,
                ControlSupport::Unknown => {
                    return Some(format!("{}: support is unknown", setting_name(key)));
                }
                ControlSupport::Supported => any = true,
            }
            if self.in_effect(gpu, key).is_none() {
                return Some(format!(
                    "{}: the current value is unavailable",
                    setting_name(key)
                ));
            }
        }
        if !any {
            return Some("This GPU has no supported profile settings".into());
        }
        let Some(state) = self.tuning.get(gpu_id) else {
            return Some("Waiting for prismd's tuning state".into());
        };
        // prismd reports no cooling it can't capture, such as after a cooling fault.
        (device.cooling.is_supported && state.reported.cooling.is_none())
            .then(|| "Cooling: the current policy is unavailable".into())
    }

    /// Whether the GPU's applied values or explicit defaults differ from its current profile, as
    /// far as Prism knows: the profile must be the caller's own, and a value prismd can't read
    /// differs from nothing.
    pub(crate) fn changes_not_saved_to_profile(&self, gpu_id: &str) -> bool {
        let Some(profile) = self.own_current_profile(gpu_id).ok() else {
            return false;
        };
        let Some(state) = self.tuning.get(gpu_id) else {
            return false;
        };
        let setting_differs = ProfileSettingKey::ALL.into_iter().any(|key| {
            let (Some(saved), Some(value)) =
                (profile.settings.get(key), reported(&state.reported, key))
            else {
                return false;
            };
            let applied = if state.defaults.contains(&key) {
                ProfileSetting::Default {}
            } else {
                ProfileSetting::Value { value }
            };
            applied != saved
        });
        let cooling_differs = matches!(
            (&profile.settings.cooling, &state.reported.cooling),
            (Some(saved), Some(applied)) if saved != applied
        );
        setting_differs || cooling_differs
    }

    /// Saves the selected GPU's applied values to its current profile.
    pub(crate) fn save_to_current_profile(&mut self) -> Vec<Effect> {
        let Some(gpu_id) = self.selected_gpu() else {
            return Vec::new();
        };
        let Ok(profile) = self.own_current_profile(&gpu_id) else {
            return Vec::new();
        };
        let change = ProfileChange::SaveTo(profile.name.clone());
        let request = DaemonRequest::SaveToCurrentProfile(SaveToCurrentProfileRequest {
            revision: self.revision_of(&gpu_id),
            gpu_id: gpu_id.clone(),
        });
        self.send_profile_change(gpu_id, change, request).1
    }

    /// Makes the selected GPU's current profile its startup profile.
    pub(crate) fn use_as_startup_profile(&mut self) -> Vec<Effect> {
        let Some(gpu_id) = self.selected_gpu() else {
            return Vec::new();
        };
        let Ok(profile) = self.own_current_profile(&gpu_id) else {
            return Vec::new();
        };
        let change = ProfileChange::Startup(profile.name.clone());
        let request = DaemonRequest::SetStartupProfile(SetStartupProfileRequest {
            gpu_id: gpu_id.clone(),
            profile_id: Some(profile.id.clone()),
        });
        self.send_profile_change(gpu_id, change, request).1
    }

    /// Sends `request` for a profile change on the GPU, noting it until prismd answers. What the
    /// change strip said about the GPU no longer holds.
    fn send_profile_change(
        &mut self,
        gpu_id: String,
        change: ProfileChange,
        request: DaemonRequest,
    ) -> (CommandId, Vec<Effect>) {
        let id = self.drafts.next_command();
        self.profile_outcomes.remove(&gpu_id);
        self.profile_commands
            .insert(id, ProfileCommand { gpu_id, change });
        (id, vec![Effect::CommandPrismd { id, request }])
    }

    /// Asks for the name of a new profile of the selected GPU's applied values.
    pub(crate) fn open_save_as(&mut self) -> Vec<Effect> {
        if let Some(gpu_id) = self.selected_gpu() {
            self.open_dialog(DialogState::ProfileName {
                naming: Naming::SaveAsNew { gpu_id },
                name: String::new(),
                error: None,
                sending: None,
                managing: None,
            });
        }
        Vec::new()
    }

    /// Asks for a new name for the selected GPU's current profile.
    pub(crate) fn open_rename_current(&mut self) -> Vec<Effect> {
        if let Some(id) = self.current_profile_id() {
            self.open_rename(&id, false);
        }
        Vec::new()
    }

    /// Asks before deleting the selected GPU's current profile.
    pub(crate) fn open_delete_current(&mut self) -> Vec<Effect> {
        if let Some(id) = self.current_profile_id() {
            self.open_delete(&id, false);
        }
        Vec::new()
    }

    /// The selected GPU's current profile's id, while it's one of the caller's own.
    fn current_profile_id(&self) -> Option<String> {
        let gpu_id = self.selected_gpu()?;
        self.own_current_profile(&gpu_id)
            .ok()
            .map(|profile| profile.id.clone())
    }

    /// Asks for the profile's new name, starting from its name, provided it can be renamed.
    pub(crate) fn open_rename(&mut self, profile_id: &str, managing: bool) {
        let Some(profile) = self.profile(profile_id) else {
            return;
        };
        if self.tuning_access().reason().is_some() {
            return;
        }
        let name = profile.name.clone();
        let managing = managing.then(|| profile.gpu_id.clone());
        self.open_dialog(DialogState::ProfileName {
            naming: Naming::Rename {
                profile_id: profile_id.into(),
            },
            name,
            error: None,
            sending: None,
            managing,
        });
    }

    /// Asks before deleting the profile, provided it can be deleted.
    pub(crate) fn open_delete(&mut self, profile_id: &str, managing: bool) {
        let Some(profile) = self.profile(profile_id) else {
            return;
        };
        if self.tuning_access().reason().is_some() {
            return;
        }
        let managing = managing.then(|| profile.gpu_id.clone());
        self.open_dialog(DialogState::DeleteProfile {
            profile_id: profile_id.into(),
            error: None,
            sending: None,
            managing,
        });
    }

    /// Lists the selected GPU's profiles, to rename or delete any of them.
    pub(crate) fn open_manage(&mut self) -> Vec<Effect> {
        if let Some(gpu_id) = self.selected_gpu() {
            self.open_dialog(DialogState::ManageProfiles { gpu_id });
        }
        Vec::new()
    }

    /// The open name dialog's name now reads `name`, which drops why prismd refused the last one.
    pub(crate) fn edit_profile_name(&mut self, edited: String) {
        if let Some(DialogState::ProfileName { name, error, .. }) = &mut self.dialog {
            *name = edited;
            *error = None;
        }
    }

    /// Closes the dialog, going back to Manage profiles when it was opened from there.
    pub(crate) fn close_dialog(&mut self) {
        self.dialog = match self.dialog.take() {
            Some(
                DialogState::ProfileName {
                    managing: Some(gpu_id),
                    ..
                }
                | DialogState::DeleteProfile {
                    managing: Some(gpu_id),
                    ..
                },
            ) => Some(DialogState::ManageProfiles { gpu_id }),
            _ => None,
        };
    }

    /// Sends what the open name or delete dialog asks for, unless it can't be sent now.
    pub(crate) fn confirm_profile_change(&mut self) -> Vec<Effect> {
        if self.profile_dialog_unavailable().is_some() {
            return Vec::new();
        }
        let (gpu_id, change, request) = match &self.dialog {
            Some(DialogState::ProfileName { naming, name, .. }) => {
                let name = name.trim().to_owned();
                match naming {
                    Naming::SaveAsNew { gpu_id } => (
                        gpu_id.clone(),
                        ProfileChange::SaveAs(name.clone()),
                        DaemonRequest::SaveProfileAsNew(SaveProfileAsNewRequest {
                            gpu_id: gpu_id.clone(),
                            name,
                            revision: self.revision_of(gpu_id),
                        }),
                    ),
                    Naming::Rename { profile_id } => {
                        let Some(profile) = self.profile(profile_id) else {
                            return Vec::new();
                        };
                        (
                            profile.gpu_id.clone(),
                            ProfileChange::Rename(profile.name.clone()),
                            DaemonRequest::RenameProfile(RenameProfileRequest {
                                profile_id: profile_id.clone(),
                                name,
                            }),
                        )
                    }
                }
            }
            Some(DialogState::DeleteProfile { profile_id, .. }) => {
                let Some(profile) = self.profile(profile_id) else {
                    return Vec::new();
                };
                (
                    profile.gpu_id.clone(),
                    ProfileChange::Delete(profile.name.clone()),
                    DaemonRequest::DeleteProfile(ProfileIdRequest {
                        profile_id: profile_id.clone(),
                    }),
                )
            }
            _ => return Vec::new(),
        };
        let (id, effects) = self.send_profile_change(gpu_id, change, request);
        if let Some(
            DialogState::ProfileName { sending, error, .. }
            | DialogState::DeleteProfile { sending, error, .. },
        ) = &mut self.dialog
        {
            *sending = Some(id);
            *error = None;
        }
        effects
    }

    /// Why the open name or delete dialog's confirmation can't be sent now, when it can't.
    fn profile_dialog_unavailable(&self) -> Option<String> {
        match self.dialog.as_ref()? {
            DialogState::ProfileName {
                naming,
                name,
                sending,
                ..
            } => {
                if sending.is_some() {
                    return Some(match naming {
                        Naming::SaveAsNew { .. } => "Saving…".into(),
                        Naming::Rename { .. } => "Renaming…".into(),
                    });
                }
                if let Some(reason) = self.needs_tuning_access() {
                    return Some(reason);
                }
                match naming {
                    Naming::SaveAsNew { gpu_id } => self
                        .name_problem(gpu_id, name, None)
                        .or_else(|| self.save_blocked(gpu_id)),
                    Naming::Rename { profile_id } => {
                        let profile = self.profile(profile_id)?;
                        self.name_problem(&profile.gpu_id, name, Some(profile_id))
                    }
                }
            }
            DialogState::DeleteProfile { sending, .. } => {
                if sending.is_some() {
                    return Some("Deleting…".into());
                }
                self.needs_tuning_access()
            }
            _ => None,
        }
    }

    /// What's wrong with `name` for a profile of the GPU, other than the profile `renaming`: it's
    /// empty, or another of the caller's profiles of the GPU has it, whatever its letter case.
    fn name_problem(&self, gpu_id: &str, name: &str, renaming: Option<&str>) -> Option<String> {
        let name = name.trim();
        if name.is_empty() {
            return Some("Enter a name for the profile".into());
        }
        self.name_clash(gpu_id, name, renaming)
    }

    /// The clash, if another of the caller's profiles of the GPU than `renaming` has `name`,
    /// whatever its letter case.
    fn name_clash(&self, gpu_id: &str, name: &str, renaming: Option<&str>) -> Option<String> {
        let name = name.trim().to_lowercase();
        self.profiles_of(gpu_id)
            .find(|profile| {
                Some(profile.id.as_str()) != renaming && profile.name.to_lowercase() == name
            })
            .map(|profile| {
                format!(
                    "A profile named \"{}\" already exists for this GPU. Choose another name.",
                    profile.name
                )
            })
    }

    /// Takes in prismd's answer to a profile change, if `command` was one: the dialog that sent it
    /// closes, or says why prismd refused it, and the change strip says how a save went. Says
    /// whether it was one.
    pub(crate) fn profile_command_answered(
        &mut self,
        command: CommandId,
        response: &DaemonResponse,
    ) -> bool {
        let Some(sent) = self.profile_commands.remove(&command) else {
            return false;
        };
        let refusal = match response {
            DaemonResponse::Profile(_)
            | DaemonResponse::Command(DaemonCommandResponse { success: true, .. }) => None,
            DaemonResponse::Error(error) => Some(error.message.clone()),
            DaemonResponse::Command(DaemonCommandResponse { error, .. }) => {
                Some(error.clone().unwrap_or_else(|| "prismd refused it.".into()))
            }
            _ => Some("prismd answered with something else. Try again.".into()),
        };
        let waiting = match &mut self.dialog {
            Some(
                DialogState::ProfileName { sending, error, .. }
                | DialogState::DeleteProfile { sending, error, .. },
            ) if *sending == Some(command) => {
                *sending = None;
                error.clone_from(&refusal);
                true
            }
            _ => false,
        };
        if waiting && refusal.is_none() {
            self.close_dialog();
        }
        if let Some(outcome) = sent.change.outcome(refusal.as_deref(), waiting) {
            self.profile_outcomes.insert(sent.gpu_id, outcome);
        }
        true
    }

    /// The session ended: the profile dialogs have nothing left to change, and changes it didn't
    /// answer may or may not have happened.
    pub(crate) fn profile_changes_lost_session(&mut self) {
        if matches!(
            self.dialog,
            Some(
                DialogState::ProfileName { .. }
                    | DialogState::DeleteProfile { .. }
                    | DialogState::ManageProfiles { .. }
            )
        ) {
            self.dialog = None;
        }
        self.profile_menu_open = false;
        for sent in std::mem::take(&mut self.profile_commands).into_values() {
            let (doing, again) = sent.change.doing();
            self.profile_outcomes.insert(
                sent.gpu_id,
                format!(
                    "The prismd connection ended before {doing} was answered. Check the profile, \
                     then {again}."
                ),
            );
        }
    }

    /// The profile name, delete or manage dialog as shown, if `dialog` is one.
    pub(crate) fn profile_change_dialog(&self, dialog: &DialogState) -> Option<Option<Dialog>> {
        Some(match dialog {
            DialogState::ProfileName {
                naming,
                name,
                error,
                ..
            } => {
                let (title, message, confirm_text, clash) = match naming {
                    Naming::SaveAsNew { gpu_id } => (
                        "Save as new profile".to_owned(),
                        format!(
                            "Saves the values in effect on {} as a new profile, which becomes its \
                             current profile.",
                            self.gpu_name(gpu_id)
                        ),
                        "Save",
                        self.name_clash(gpu_id, name, None),
                    ),
                    Naming::Rename { profile_id } => {
                        let Some(profile) = self.profile(profile_id) else {
                            return Some(None);
                        };
                        (
                            format!("Rename {}", profile.name),
                            "Games, the startup profile and the current profile keep following \
                             it under its new name."
                                .to_owned(),
                            "Rename",
                            self.name_clash(&profile.gpu_id, name, Some(profile_id)),
                        )
                    }
                };
                Some(Dialog::ProfileName(ProfileNameDialog {
                    title,
                    message,
                    name: name.clone(),
                    error: error.clone().or(clash),
                    confirm_text,
                    confirm_unavailable: self.profile_dialog_unavailable(),
                }))
            }
            DialogState::DeleteProfile {
                profile_id, error, ..
            } => {
                let Some(profile) = self.profile(profile_id) else {
                    return Some(None);
                };
                let startup = self
                    .profiles
                    .startup_profiles
                    .get(&profile.gpu_id)
                    .is_some_and(|startup| startup.id == profile.id);
                let mut message = "Deleting it can't be undone. The GPU keeps the values in \
                                   effect, and games linked to it no longer switch to it."
                    .to_owned();
                if startup {
                    message.push_str(
                        " It's the GPU's startup profile, so the GPU will have no startup profile.",
                    );
                }
                Some(Dialog::DeleteProfile(ConfirmDelete {
                    title: format!("Delete {}?", profile.name),
                    message,
                    error: error.clone(),
                    confirm_unavailable: self.profile_dialog_unavailable(),
                }))
            }
            DialogState::ManageProfiles { gpu_id } => {
                let current = self.profiles.current_profiles.get(gpu_id);
                let startup = self.profiles.startup_profiles.get(gpu_id);
                Some(Dialog::ManageProfiles(ManageProfiles {
                    title: "Manage profiles".into(),
                    message: format!("The profiles of {}.", self.gpu_name(gpu_id)),
                    profiles: self
                        .profiles_of(gpu_id)
                        .map(|profile| {
                            let is = |of: Option<&prism_protocol::CurrentProfile>| {
                                of.is_some_and(|of| of.id == profile.id)
                            };
                            let marks: Vec<_> = [
                                is(current).then_some("Current profile"),
                                is(startup).then_some("Startup profile"),
                            ]
                            .into_iter()
                            .flatten()
                            .collect();
                            ManagedProfile {
                                id: profile.id.clone(),
                                name: profile.name.clone(),
                                marks: marks.join(" · "),
                            }
                        })
                        .collect(),
                    actions_unavailable: self.needs_tuning_access(),
                }))
            }
            _ => return None,
        })
    }

    /// The GPU as the user knows it, such as "GPU 0 · RTX 5090 · 01:00.0".
    fn gpu_name(&self, gpu_id: &str) -> String {
        self.gpus()
            .iter()
            .enumerate()
            .find(|(_, gpu)| gpu.id == gpu_id)
            .map_or_else(|| gpu_id.to_owned(), |(index, gpu)| gpu_label(index, gpu))
    }
}
