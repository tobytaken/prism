use std::io::{BufRead, BufReader, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use prism_rust::framing::{read_frame, write_frame};
use prism_rust::gpu::{GpuDaemonStatus, TuningStoreHealth};
use prism_rust::protocol::{DaemonRequest, DaemonResponse, SessionMessage, SessionRequest};

struct Sidecar(Child);
impl Drop for Sidecar {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_sidecar(socket: &Path, stdout: Stdio) -> Sidecar {
    Sidecar(
        Command::new(env!("CARGO_BIN_EXE_prism_backend"))
            .env("PRISM_SOCKET_PATH", socket)
            .stdin(Stdio::piped())
            .stdout(stdout)
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    )
}

fn output_lines(sidecar: &mut Sidecar) -> mpsc::Receiver<serde_json::Value> {
    let stdout = sidecar.0.stdout.take().unwrap();
    let (sender, lines) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if sender
                .send(serde_json::from_str::<serde_json::Value>(&line).unwrap())
                .is_err()
            {
                break;
            }
        }
    });
    lines
}

fn accept_stream(listener: &UnixListener) -> UnixStream {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "sidecar never connected"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("{error}"),
        }
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
}

fn accept_session(listener: &UnixListener) -> UnixStream {
    let mut stream = accept_stream(listener);
    let request = read_frame::<SessionRequest>(&mut stream).unwrap().unwrap();
    assert_eq!(request.request, DaemonRequest::OpenSession);
    write_frame(
        &mut stream,
        &SessionMessage::Response {
            id: request.id,
            response: DaemonResponse::DaemonStatus(GpuDaemonStatus::connected(
                "/test/prismd.sock",
                TuningStoreHealth::Healthy {},
            )),
        },
    )
    .unwrap();
    stream
}

