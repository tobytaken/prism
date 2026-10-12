//! Opening links in the desktop's app for them, through the desktop portal's
//! `org.freedesktop.portal.OpenURI`, which works natively and in Flatpak, or else `xdg-open`.
//!
//! THIRD-PARTY-LICENSES is installed with Prism, where only Prism may see it in a sandbox, so the
//! portal gets the file itself rather than its path.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::File;
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::thread;

use prism_ui::Link;
use zbus::blocking::Connection;
use zbus::zvariant::{Fd, Value};

use crate::single_instance::APP_ID;

/// Opens links in the desktop's app for them.
pub trait Opener {
    /// Opens `link`, saying on stderr when it can't.
    fn open(&self, link: Link);
}

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const OPEN_URI: &str = "org.freedesktop.portal.OpenURI";
/// The deb and rpm packages' name, which their license directories are named after.
const PACKAGE: &str = "prism-gpu";
const THIRD_PARTY_LICENSES: &str = "THIRD-PARTY-LICENSES";

/// The desktop portal, each link on a thread of its own, since the portal may ask the user which
/// app to open it in.
#[derive(Clone)]
pub struct DesktopOpener {
    connect: Arc<dyn Fn() -> zbus::Result<Connection> + Send + Sync>,
}

impl DesktopOpener {
    /// On the session bus.
    pub fn session() -> Self {
        Self {
            connect: Arc::new(Connection::session),
        }
    }
}

impl Opener for DesktopOpener {
    fn open(&self, link: Link) {
        let connect = self.connect.clone();
        let started = thread::Builder::new()
            .name("prism-opener".into())
            .spawn(move || {
                if let Err(error) = open(&*connect, link) {
                    eprintln!("prism: can't open {link:?}: {error}");
                }
            });
        if let Err(error) = started {
            eprintln!("prism: can't open {link:?}: {error}");
        }
    }
}

fn open(connect: &dyn Fn() -> zbus::Result<Connection>, link: Link) -> Result<(), String> {
    let target = match link.url() {
        Some(url) => Target::Url(url.to_owned()),
        None => {
            let executable = std::env::current_exe().map_err(|error| error.to_string())?;
            let path = third_party_licenses(&executable)
                .into_iter()
                .find(|path| path.is_file())
                .ok_or_else(|| format!("{THIRD_PARTY_LICENSES} isn't installed"))?;
            Target::File(path)
        }
    };
    let through_portal = connect().and_then(|connection| match &target {
        Target::Url(url) => open_uri(&connection, url),
        Target::File(path) => open_file(&connection, path),
    });
    match through_portal {
        Ok(()) => Ok(()),
        // Without a portal, as on a desktop that doesn't run one.
        Err(portal) => xdg_open(&target).map_err(|error| format!("{portal}; {error}")),
    }
}

enum Target {
    Url(String),
    File(PathBuf),
}

fn open_uri(connection: &Connection, url: &str) -> zbus::Result<()> {
    let options: HashMap<&str, Value> = HashMap::new();
    connection.call_method(
        Some(PORTAL),
        PORTAL_PATH,
        Some(OPEN_URI),
        "OpenURI",
        &("", url, options),
    )?;
    Ok(())
}

fn open_file(connection: &Connection, path: &Path) -> zbus::Result<()> {
    let file = File::open(path).map_err(|error| zbus::Error::Failure(error.to_string()))?;
    let options: HashMap<&str, Value> = HashMap::new();
    connection.call_method(
        Some(PORTAL),
        PORTAL_PATH,
        Some(OPEN_URI),
        "OpenFile",
        &("", Fd::from(file.as_fd()), options),
    )?;
    Ok(())
}

fn xdg_open(target: &Target) -> Result<(), String> {
    let argument = match target {
        Target::Url(url) => OsStr::new(url),
        Target::File(path) => path.as_os_str(),
    };
    let status = Command::new("xdg-open")
        .arg(argument)
        .status()
        .map_err(|error| format!("can't run xdg-open: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("xdg-open failed: {status}"))
    }
}

/// Where THIRD-PARTY-LICENSES may be, most likely first, for the Prism at `executable`: under
/// its prefix, where each package installs it, then in the source tree, for a development build.
fn third_party_licenses(executable: &Path) -> Vec<PathBuf> {
    let prefix = executable.parent().and_then(Path::parent);
    let installed = prefix.into_iter().flat_map(|prefix| {
        let share = prefix.join("share");
        [
            // Flatpak and AppImage.
            share.join("licenses").join(APP_ID),
            // rpm.
            share.join("licenses").join(PACKAGE),
            // deb.
            share.join("doc").join(PACKAGE),
        ]
    });
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    installed
        .chain([source])
        .map(|directory| directory.join(THIRD_PARTY_LICENSES))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::third_party_licenses;

    #[test]
    fn third_party_licenses_are_looked_for_where_each_package_installs_them() {
        let candidates = third_party_licenses(Path::new("/usr/bin/prism"));

        assert_eq!(
            candidates[..3],
            [
                "/usr/share/licenses/io.github.tobytaken.Prism/THIRD-PARTY-LICENSES",
                "/usr/share/licenses/prism-gpu/THIRD-PARTY-LICENSES",
                "/usr/share/doc/prism-gpu/THIRD-PARTY-LICENSES",
            ]
            .map(PathBuf::from)
        );
        assert_eq!(
            third_party_licenses(Path::new("/app/bin/prism"))[0],
            Path::new("/app/share/licenses/io.github.tobytaken.Prism/THIRD-PARTY-LICENSES")
        );
    }

    #[test]
    fn a_development_build_finds_the_source_trees_third_party_licenses() {
        let candidates = third_party_licenses(Path::new("/home/user/prism/target/debug/prism"));

        assert!(candidates.last().unwrap().is_file());
    }
}
