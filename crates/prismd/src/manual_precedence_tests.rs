//! A manual override's precedence over automatic switching, reserved in the order its request
//! arrives on the socket, before a later request on the connection can let automation write.

use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::Duration;

use prism_protocol::framing::{read_frame, write_frame};
use prism_protocol::{DaemonRequest, DaemonResponse, SessionMessage, SessionRequest};
use serde_json::{Value, json};

use super::harness::{FakeGpuBackend, FakeSession, Harness, Setter, tunable_gpu_with_id};
use super::{SessionPolicy, Work};

const LINKED: &str = "GPU-A";
const OTHER: &str = "GPU-B";
const DOTA: u32 = 570;

/// The test process's own user, as the socket identifies it.
fn me() -> (u32, String) {
    (
        users::get_current_uid(),
        users::get_current_username()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
    )
}

/// A policy that lets the test process's user tune.
fn tuning_policy() -> SessionPolicy {
    SessionPolicy {
        admin_group: "prism-test-no-admin-group".into(),
        admin_user: Some(me().1),
    }
}

/// A policy that doesn't let the test process's user tune, unless they're root.
fn read_only_policy() -> SessionPolicy {
    SessionPolicy {
        admin_group: "prism-test-no-admin-group".into(),
        admin_user: None,
    }
}

/// The test process's user with Dota 2 linked to Gaming on GPU-A and automatic switching on.
/// Daily and Loud, which asks for more power than GPU-A allows, are theirs too, and so is Quiet,
/// on GPU-B.
fn linked_dota() -> (Harness, FakeSession) {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![
        tunable_gpu_with_id(LINKED),
        tunable_gpu_with_id(OTHER),
    ]));
    // Past the grace period after the daemon starts.
    daemon.clock.advance(5_000);
    let (uid, username) = me();
    daemon.install_steam_app(&username, DOTA, "Dota 2");
    let session = daemon.user(uid, &username);
    session.seed_profiles(json!([
        { "id": "daily", "name": "Daily", "gpu_id": LINKED,
          "settings": { "power_limit_watts": value(400), "graphics_offset_mhz": value(60) } },
        { "id": "gaming", "name": "Gaming", "gpu_id": LINKED,
          "settings": { "power_limit_watts": value(350), "graphics_offset_mhz": value(150) } },
        { "id": "loud", "name": "Loud", "gpu_id": LINKED,
          "settings": { "power_limit_watts": value(650), "graphics_offset_mhz": value(60) } },
        { "id": "quiet", "name": "Quiet", "gpu_id": OTHER,
          "settings": { "power_limit_watts": value(300) } }
    ]));
    ok(session.send_json(json!({ "type": "add_steam_game", "payload": { "app_id": DOTA } })));
    ok(
        session.send_json(json!({ "type": "set_game_profile", "payload": {
        "game": { "kind": "steam", "app_id": DOTA }, "gpu_id": LINKED, "profile_id": "gaming"
    } })),
    );
    ok(session
        .send_json(json!({ "type": "set_automatic_switching", "payload": { "enabled": true } })));
    daemon.settle();
    (daemon, session)
}

fn value(value: u32) -> Value {
    json!({ "kind": "value", "value": value })
}

fn ok(response: Value) -> Value {
    assert_ne!(response["type"], "error", "{response}");
    response["payload"].clone()
}

/// GPU-A's power limit and graphics clock offset as the hardware reports them.
fn values(daemon: &Harness) -> (Option<u32>, Option<i32>) {
    let gpu = daemon.gpus.gpu(LINKED);
    (
        gpu.power.current_limit_watts,
        gpu.clocks.graphics.current_offset_mhz,
    )
}

const DAILY: (Option<u32>, Option<i32>) = (Some(400), Some(60));
const CORRECTED_LOUD: (Option<u32>, Option<i32>) = (Some(600), Some(60));
const GAMING: (Option<u32>, Option<i32>) = (Some(350), Some(150));

fn status(session: &FakeSession) -> Value {
    ok(session.send_json(json!({ "type": "get_automatic_switching_status" })))
}

/// A session connection over a socket, whose requests' remaining work waits in `work` until the
/// test runs it.
struct SocketSession {
    client: UnixStream,
    work: Receiver<Work>,
    serving: JoinHandle<Result<(), String>>,
}

impl SocketSession {
    fn open(daemon: &Harness, policy: SessionPolicy) -> Self {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let core = daemon.core();
        let (started, work) = mpsc::channel::<Work>();
        let serving = std::thread::spawn(move || {
            super::handle_connection_starting(&core, &policy, &mut server, &|work| {
                started.send(work).unwrap();
            })
        });
        write_frame(
            &mut client,
            &SessionRequest {
                id: 0,
                request: DaemonRequest::Hello(prism_protocol::HelloRequest::current("0.1.0")),
            },
        )
        .unwrap();
        let mut session = Self {
            client,
            work,
            serving,
        };
        assert!(matches!(session.response(0), DaemonResponse::Welcome(_)));
        session
    }

    /// Sends the requests together, without waiting for any response.
    fn send(&mut self, requests: Vec<(u64, Value)>) {
        for (id, request) in requests {
            let request = serde_json::from_value(request).unwrap();
            write_frame(&mut self.client, &SessionRequest { id, request }).unwrap();
        }
    }

