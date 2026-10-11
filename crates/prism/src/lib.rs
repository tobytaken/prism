//! Prism's GUI: the Slint view over the prism-ui core, and the composition root that wires it to
//! the GPUs, prismd and the host.

/// The Slint view, generated from `ui/app.slint`.
#[allow(clippy::todo)] // Slint's generated code, not Prism's.
mod ui {
    slint::include_modules!();
}

pub use crate::ui::*;

mod activation;
mod app;
mod chart;
mod environment;
mod prismd;
mod prismd_host;
mod sampler;
pub mod settings;
pub mod single_instance;
mod sync;

pub use crate::activation::DesktopActivation;
pub use crate::app::App;
pub use crate::environment::{
    Clock, Environment, HostServices, Launches, NoLaunches, Sampling, SystemClock, WindowActivation,
};
pub use crate::prismd::{NoPrismd, PrismdConnector, PrismdReport, SocketConnector};
pub use crate::prismd_host::{HostPrismd, PrismdService};
