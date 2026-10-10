//! Raising the window through the desktop. On Wayland only an activation token lets Prism focus
//! its own window (ADR 0012), so the token a later launch hands over is passed on through
//! `xdg_activation_v1`; on X11 the window manager is asked to focus it.

use std::error::Error;
use std::ffi::c_void;
use std::ptr::NonNull;

use slint::winit_030::WinitWindowAccessor;
use slint::winit_030::winit::raw_window_handle::{
    HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
};

use crate::environment::WindowActivation;

/// Focuses the window through the desktop: Wayland's `xdg_activation_v1` with the activation
/// token, or the X11 window manager. Does nothing on other backends, such as the testing one.
#[derive(Debug, Clone, Copy, Default)]
pub struct DesktopActivation;

impl WindowActivation for DesktopActivation {
    fn activate(&self, window: &slint::Window, activation_token: Option<&str>) {
        let activated = window.with_winit_window(|window| {
            let display = window.display_handle().map(|handle| handle.as_raw());
            let surface = window.window_handle().map(|handle| handle.as_raw());
            match (display, surface) {
                (Ok(RawDisplayHandle::Wayland(display)), Ok(RawWindowHandle::Wayland(surface))) => {
                    // Without a token the compositor wouldn't focus the window anyway.
                    let Some(token) = activation_token else {
                        return Ok(());
                    };
                    activate_on_wayland(display.display, surface.surface, token)
                }
                _ => {
                    window.focus_window();
                    Ok(())
                }
            }
        });
        if let Some(Err(error)) = activated {
            eprintln!("prism: can't raise the window: {error}");
        }
    }
}

/// Activates the window's `surface` with `token` through the compositor's `xdg_activation_v1`,
/// on winit's own Wayland `display`.
fn activate_on_wayland(
    display: NonNull<c_void>,
    surface: NonNull<c_void>,
    token: &str,
) -> Result<(), Box<dyn Error>> {
    use wayland_client::backend::{Backend, ObjectId};
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::wl_registry::{self, WlRegistry};
    use wayland_client::protocol::wl_surface::WlSurface;
    use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
    use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::{
        self, XdgActivationV1,
    };

    /// The activation needs no events.
    struct Ignore;

    impl Dispatch<WlRegistry, GlobalListContents> for Ignore {
        fn event(
            _: &mut Self,
            _: &WlRegistry,
            _: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<XdgActivationV1, ()> for Ignore {
        fn event(
            _: &mut Self,
            _: &XdgActivationV1,
            _: xdg_activation_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    // SAFETY: the display is winit's connection, which lives as long as the window this is
    // called with, and so outlives this guest backend, dropped at the end of this function.
    let backend = unsafe { Backend::from_foreign_display(display.as_ptr().cast()) };
    let connection = Connection::from_backend(backend);
    // The roundtrip runs on a queue of its own, so winit's events stay on winit's queue.
    let (globals, queue) = registry_queue_init::<Ignore>(&connection)?;
    let activation: XdgActivationV1 = globals.bind(&queue.handle(), 1..=1, ())?;
    // SAFETY: the surface is the window's `wl_surface`, which outlives this call.
    let surface_id =
        unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.as_ptr().cast()) }?;
    let surface = WlSurface::from_id(&connection, surface_id)?;
    activation.activate(token.to_owned(), &surface);
    activation.destroy();
    connection.flush()?;
    Ok(())
}
