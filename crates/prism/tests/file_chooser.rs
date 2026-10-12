//! Choosing a program file through the desktop portal on a private session bus, with a fake
//! `org.freedesktop.portal.FileChooser` and `org.freedesktop.portal.Documents`: a file chosen
//! natively is at its host location already, and one the document portal shares is converted to
//! its host location before Prism adds it.

mod support;

use std::collections::HashMap;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use prism::{FileChooser, PortalFileChooser};
use prism_ui::ProgramFile;
use zbus::message::Header;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use support::bus::PrivateBus;

const WAIT: Duration = Duration::from_secs(5);
const MOUNT: &str = "/run/user/1000/doc";

/// What the user does in the file chooser.
#[derive(Clone)]
enum User {
    Chooses(&'static str),
    Cancels,
    /// The portal can't open a file chooser at all, such as with no desktop backend.
    CantChoose,
}

/// The portal's file chooser, which answers each request at once as `user` does.
struct FakeFileChooser {
    user: User,
}

#[zbus::interface(name = "org.freedesktop.portal.FileChooser")]
impl FakeFileChooser {
    async fn open_file(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
        _parent_window: String,
        _title: String,
        options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let sender = header.sender().unwrap().as_str();
        let token = String::try_from(options["handle_token"].clone()).unwrap();
        let handle = format!(
            "/org/freedesktop/portal/desktop/request/{}/{token}",
            sender.trim_start_matches(':').replace('.', "_")
        );
        let (code, results) = match &self.user {
            User::Chooses(uri) => (
                0u32,
                HashMap::from([("uris", Value::from(vec![uri.to_string()]))]),
            ),
            User::Cancels => (1, HashMap::new()),
            User::CantChoose => {
                return Err(zbus::fdo::Error::NotSupported(
                    "No file chooser backend".into(),
                ));
            }
        };
        connection
            .emit_signal(
                None::<()>,
                handle.as_str(),
                "org.freedesktop.portal.Request",
                "Response",
                &(code, results),
            )
            .await
            .unwrap();
        Ok(OwnedObjectPath::try_from(handle).unwrap())
    }
}

/// The document portal, mounted at [`MOUNT`], which knows document `abc123` and records which
/// documents it's asked about.
struct FakeDocuments {
    asked: Sender<Vec<String>>,
}

#[zbus::interface(name = "org.freedesktop.portal.Documents")]
impl FakeDocuments {
    fn get_mount_point(&self) -> Vec<u8> {
        [MOUNT.as_bytes(), b"\0"].concat()
    }

    fn get_host_paths(&self, ids: Vec<String>) -> HashMap<String, Vec<u8>> {
        let _ = self.asked.send(ids.clone());
        ids.into_iter()
            .filter(|id| id == "abc123")
            .map(|id| (id, b"/home/alice/Games/Celeste/Celeste\0".to_vec()))
            .collect()
    }
}

/// Chooses a program file on a desktop whose user does as `user` says: what Prism got, and the
/// documents the document portal was asked about.
fn choose(user: User) -> (ProgramFile, Vec<Vec<String>>) {
    let bus = PrivateBus::start();
    let portal = bus.connect();
    let (asked, questions) = mpsc::channel();
    portal
        .object_server()
        .at("/org/freedesktop/portal/desktop", FakeFileChooser { user })
        .unwrap();
    portal
        .object_server()
        .at("/org/freedesktop/portal/documents", FakeDocuments { asked })
        .unwrap();
    portal
        .request_name("org.freedesktop.portal.Desktop")
        .unwrap();
    portal
        .request_name("org.freedesktop.portal.Documents")
        .unwrap();
    let chosen = chosen_with(PortalFileChooser::with_connection(bus.connect()));
    (chosen, questions.try_iter().collect())
}

fn chosen_with(chooser: PortalFileChooser) -> ProgramFile {
    let (sender, chosen) = mpsc::channel();
    chooser.choose_program(Box::new(move |file| {
        let _ = sender.send(file);
    }));
    chosen
        .recv_timeout(WAIT)
        .expect("the file chooser answered")
}

#[test]
fn a_file_chosen_natively_is_at_its_host_location_already() {
    let (chosen, asked) = choose(User::Chooses("file:///home/alice/My%20Games/factorio"));
    assert_eq!(
        chosen,
        ProgramFile::Chosen("/home/alice/My Games/factorio".into())
    );
    assert_eq!(asked, Vec::<Vec<String>>::new());
}

#[test]
fn a_file_the_document_portal_shares_is_converted_to_its_host_location() {
    let (chosen, asked) = choose(User::Chooses("file:///run/user/1000/doc/abc123/Celeste"));
    assert_eq!(
        chosen,
        ProgramFile::Chosen("/home/alice/Games/Celeste/Celeste".into())
    );
    assert_eq!(asked, [["abc123"]]);
}

#[test]
fn a_document_without_a_host_path_says_so() {
    let (chosen, _) = choose(User::Chooses("file:///run/user/1000/doc/0bad00/game.exe"));
    assert_eq!(
        chosen,
        ProgramFile::Failed(
            "Prism couldn't find where game.exe is on the host: the document portal doesn't know \
             it."
            .into()
        )
    );
}

#[test]
fn canceling_chooses_nothing() {
    assert_eq!(choose(User::Cancels).0, ProgramFile::Canceled);
}

#[test]
fn a_portal_that_cant_choose_says_why() {
    let ProgramFile::Failed(why) = choose(User::CantChoose).0 else {
        panic!("expected a failure");
    };
    assert!(
        why.starts_with("The file chooser isn't available: ") && why.contains("No file chooser"),
        "{why}"
    );
}
