//! Prism's UI logic: a pure state machine with no toolkit, threads or I/O.
//!
//! [`Core`] lives on the UI thread. Everything that happens reaches it as an [`Input`] through
//! [`Core::handle`], which returns the [`Effect`]s the caller performs. The view reads the
//! resulting state through the core's view models.

mod core;
mod daemon;
pub mod format;
mod host;
mod notifications;
mod palette;
pub mod telemetry;
mod view;

pub use crate::core::{
    AppearanceMode, Core, Effect, Input, Intent, NotConnected, PrismdConnection, Section,
    SessionFailure, SettingsSection, TelemetrySource, Timer, TuningAccess, TuningControl,
};
pub use crate::daemon::{DaemonAction, DaemonSettings, Fact, ServiceAction, ServiceOutcome};
pub use crate::host::{Distro, PrismdHost, PrismdInstall, UnitState};
pub use crate::notifications::{Notification, NotificationId};
pub use crate::palette::{Key, Palette, PaletteEntry, Shortcut};
pub use crate::view::{
    Content, ExplainedState, GpuWorkspace, Sidebar, SidebarGpu, StatusBar, TemperatureTrack,
};
