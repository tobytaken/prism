//! An X display of the test's own, run by `Xvfb`.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use tempfile::TempDir;

pub struct PrivateDisplay {
    server: Child,
    /// What `DISPLAY` names it by, such as `:3`.
    name: String,
    /// Where glvnd looks for EGL vendors, and finds none.
    no_egl_vendors: TempDir,
}

impl PrivateDisplay {
    /// A display where clients find no OpenGL, as in a Flatpak without its GL extension: the
    /// server has no GLX extension, and clients pointed at it find no EGL vendor.
    pub fn without_opengl() -> Self {
        // Xvfb picks a free display and writes its number to the descriptor, here stdout.
        let mut server = Command::new("Xvfb")
            .args(["-displayfd", "1", "-nolisten", "tcp", "-extension", "GLX"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("run Xvfb, from the xvfb package, for the test's display");
        let mut number = String::new();
        BufReader::new(server.stdout.take().unwrap())
            .read_line(&mut number)
            .unwrap();
        Self {
            server,
            name: format!(":{}", number.trim()),
            no_egl_vendors: tempfile::tempdir().unwrap(),
        }
    }

    /// Points `command` at this display, and away from any Wayland one.
    pub fn show<'a>(&self, command: &'a mut Command) -> &'a mut Command {
        command
            .env("DISPLAY", &self.name)
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("WAYLAND_SOCKET")
            .env("__EGL_VENDOR_LIBRARY_DIRS", self.no_egl_vendors.path())
    }
}

impl Drop for PrivateDisplay {
    fn drop(&mut self) {
        let _ = self.server.kill();
        let _ = self.server.wait();
    }
}
