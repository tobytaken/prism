//! Prism's GUI: the Slint view over the prism-ui core, and the composition root that wires it to
//! the GPUs, prismd and the host.

/// The Slint view, generated from `ui/app.slint`.
#[allow(clippy::todo)] // Slint's generated code, not Prism's.
mod ui {
    slint::include_modules!();
}

pub use crate::ui::*;

mod app;
mod chart;
mod environment;
mod sampler;
pub mod settings;
mod sync;

pub use crate::app::App;
pub use crate::environment::{
    Clock, Environment, HostServices, PrismdConnector, Sampling, SystemClock,
};
