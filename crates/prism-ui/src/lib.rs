//! Prism's UI logic: a pure state machine with no toolkit, threads or I/O.
//!
//! [`Core`] lives on the UI thread. Everything that happens reaches it as an [`Input`] through
//! [`Core::handle`], which returns the [`Effect`]s the caller performs. The view reads the
//! resulting state through the core's view models.

mod activity;
mod cooling;
mod cooling_faults;
mod core;
mod curve;
mod daemon;
mod drafts;
mod drivers;
pub mod format;
mod games;
mod host;
mod notifications;
mod palette;
mod profile_management;
mod profiles;
mod settings;
mod switching;
pub mod telemetry;
mod view;

pub use crate::activity::ActivityAction;
pub use crate::cooling::{FanArrangement, FanMode};
pub use crate::core::{
    AppearanceMode, CommandId, Core, Effect, Input, Intent, NotConnected, PrismdConnection,
    Section, SessionFailure, SettingsSection, TelemetrySource, Timer, TuningAccess, TuningControl,
};
pub use crate::daemon::{DaemonAction, DaemonSettings, Fact, ServiceAction, ServiceOutcome};
pub use crate::drafts::ReviewChange;
pub use crate::games::ProgramFile;
pub use crate::host::{Distro, PrismdHost, PrismdInstall, UnitState};
pub use crate::notifications::{Notification, NotificationId};
pub use crate::palette::{Key, Palette, PaletteEntry, Shortcut};
pub use crate::settings::{About, GuiSettings, Link, SLINT_WEBSITE};
pub use crate::view::{
    ActivityButton, ActivityItem, ActivityPanel, AddProgram, AddSteamGame, ChangeStrip,
    ConfirmClose, ConfirmDelete, Content, CoolingProblem, CurveMarker, CurvePoint, Detail, Dialog,
    DiscardForProfile, DriverConfirmation, DriverJobView, DriverPlanView, DriversSection,
    ExplainedState, FanArrangementControl, FanChoice, FanCurveEditor, FanModeChoice,
    FanModeControl, GameLink, GameRow, GamesSection, GpuWorkspace, HardwareDetails, LinkChoice,
    ManageProfiles, ManagedProfile, MenuEntry, ProfileChoice, ProfileCorrections, ProfileMenu,
    ProfileNameDialog, ProfileSwitcher, ProgramChoice, Review, Sidebar, SidebarGpu, StatusBar,
    SteamAppChoice, Stepper, SwitchingStatus, Track, TrackChart, TrackKind, Tunable,
};