#[test]
fn sidecar_forwards_notifications_reconnects_and_closes_its_session_when_stdin_closes() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("prismd.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut sidecar = spawn_sidecar(&socket, Stdio::piped());
    let lines = output_lines(&mut sidecar);
    let mut first = accept_session(&listener);
    let connected = lines.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(connected["method"], "daemon.statusChanged");
    assert_eq!(connected["params"]["reachable"], true);
    assert!(connected.get("id").is_none());
    let status: GpuDaemonStatus = serde_json::from_str(include_str!(
        "../../src/test/contracts/daemon-status-unhealthy-store.json"
    ))
    .unwrap();
    write_frame(
        &mut first,
        &SessionMessage::Event {
            event: prism_rust::protocol::DaemonEvent::StoreHealthChanged(
                status.tuning_store.clone().unwrap(),
            ),
        },
    )
    .unwrap();
    assert_eq!(
        lines.recv_timeout(Duration::from_secs(2)).unwrap()["params"]["tuningStore"],
        serde_json::to_value(status.tuning_store).unwrap()
    );
    first.shutdown(std::net::Shutdown::Both).unwrap();
    assert_eq!(
        lines.recv_timeout(Duration::from_secs(2)).unwrap()["params"]["reachable"],
        false
    );
    let mut resumed = accept_session(&listener);
    assert_eq!(
        lines.recv_timeout(Duration::from_secs(2)).unwrap()["params"]["reachable"],
        true
    );
    write_frame(
        &mut resumed,
        &serde_json::from_str::<SessionMessage>(include_str!(
            "../../src/test/contracts/session-store-health.json"
        ))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        lines.recv_timeout(Duration::from_secs(2)).unwrap()["params"]["tuningStore"]["status"],
        "healthy"
    );
    let states: serde_json::Value = serde_json::from_str(include_str!(
        "../../src/test/contracts/gpu-tuning-states.json"
    ))
    .unwrap();
    write_frame(
        &mut resumed,
        &serde_json::json!({
            "type": "event", "event": { "type": "tuningStateChanged", "payload": states[0] }
        }),
    )
    .unwrap();
    assert_eq!(
        lines.recv_timeout(Duration::from_secs(2)).unwrap(),
        serde_json::json!({ "method": "gpu.tuningStateChanged", "params": states[0] })
    );
    let running: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../src/test/contracts/running-games.json")).unwrap();
    for state in running {
        let event = serde_json::json!({"type":"event", "event": {"type":"runningGamesChanged", "payload":state}});
        let typed: SessionMessage = serde_json::from_value(event.clone()).unwrap();
        assert_eq!(serde_json::to_value(&typed).unwrap(), event);
        write_frame(&mut resumed, &typed).unwrap();
        assert_eq!(
            lines.recv_timeout(Duration::from_secs(2)).unwrap(),
            serde_json::json!({"method":"games.runningChanged", "params":state})
        );
        writeln!(
            sidecar.0.stdin.as_mut().unwrap(),
            "{}",
            serde_json::json!({"id":8, "method":"games.getRunning"})
        )
        .unwrap();
        let request = read_frame::<SessionRequest>(&mut resumed).unwrap().unwrap();
        assert_eq!(request.request, DaemonRequest::GetRunningGames);
        write_frame(
            &mut resumed,
            &SessionMessage::Response {
                id: request.id,
                response: DaemonResponse::RunningGames(
                    serde_json::from_value(state.clone()).unwrap(),
                ),
            },
        )
        .unwrap();
        assert_eq!(
            lines.recv_timeout(Duration::from_secs(2)).unwrap(),
            serde_json::json!({"id":8,"result":state})
        );
    }
    writeln!(sidecar.0.stdin.as_mut().unwrap(), "{}", serde_json::json!({"id": 9, "method": "gpu.setPowerLimit", "params": {"gpuId": "GPU-123", "limitWatts": 450}})).unwrap();
    writeln!(sidecar.0.stdin.as_mut().unwrap(), "{}", serde_json::json!({"id": 10, "method": "gpu.setPowerLimit", "params": {"gpuId": "GPU-123", "limitWatts": 451}})).unwrap();
    let one = read_frame::<SessionRequest>(&mut resumed).unwrap().unwrap();
    let two = read_frame::<SessionRequest>(&mut resumed).unwrap().unwrap();
    for request in [two, one] {
        let DaemonRequest::SetPowerLimit(power) = request.request else {
            panic!("expected a tuning request on the session")
        };
        let success = match power.limit_watts {
            450 => true,
            451 => false,
            _ => panic!("unexpected requested limit"),
        };
        write_frame(
            &mut resumed,
            &SessionMessage::Response {
                id: request.id,
                response: DaemonResponse::Command(prism_rust::protocol::DaemonCommandResponse {
                    success,
                    error: None,
                }),
            },
        )
        .unwrap();
    }
    let mut replies = vec![
        lines.recv_timeout(Duration::from_secs(2)).unwrap(),
        lines.recv_timeout(Duration::from_secs(2)).unwrap(),
    ];
    replies.sort_by_key(|reply| reply["id"].as_u64());
    assert_eq!(
        replies,
        vec![
            serde_json::json!({"id": 9, "result": {"success": true, "error": null}}),
            serde_json::json!({"id": 10, "result": {"success": false, "error": null}})
        ]
    );
    writeln!(sidecar.0.stdin.as_mut().unwrap(), "{}", serde_json::json!({"id": 11, "method": "gpu.setPowerLimit", "params": {"gpuId": "GPU-123", "limitWatts": 452}})).unwrap();
    read_frame::<SessionRequest>(&mut resumed).unwrap().unwrap();
    drop(sidecar.0.stdin.take());
    assert!(read_frame::<SessionRequest>(&mut resumed)
        .unwrap()
        .is_none());
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(status) = sidecar.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "sidecar stayed alive after stdin closed"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn profile_management_crosses_the_sidecar_session_by_id_and_preserves_daemon_errors() {
    use serde_json::json;
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("prismd.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut sidecar = spawn_sidecar(&socket, Stdio::piped());
    let lines = output_lines(&mut sidecar);
    let mut session = accept_session(&listener);
    assert_eq!(
        lines.recv_timeout(Duration::from_secs(2)).unwrap()["method"],
        "daemon.statusChanged"
    );
    let profiles: Vec<prism_rust::protocol::GpuTuningProfile> = serde_json::from_str(include_str!(
        "../../src/test/contracts/gpu-tuning-profiles.json"
    ))
    .unwrap();
    let mut renamed = profiles[0].clone();
    renamed.name = "Silent".into();
    for (id, method, params, expected, response) in [
        (
            9,
            "gpu.renameProfile",
            json!({ "profileId": renamed.id, "name": "  Silent  " }),
            json!({ "type": "rename_profile", "payload": { "profileId": renamed.id, "name": "  Silent  " } }),
            DaemonResponse::Profile(renamed.clone()),
        ),
        (
            10,
            "gpu.deleteProfile",
            json!({ "profileId": renamed.id }),
            json!({ "type": "delete_profile", "payload": { "profileId": renamed.id } }),
            DaemonResponse::Command(prism_rust::protocol::DaemonCommandResponse {
                success: true,
                error: None,
            }),
        ),
        (
            11,
            "gpu.deleteProfile",
            json!({ "profileId": renamed.id }),
            json!({ "type": "delete_profile", "payload": { "profileId": renamed.id } }),
            DaemonResponse::Error(prism_rust::gpu::GpuCommandError::new(
                prism_rust::gpu::GpuCommandErrorCode::NotFound,
                "The GPU tuning profile no longer exists.",
            )),
        ),
    ] {
        writeln!(
            sidecar.0.stdin.as_mut().unwrap(),
            "{}",
            json!({ "id": id, "method": method, "params": params })
        )
        .unwrap();
        let received = read_frame::<SessionRequest>(&mut session).unwrap().unwrap();
        assert_eq!(serde_json::to_value(received.request).unwrap(), expected);
        write_frame(
            &mut session,
            &SessionMessage::Response {
                id: received.id,
                response,
            },
        )
        .unwrap();
        let reply = lines.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(reply["id"], id);
        match id {
            9 => assert_eq!(reply["result"], serde_json::to_value(&renamed).unwrap()),
            10 => assert_eq!(reply["result"], json!({ "success": true, "error": null })),
            11 => assert_eq!(
                reply["error"],
                json!({ "code": "not_found", "message": "The GPU tuning profile no longer exists." })
            ),
            _ => unreachable!(),
        }
    }
}

#[test]
fn game_requests_cross_the_sidecar_session_and_preserve_daemon_errors() {
    use serde_json::json;
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("prismd.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut sidecar = spawn_sidecar(&socket, Stdio::piped());
    let lines = output_lines(&mut sidecar);
    let mut session = accept_session(&listener);
    assert_eq!(
        lines.recv_timeout(Duration::from_secs(2)).unwrap()["method"],
        "daemon.statusChanged"
    );
    let state: serde_json::Value =
        serde_json::from_str(include_str!("../../src/test/contracts/games-state.json")).unwrap();
    let games = || DaemonResponse::Games(serde_json::from_value(state.clone()).unwrap());
    let dota = json!({ "kind": "steam", "appId": 570 });
    let apps = json!([{ "appId": 570, "name": "Dota 2" }]);
    for (id, method, params, expected, response, reply) in [
        (
            20,
            "games.listSteamApps",
            None,
            json!({ "type": "list_steam_apps" }),
            DaemonResponse::SteamApps(serde_json::from_value(apps.clone()).unwrap()),
            json!({ "result": apps }),
        ),
        (
            21,
            "games.get",
            None,
            json!({ "type": "get_games" }),
            games(),
            json!({ "result": state }),
        ),
        (
            22,
            "games.addSteamGame",
            Some(json!({ "appId": 570 })),
            json!({ "type": "add_steam_game", "payload": { "appId": 570 } }),
            games(),
            json!({ "result": state }),
        ),
        (
            23,
            "games.setProfile",
            Some(json!({ "game": dota, "gpuId": "GPU-123", "profileId": null })),
            json!({ "type": "set_game_profile", "payload": { "game": dota, "gpuId": "GPU-123", "profileId": null } }),
            games(),
            json!({ "result": state }),
        ),
        (
            24,
            "games.remove",
            Some(json!({ "game": dota })),
            json!({ "type": "remove_game", "payload": { "game": dota } }),
            games(),
            json!({ "result": state }),
        ),
        (
            25,
            "games.setAutomaticSwitching",
            Some(json!({ "enabled": true })),
            json!({ "type": "set_automatic_switching", "payload": { "enabled": true } }),
            DaemonResponse::Error(prism_rust::gpu::GpuCommandError::new(
                prism_rust::gpu::GpuCommandErrorCode::Io,
                "Failed to write Prism's tuning store.",
            )),
            json!({ "error": { "code": "io", "message": "Failed to write Prism's tuning store." } }),
        ),
    ] {
        let mut request = json!({ "id": id, "method": method });
        if let Some(params) = params {
            request["params"] = params;
        }
        writeln!(sidecar.0.stdin.as_mut().unwrap(), "{request}").unwrap();
        let received = read_frame::<SessionRequest>(&mut session).unwrap().unwrap();
        assert_eq!(serde_json::to_value(received.request).unwrap(), expected);
        write_frame(
            &mut session,
            &SessionMessage::Response {
                id: received.id,
                response,
            },
        )
        .unwrap();
        let mut expected_reply = reply;
        expected_reply["id"] = json!(id);
        assert_eq!(
            lines.recv_timeout(Duration::from_secs(2)).unwrap(),
            expected_reply
        );
    }
}

#[test]
fn stdin_close_interrupts_a_daemon_that_has_not_answered_the_session_opening() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("prismd.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut sidecar = spawn_sidecar(&socket, Stdio::null());
    let mut stream = accept_stream(&listener);
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    read_frame::<SessionRequest>(&mut stream).unwrap().unwrap();
    drop(sidecar.0.stdin.take());
    assert!(read_frame::<SessionRequest>(&mut stream).unwrap().is_none());
}

#[test]
fn tuning_requests_wait_for_the_first_session_to_finish_opening() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("prismd.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut sidecar = spawn_sidecar(&socket, Stdio::piped());
    let lines = output_lines(&mut sidecar);
    let mut stream = accept_stream(&listener);
    let opening = read_frame::<SessionRequest>(&mut stream).unwrap().unwrap();
    writeln!(sidecar.0.stdin.as_mut().unwrap(), "{}", serde_json::json!({"id": 9, "method": "gpu.setPowerLimit", "params": {"gpuId": "GPU-123", "limitWatts": 450}})).unwrap();
    assert!(
        matches!(
            lines.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ),
        "request failed before session opening finished"
    );
    write_frame(
        &mut stream,
        &SessionMessage::Response {
            id: opening.id,
            response: DaemonResponse::DaemonStatus(GpuDaemonStatus::connected(
                "/test/prismd.sock",
                TuningStoreHealth::Healthy {},
            )),
        },
    )
    .unwrap();
    let request = read_frame::<SessionRequest>(&mut stream).unwrap().unwrap();
    assert!(matches!(request.request, DaemonRequest::SetPowerLimit(_)));
    write_frame(
        &mut stream,
        &SessionMessage::Response {
            id: request.id,
            response: DaemonResponse::Command(prism_rust::protocol::DaemonCommandResponse {
                success: true,
                error: None,
            }),
        },
    )
    .unwrap();
    assert_eq!(
        lines.recv_timeout(Duration::from_secs(2)).unwrap()["method"],
        "daemon.statusChanged"
    );
    assert_eq!(
        lines.recv_timeout(Duration::from_secs(2)).unwrap(),
        serde_json::json!({"id": 9, "result": {"success": true, "error": null}})
    );
}

#[test]
fn session_rejection_keeps_the_daemons_permission_error() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("prismd.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut sidecar = spawn_sidecar(&socket, Stdio::piped());
    let lines = output_lines(&mut sidecar);
    let mut stream = accept_stream(&listener);
    let opening = read_frame::<SessionRequest>(&mut stream).unwrap().unwrap();
    write_frame(
        &mut stream,
        &SessionMessage::Response {
            id: opening.id,
            response: DaemonResponse::Error(prism_rust::gpu::GpuCommandError::new(
                prism_rust::gpu::GpuCommandErrorCode::NoPermission,
                "User bob is not allowed to access prismd (primary group: users)",
            )),
        },
    )
    .unwrap();
    assert_eq!(
        lines.recv_timeout(Duration::from_secs(2)).unwrap()["params"]["reachable"],
        false
    );
    writeln!(sidecar.0.stdin.as_mut().unwrap(), "{}", serde_json::json!({"id": 9, "method": "gpu.setPowerLimit", "params": {"gpuId": "GPU-123", "limitWatts": 450}})).unwrap();
    assert_eq!(
        lines.recv_timeout(Duration::from_secs(3)).unwrap(),
        serde_json::json!({"id": 9, "error": {"code": "no_permission", "message": "User bob is not allowed to access prismd (primary group: users)"}})
    );
}

#[test]
fn stdin_close_exits_even_when_notification_output_is_blocked() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("prismd.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut sidecar = spawn_sidecar(&socket, Stdio::piped());
    let mut stream = accept_session(&listener);
    write_frame(
        &mut stream,
        &SessionMessage::Event {
            event: prism_rust::protocol::DaemonEvent::StoreHealthChanged(
                TuningStoreHealth::Unhealthy {
                    problem: prism_rust::gpu::TuningStoreProblem::Unreadable,
                    path: "/store".into(),
                    detail: "x".repeat(512 * 1024),
                },
            ),
        },
    )
    .unwrap();
    let stdout_fd = sidecar.0.stdout.as_ref().unwrap().as_raw_fd();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let mut unread: libc::c_int = 0;
        assert_eq!(
            unsafe { libc::ioctl(stdout_fd, libc::FIONREAD, &mut unread) },
            0
        );
        if unread >= 4096 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "notification never reached stdout"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(sidecar.0.stdin.take());
    assert!(read_frame::<SessionRequest>(&mut stream).unwrap().is_none());
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    loop {
        if let Some(status) = sidecar.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "sidecar stayed alive with blocked stdout after stdin closed"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_manual_write_can_wait_behind_startup_longer_than_the_old_two_second_timeout() {
    use serde_json::json;
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("prismd.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut sidecar = spawn_sidecar(&socket, Stdio::piped());
    let lines = output_lines(&mut sidecar);
    let mut session = accept_session(&listener);
    lines.recv_timeout(Duration::from_secs(2)).unwrap();
    writeln!(sidecar.0.stdin.as_mut().unwrap(), "{}", json!({
        "id": 9, "method": "gpu.setPowerLimit", "params": { "gpuId": "GPU-123", "limitWatts": 350 }
    })).unwrap();
    let request = read_frame::<SessionRequest>(&mut session).unwrap().unwrap();
    std::thread::sleep(Duration::from_millis(2200));
    write_frame(
        &mut session,
        &SessionMessage::Response {
            id: request.id,
            response: DaemonResponse::Command(prism_rust::protocol::DaemonCommandResponse {
                success: true,
                error: None,
            }),
        },
    )
    .unwrap();
    let response = lines.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(
        response,
        json!({ "id": 9, "result": { "success": true, "error": null } })
    );
}

#[test]
fn startup_choices_and_outcomes_cross_the_sidecar_session() {
    use prism_rust::protocol::{DaemonCommandResponse, StartupState};
    use serde_json::json;
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("prismd.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut sidecar = spawn_sidecar(&socket, Stdio::piped());
    let lines = output_lines(&mut sidecar);
    let mut session = accept_session(&listener);
    lines.recv_timeout(Duration::from_secs(2)).unwrap();
    let state: StartupState =
        serde_json::from_str(include_str!("../../src/test/contracts/startup-state.json")).unwrap();
    for (id, method, params, expected, response) in [
        (
            9,
            "gpu.getStartupState",
            None,
            json!({ "type": "get_startup_state" }),
            DaemonResponse::StartupState(state.clone()),
        ),
        (
            10,
            "gpu.setStartupBehavior",
            Some(json!({ "behavior": "startupProfiles" })),
            json!({ "type": "set_startup_behavior", "payload": { "behavior": "startupProfiles" } }),
            DaemonResponse::Command(DaemonCommandResponse {
                success: true,
                error: None,
            }),
        ),
        (
            11,
            "gpu.setStartupProfile",
            Some(json!({ "gpuId": "GPU-123", "profileId": null })),
            json!({ "type": "set_startup_profile", "payload": { "gpuId": "GPU-123", "profileId": null } }),
            DaemonResponse::Command(DaemonCommandResponse {
                success: true,
                error: None,
            }),
        ),
        (
            12,
            "gpu.dismissStartupOutcome",
            Some(json!({ "gpuId": "GPU-123" })),
            json!({ "type": "dismiss_startup_outcome", "payload": { "gpuId": "GPU-123" } }),
            DaemonResponse::Command(DaemonCommandResponse {
                success: true,
                error: None,
            }),
        ),
    ] {
        writeln!(
            sidecar.0.stdin.as_mut().unwrap(),
            "{}",
            json!({ "id": id, "method": method, "params": params })
        )
        .unwrap();
        let request = read_frame::<SessionRequest>(&mut session).unwrap().unwrap();
        assert_eq!(serde_json::to_value(request.request).unwrap(), expected);
        let result = if id == 9 {
            serde_json::to_value(&state).unwrap()
        } else {
            json!({ "success": true, "error": null })
        };
        write_frame(
            &mut session,
            &SessionMessage::Response {
                id: request.id,
                response,
            },
        )
        .unwrap();
        assert_eq!(
            lines.recv_timeout(Duration::from_secs(2)).unwrap(),
            json!({ "id": id, "result": result })
        );
    }
}
