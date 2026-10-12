//! Choosing a program file through the desktop portal's `org.freedesktop.portal.FileChooser`,
//! which works natively and in Flatpak, and the document-portal host-path recovery its answer
//! goes through before it crosses to prismd.
//!
//! In a sandbox, the file chooser shares the file through the document portal, at a path such as
//! `/run/user/1000/doc/<id>/Game.exe` that means nothing to prismd, which recognizes a program by
//! its host location. The document portal's `GetHostPaths` tells where the document is on the
//! host. A file chosen outside the document portal is at its host location already.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use prism_ui::ProgramFile;
use zbus::blocking::{Connection, MessageIterator};
use zbus::message::Type;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::{MatchRule, Message};

/// Opens the desktop's file chooser for a program file.
pub trait FileChooser {
    /// Asks the user for a program file, calling `chosen` with its host location, or why there
    /// is none, from any thread.
    fn choose_program(&self, chosen: Box<dyn FnOnce(ProgramFile) + Send>);
}

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const FILE_CHOOSER: &str = "org.freedesktop.portal.FileChooser";
const REQUEST: &str = "org.freedesktop.portal.Request";
const DOCUMENTS: &str = "org.freedesktop.portal.Documents";
const DOCUMENTS_PATH: &str = "/org/freedesktop/portal/documents";
/// Where Flatpak mounts the document portal in every sandbox, besides its own mount point.
const FLATPAK_DOCUMENTS: &str = "/run/flatpak/doc";

/// The desktop portal's file chooser, each on a thread of its own, since it waits for the user.
#[derive(Clone)]
pub struct PortalFileChooser {
    connect: Arc<dyn Fn() -> zbus::Result<Connection> + Send + Sync>,
}

impl PortalFileChooser {
    /// On the session bus.
    pub fn session() -> Self {
        Self {
            connect: Arc::new(Connection::session),
        }
    }

    /// On `connection`, a connection to the session bus.
    pub fn with_connection(connection: Connection) -> Self {
        Self {
            connect: Arc::new(move || Ok(connection.clone())),
        }
    }
}

impl FileChooser for PortalFileChooser {
    fn choose_program(&self, chosen: Box<dyn FnOnce(ProgramFile) + Send>) {
        let connect = self.connect.clone();
        let started = thread::Builder::new()
            .name("prism-file-chooser".into())
            .spawn(move || chosen(choose(&*connect)));
        if let Err(error) = started {
            eprintln!("prism: can't open the file chooser: {error}");
        }
    }
}

/// Asks the file chooser for a program file, and recovers its host location.
fn choose(connect: &dyn Fn() -> zbus::Result<Connection>) -> ProgramFile {
    let connection = match connect() {
        Ok(connection) => connection,
        Err(error) => return unavailable(&error),
    };
    let uri = match open_file(&connection) {
        Ok(Some(uri)) => uri,
        Ok(None) => return ProgramFile::Canceled,
        Err(error) => return unavailable(&error),
    };
    let Some(chosen) = file_path(&uri) else {
        return ProgramFile::Failed(format!(
            "Prism can only add a program file on this computer, not {uri}."
        ));
    };
    let mut mounts = vec![PathBuf::from(FLATPAK_DOCUMENTS)];
    mounts.extend(std::env::var_os("XDG_RUNTIME_DIR").map(|dir| Path::new(&dir).join("doc")));
    mounts.extend(mount_point(&connection).ok());
    match host_location(&chosen, &mounts, |id| host_path(&connection, id)) {
        Ok(path) => ProgramFile::Chosen(path),
        Err(why) => ProgramFile::Failed(why),
    }
}

fn unavailable(error: &zbus::Error) -> ProgramFile {
    ProgramFile::Failed(format!("The file chooser isn't available: {error}"))
}

/// Opens the file chooser, waiting for the user: the URI of the file chosen, or none when the
/// user canceled.
fn open_file(connection: &Connection) -> zbus::Result<Option<String>> {
    static REQUESTS: AtomicU64 = AtomicU64::new(0);
    let token = format!("prism_{}", REQUESTS.fetch_add(1, Ordering::Relaxed));
    let sender = connection
        .unique_name()
        .ok_or_else(|| zbus::Error::Failure("no unique name on the session bus".into()))?;
    // The request's path, which the portal derives from the token, is matched before the call,
    // so its answer can't come first.
    let expected = format!(
        "{PORTAL_PATH}/request/{}/{token}",
        sender.trim_start_matches(':').replace('.', "_")
    );
    let mut answers = responses(connection, &expected)?;
    let options = HashMap::from([
        ("handle_token", Value::from(token.as_str())),
        ("modal", Value::from(true)),
    ]);
    let request: OwnedObjectPath = connection
        .call_method(
            Some(PORTAL),
            PORTAL_PATH,
            Some(FILE_CHOOSER),
            "OpenFile",
            &("", "Choose the program to add", options),
        )?
        .body()
        .deserialize()?;
    // Portals older than handle tokens choose the path themselves.
    if request.as_str() != expected {
        answers = responses(connection, request.as_str())?;
    }
    let message = answers
        .next()
        .ok_or_else(|| zbus::Error::Failure("the file chooser never answered".into()))??;
    response(&message)
}

