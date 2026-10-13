//! Compact mode: the same window with each GPU's vitals, profile buttons and automatic switching
//! status, stacked. It doesn't edit, so a command that needs more opens the full layout.

use crate::core::{Core, Effect};
use crate::palette::Command;

impl Command {
    /// Whether running it shows what only the full layout has, such as a GPU's workspace, a
    /// section, Activity, or a dialog about editing.
    fn needs_full_layout(&self) -> bool {
        match self {
            Self::ApplyProfile(_)
            | Self::SetSwitchingPaused { .. }
            | Self::RetrySwitching(_)
            | Self::SetAutomaticSwitching(_)
            | Self::SetAppearance(_)
            | Self::SetDesktopNotifications(_)
            | Self::RunService(_)
            | Self::CopyPrismdSetup
            | Self::ToggleCompactMode
            | Self::Quit => false,
            Self::GoToGpu(_)
            | Self::GoTo(_)
            | Self::GoToSettings(_)
            | Self::ShowRange(_)
            | Self::ApplyChanges
            | Self::DiscardChanges
            | Self::RecallRememberedCurve
            | Self::SaveToCurrentProfile
            | Self::SaveAsNewProfile
            | Self::RenameProfile
            | Self::DeleteProfile
            | Self::UseAsStartupProfile
            | Self::ManageProfiles
            | Self::AddSteamGame
            | Self::AddProgram
            | Self::OpenActivity
            | Self::RetryCooling(_)
            | Self::RunDriverPlan
            | Self::RebootHost => true,
        }
    }
}

impl Core {
    /// Shows compact mode in place of the full layout, closing Activity and the profile menu,
    /// which it doesn't have, or the full layout again, and saves the choice.
    pub(crate) fn toggle_compact_mode(&mut self) -> Vec<Effect> {
        self.compact = !self.compact;
        if self.compact {
            self.close_activity();
            self.profile_menu_open = false;
        }
        vec![Effect::SaveSettings(self.gui_settings())]
    }

    /// Opens the full layout, and saves the choice, when compact mode is showing and `command`
    /// needs more than it has.
    pub(crate) fn leave_compact_mode_for(&mut self, command: &Command) -> Vec<Effect> {
        if !self.compact || !command.needs_full_layout() {
            return Vec::new();
        }
        self.compact = false;
        vec![Effect::SaveSettings(self.gui_settings())]
    }
}
