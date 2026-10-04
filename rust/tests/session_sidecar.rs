use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
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

fn accept_session(listener: &UnixListener) -> UnixStream {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut stream = loop {
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
    let mut sidecar = Sidecar(
        Command::new(env!("CARGO_BIN_EXE_prism_backend"))
            .env("PRISM_SOCKET_PATH", &socket)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let stdout = sidecar.0.stdout.take().unwrap();
    let (sender, lines) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            sender
                .send(serde_json::from_str::<serde_json::Value>(&line.unwrap()).unwrap())
                .unwrap();
        }
    });
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
fn stdin_close_interrupts_a_daemon_that_has_not_answered_the_session_opening() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("prismd.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut sidecar = Sidecar(
        Command::new(env!("CARGO_BIN_EXE_prism_backend"))
            .env("PRISM_SOCKET_PATH", &socket)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut stream = loop {
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
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    read_frame::<SessionRequest>(&mut stream).unwrap().unwrap();
    drop(sidecar.0.stdin.take());
    assert!(read_frame::<SessionRequest>(&mut stream).unwrap().is_none());
}