/// The `Response` signals of the portal request at `path`.
fn responses(connection: &Connection, path: &str) -> zbus::Result<MessageIterator> {
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface(REQUEST)?
        .member("Response")?
        .path(path.to_owned())?
        .build();
    MessageIterator::for_match_rule(rule, connection, None)
}

/// The URI of the first file a file chooser's `Response` chose, or none when the user canceled.
fn response(message: &Message) -> zbus::Result<Option<String>> {
    let (code, results) = message
        .body()
        .deserialize::<(u32, HashMap<String, OwnedValue>)>()?;
    match code {
        0 => {
            let uris = results
                .get("uris")
                .map(|uris| Vec::<String>::try_from(uris.clone()))
                .transpose()?
                .unwrap_or_default();
            Ok(uris.into_iter().next())
        }
        1 => Ok(None),
        _ => Err(zbus::Error::Failure("the file chooser failed".into())),
    }
}

/// Where the document portal is mounted, as it says.
fn mount_point(connection: &Connection) -> zbus::Result<PathBuf> {
    let path: Vec<u8> = connection
        .call_method(
            Some(DOCUMENTS),
            DOCUMENTS_PATH,
            Some(DOCUMENTS),
            "GetMountPoint",
            &(),
        )?
        .body()
        .deserialize()?;
    Ok(bytes_path(&path))
}

/// The host path of the document `id`, if the document portal knows it.
fn host_path(connection: &Connection, id: &str) -> Result<Option<PathBuf>, String> {
    let paths: HashMap<String, Vec<u8>> = connection
        .call_method(
            Some(DOCUMENTS),
            DOCUMENTS_PATH,
            Some(DOCUMENTS),
            "GetHostPaths",
            &(&[id][..],),
        )
        .and_then(|reply| reply.body().deserialize())
        .map_err(|error| error.to_string())?;
    Ok(paths
        .get(id)
        .filter(|path| !path.is_empty())
        .map(|path| bytes_path(path)))
}

/// A path the document portal sent as bytes, which may end in a NUL.
fn bytes_path(bytes: &[u8]) -> PathBuf {
    let bytes = bytes.strip_suffix(&[0]).unwrap_or(bytes);
    PathBuf::from(OsStr::from_bytes(bytes))
}

/// The local path a `file://` URI names, percent-decoded; none for any other URI.
fn file_path(uri: &str) -> Option<PathBuf> {
    let encoded = uri.strip_prefix("file://")?;
    // Only this computer's files: no host, or localhost.
    let encoded = encoded.strip_prefix("localhost").unwrap_or(encoded);
    if !encoded.starts_with('/') {
        return None;
    }
    let mut bytes = Vec::with_capacity(encoded.len());
    let mut rest = encoded.as_bytes();
    while let Some((&byte, after)) = rest.split_first() {
        let decoded = (byte == b'%')
            .then(|| after.get(..2))
            .flatten()
            .and_then(|hex| u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok());
        match decoded {
            Some(decoded) => {
                bytes.push(decoded);
                rest = &after[2..];
            }
            None => {
                bytes.push(byte);
                rest = after;
            }
        }
    }
    Some(PathBuf::from(OsStr::from_bytes(&bytes)))
}

