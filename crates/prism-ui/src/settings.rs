//! The GUI's own settings, which Settings → Appearance and Notifications change and Prism saves
//! per user, and what Settings → About shows.

use crate::core::{AppearanceMode, Core, Effect};
use crate::games::on_off;
use crate::palette::Command;

/// The settings the GUI keeps for itself, per user, apart from prismd's tuning store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuiSettings {
    pub appearance: AppearanceMode,
    /// Whether Prism sends desktop notifications at all.
    pub desktop_notifications: bool,
}

impl Default for GuiSettings {
    /// System appearance, with desktop notifications on.
    fn default() -> Self {
        Self {
            appearance: AppearanceMode::System,
            desktop_notifications: true,
        }
    }
}

/// Something Prism opens in the desktop's app for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    /// The THIRD-PARTY-LICENSES file installed with Prism.
    ThirdPartyLicenses,
    /// Slint's website, which the "Built with Slint" line leads to.
    SlintWebsite,
}

impl Link {
    /// The web address it opens, for one on the web.
    pub fn url(self) -> Option<&'static str> {
        match self {
            Self::ThirdPartyLicenses => None,
            Self::SlintWebsite => Some("https://slint.dev"),
        }
    }
}

/// What Settings → About shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct About {
    pub version: String,
    /// Prism's license, as an SPDX expression.
    pub license: String,
    /// Where "Built with Slint" leads.
    pub slint_website: String,
}

impl AppearanceMode {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    /// What the palette calls using it, such as "Use dark appearance".
    pub(crate) fn command(self) -> &'static str {
        match self {
            Self::System => "Use system appearance",
            Self::Light => "Use light appearance",
            Self::Dark => "Use dark appearance",
        }
    }
}

impl Core {
    pub fn about(&self) -> About {
        About {
            version: env!("CARGO_PKG_VERSION").into(),
            license: env!("CARGO_PKG_LICENSE").into(),
            slint_website: Link::SlintWebsite.url().unwrap_or_default().into(),
        }
    }

    /// The settings as they are now, as Prism saves them.
    pub(crate) fn gui_settings(&self) -> GuiSettings {
        GuiSettings {
            appearance: self.appearance,
            desktop_notifications: self.desktop_notifications(),
        }
    }

    /// Uses `mode`, unless it's in use already, and saves it.
    pub(crate) fn set_appearance(&mut self, mode: AppearanceMode) -> Vec<Effect> {
        if self.appearance == mode {
            return Vec::new();
        }
        self.appearance = mode;
        vec![Effect::SaveSettings(self.gui_settings())]
    }

    /// Turns desktop notifications on or off, unless they are already, and saves it. Off, the
    /// live ones go.
    pub(crate) fn set_desktop_notifications(&mut self, enabled: bool) -> Vec<Effect> {
        if self.desktop_notifications() == enabled {
            return Vec::new();
        }
        let mut effects: Vec<_> = self
            .notifications
            .set_enabled(enabled)
            .into_iter()
            .map(Effect::WithdrawNotification)
            .collect();
        effects.push(Effect::SaveSettings(self.gui_settings()));
        effects
    }

    /// The palette's appearance commands, one for each mode.
    pub(crate) fn appearance_commands(&self) -> Vec<(Command, String)> {
        AppearanceMode::ALL
            .into_iter()
            .map(|mode| (Command::SetAppearance(mode), mode.command().to_owned()))
            .collect()
    }

    /// The palette's command that turns desktop notifications on or off, as they aren't now.
    pub(crate) fn desktop_notifications_command(&self) -> (Command, String) {
        let enable = !self.desktop_notifications();
        (
            Command::SetDesktopNotifications(enable),
            format!("Turn desktop notifications {}", on_off(enable)),
        )
    }
}