    /// Reads on until the response to request `id`, passing over the snapshot and events.
    fn response(&mut self, id: u64) -> DaemonResponse {
        loop {
            match read_frame::<SessionMessage>(&mut self.client).unwrap() {
                Some(SessionMessage::Response {
                    id: answered,
                    response,
                }) => {
                    assert_eq!(answered, id, "{response:?}");
                    return response;
                }
                Some(_) => {}
                None => panic!("the connection closed before answering request {id}"),
            }
        }
    }

    /// Runs the next request's remaining work on its own thread, as the daemon does.
    fn run_next_work(&self) {
        let work = self
            .work
            .recv_timeout(Duration::from_secs(5))
            .expect("a request left work to run");
        std::thread::spawn(work);
    }

    /// Ends the connection as a closing Prism does, and waits until the daemon notices.
    fn close(self) {
        self.client.shutdown(std::net::Shutdown::Write).unwrap();
        self.serving.join().unwrap().unwrap();
    }
}

fn set_edit_holds(gpu_ids: &[&str]) -> Value {
    json!({ "type": "set_edit_holds", "payload": { "gpu_ids": gpu_ids } })
}

fn apply_profile_by_id(profile_id: &str) -> Value {
    json!({ "type": "apply_profile_by_id", "payload": { "profile_id": profile_id } })
}

/// Lowering the profile's power limit to GPU-A's maximum, then applying it.
fn correct_and_apply_loud() -> Value {
    json!({ "type": "correct_and_apply_profile", "payload": {
        "profile_id": "loud", "corrections": [{ "setting": "power_limit_watts",
            "saved_value": 650, "proposed_value": 600, "bound": "maximum" }]
    } })
}

fn succeeded(response: DaemonResponse) {
    let application = match response {
        DaemonResponse::ProfileApplication(application) => application,
        DaemonResponse::CorrectedProfileApplication(corrected) => corrected.application,
        response => panic!("the profile wasn't applied: {response:?}"),
    };
    assert_eq!(
        application.power.as_ref().map(|outcome| &outcome.status),
        Some(&prism_gpu::TuningOutcomeStatus::Succeeded),
        "{application:?}"
    );
}

#[test]
fn releasing_an_edit_hold_after_a_profile_application_cannot_let_automation_write_first() {
    for (application, applied) in [
        (apply_profile_by_id("daily"), DAILY),
        (correct_and_apply_loud(), CORRECTED_LOUD),
    ] {
        let (daemon, session) = linked_dota();
        let mut socket = SocketSession::open(&daemon, tuning_policy());
        socket.send(vec![(1, set_edit_holds(&[LINKED]))]);
        ok(serde_json::to_value(socket.response(1)).unwrap());
        daemon.run_steam_game(123, me().0, 420, DOTA);
        daemon.scan();
        assert_eq!(
            status(&session)["gpus"][0]["state"]["status"],
            "waiting_for_edits"
        );
        let before = daemon.gpus.writes();

        // Confirming the profile replaces the draft: the application, then the hold's release.
        socket.send(vec![(2, application.clone()), (3, set_edit_holds(&[]))]);
        ok(serde_json::to_value(socket.response(3)).unwrap());
        // Whatever the release queued has run before the application's work starts.
        daemon.settle();
        socket.run_next_work();
        succeeded(socket.response(2));
        daemon.settle();

        assert_eq!(
            daemon.gpus.writes()[before.len()..],
            [
                (LINKED.to_string(), Setter::PowerLimit),
                (LINKED.to_string(), Setter::GraphicsOffset),
            ],
            "only the manual application wrote: {application}"
        );
        assert_eq!(values(&daemon), applied);
        assert_eq!(
            status(&session)["gpus"][0]["state"],
            json!({ "status": "manual_override", "changed_outside_prism": false })
        );
        socket.close();
    }
}

#[test]
fn a_refused_profile_application_leaves_automatic_switching_as_if_it_never_arrived() {
    // Root is always allowed to tune.
    if users::get_current_uid() == 0 {
        return;
    }
    let (daemon, session) = linked_dota();
    let mut socket = SocketSession::open(&daemon, read_only_policy());
    let revision = daemon.revision(LINKED);
    socket.send(vec![(
        1,
        json!({ "type": "apply_profile", "payload": {
            "gpu_id": LINKED, "expected_revision": revision,
            "settings": { "power_limit_watts": value(380) }
        } }),
    )]);
    // The refusal waits to be answered while the game starts.
    daemon.run_steam_game(123, me().0, 420, DOTA);
    daemon.scan();
    assert_eq!(values(&daemon), GAMING);
    assert_eq!(status(&session)["gpus"][0]["state"]["status"], "active");

    socket.run_next_work();
    let DaemonResponse::Error(refused) = socket.response(1) else {
        panic!("a read-only caller's application was allowed")
    };
    assert_eq!(refused.code, prism_gpu::GpuCommandErrorCode::NoPermission);
    socket.close();
}

#[test]
fn a_blocked_profile_application_holds_up_no_other_request_or_the_end_of_its_connection() {
    let (daemon, _session) = linked_dota();
    let mut socket = SocketSession::open(&daemon, tuning_policy());
    let gate = daemon.gpus.block(LINKED, Setter::PowerLimit);
    socket.send(vec![(1, apply_profile_by_id("daily"))]);
    socket.run_next_work();
    gate.wait_until_entered();

    socket.send(vec![(2, apply_profile_by_id("quiet"))]);
    socket.run_next_work();
    succeeded(socket.response(2));
    assert_eq!(daemon.gpus.gpu(OTHER).power.current_limit_watts, Some(300));

    socket.close();
    gate.release();
    daemon.settle();
    assert_eq!(values(&daemon), DAILY);
}