/// The host location of `chosen`, a file the file chooser gave: as it is, unless it's in the
/// document portal under one of `mounts`, whose document `host_paths` tells the host path of.
///
/// A document holds one file or directory, by its own name, as `<mount>/<id>/<name>`, or
/// `<mount>/by-app/<app id>/<id>/<name>` on the host; the host path stands for `<name>`.
pub(crate) fn host_location(
    chosen: &Path,
    mounts: &[PathBuf],
    host_paths: impl FnOnce(&str) -> Result<Option<PathBuf>, String>,
) -> Result<String, String> {
    let name = chosen.file_name().map_or_else(
        || chosen.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let host = match mounts.iter().find(|mount| chosen.starts_with(mount)) {
        None => chosen.to_path_buf(),
        Some(mount) => {
            let missing =
                |why: &str| format!("Prism couldn't find where {name} is on the host: {why}");
            let Some((id, within)) = document(chosen, mount) else {
                return Err(missing("it isn't a file the document portal shares."));
            };
            match host_paths(id) {
                // Joining nothing would end it in a slash.
                Ok(Some(host)) if within.as_os_str().is_empty() => host,
                Ok(Some(host)) => host.join(within),
                Ok(None) => return Err(missing("the document portal doesn't know it.")),
                Err(error) => return Err(missing(&error)),
            }
        }
    };
    host.into_os_string()
        .into_string()
        .map_err(|_| format!("prismd can't take {name}'s location, which isn't valid UTF-8."))
}

/// The id of the document `path` is in, under the document portal's `mount`, and where in the
/// document's file or directory it is; none for a path that's no document's file.
fn document<'a>(path: &'a Path, mount: &Path) -> Option<(&'a str, PathBuf)> {
    let mut parts = path.strip_prefix(mount).ok()?.components();
    let mut id = parts.next()?;
    if id == Component::Normal(OsStr::new("by-app")) {
        parts.next()?;
        id = parts.next()?;
    }
    let Component::Normal(id) = id else {
        return None;
    };
    // The document's own file or directory, which its host path stands for.
    parts.next()?;
    Some((id.to_str()?, parts.collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOUNT: &str = "/run/user/1000/doc";

    fn mounts() -> Vec<PathBuf> {
        vec![PathBuf::from(FLATPAK_DOCUMENTS), PathBuf::from(MOUNT)]
    }

    /// The document portal, which knows document `4f1c2a` as the host's `/home/alice/Games/
    /// Celeste/Celeste` and records which documents it was asked about.
    fn portal(asked: &mut Vec<String>) -> impl FnOnce(&str) -> Result<Option<PathBuf>, String> {
        move |id| {
            asked.push(id.to_owned());
            Ok((id == "4f1c2a").then(|| PathBuf::from("/home/alice/Games/Celeste/Celeste")))
        }
    }

    #[test]
    fn a_file_in_the_document_portal_is_converted_to_its_host_location() {
        for chosen in [
            "/run/user/1000/doc/4f1c2a/Celeste",
            "/run/flatpak/doc/4f1c2a/Celeste",
            "/run/user/1000/doc/by-app/io.github.tobytaken.Prism/4f1c2a/Celeste",
        ] {
            let mut asked = Vec::new();
            assert_eq!(
                host_location(Path::new(chosen), &mounts(), portal(&mut asked)),
                Ok("/home/alice/Games/Celeste/Celeste".to_owned()),
                "{chosen}"
            );
            assert_eq!(asked, ["4f1c2a"], "{chosen}");
        }
    }

    #[test]
    fn a_file_in_a_shared_directory_is_found_within_its_host_path() {
        let chosen =
            Path::new("/run/user/1000/doc/9d00be/Games/Hollow Knight/hollow_knight.x86_64");
        let found = host_location(chosen, &mounts(), |id| {
            assert_eq!(id, "9d00be");
            Ok(Some(PathBuf::from("/mnt/data/Games")))
        });
        assert_eq!(
            found.as_deref(),
            Ok("/mnt/data/Games/Hollow Knight/hollow_knight.x86_64")
        );
    }

    #[test]
    fn a_file_outside_the_document_portal_is_at_its_host_location_already() {
        let mut asked = Vec::new();
        assert_eq!(
            host_location(
                Path::new("/home/alice/Games/factorio/bin/factorio"),
                &mounts(),
                portal(&mut asked)
            ),
            Ok("/home/alice/Games/factorio/bin/factorio".to_owned())
        );
        // Nor is a path that only starts like the mount.
        assert_eq!(
            host_location(
                Path::new("/run/user/1000/docs/a/b"),
                &mounts(),
                portal(&mut asked)
            ),
            Ok("/run/user/1000/docs/a/b".to_owned())
        );
        assert_eq!(asked, Vec::<String>::new());
    }

    #[test]
    fn a_document_without_a_host_path_says_so() {
        let mut asked = Vec::new();
        assert_eq!(
            host_location(
                Path::new("/run/user/1000/doc/0bad00/game.exe"),
                &mounts(),
                portal(&mut asked)
            ),
            Err(
                "Prism couldn't find where game.exe is on the host: the document portal doesn't \
                 know it."
                    .to_owned()
            )
        );
        assert_eq!(
            host_location(
                Path::new("/run/user/1000/doc/0bad00/game.exe"),
                &mounts(),
                |_| Err("No such method “GetHostPaths”".into())
            ),
            Err(
                "Prism couldn't find where game.exe is on the host: No such method \
                 “GetHostPaths”"
                    .to_owned()
            )
        );
        // The mount itself, or a document's directory, names no file of a document.
        for chosen in [MOUNT, "/run/user/1000/doc/4f1c2a"] {
            assert!(
                host_location(Path::new(chosen), &mounts(), |_| Ok(None)).is_err(),
                "{chosen}"
            );
        }
    }

    #[test]
    fn file_uris_are_decoded_and_others_refused() {
        assert_eq!(
            file_path("file:///home/alice/My%20Games/Game%2B1.exe"),
            Some(PathBuf::from("/home/alice/My Games/Game+1.exe"))
        );
        assert_eq!(
            file_path("file://localhost/opt/game"),
            Some(PathBuf::from("/opt/game"))
        );
        // A stray percent sign stays as it is.
        assert_eq!(file_path("file:///a/100%"), Some(PathBuf::from("/a/100%")));
        assert_eq!(
            file_path("file:///a/caf%C3%A9"),
            Some(PathBuf::from("/a/café"))
        );
        for uri in [
            "smb://nas/games/game.exe",
            "file://nas/game.exe",
            "/plain/path",
        ] {
            assert_eq!(file_path(uri), None, "{uri}");
        }
    }
}
