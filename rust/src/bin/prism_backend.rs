use std::io::{self, BufRead, BufReader, Write};
use std::sync::{Arc, Mutex};

use prism_rust::bridge::{handle_request, invalid_request_response, BridgeRequest};
use prism_rust::daemon_client::SessionNotification;

fn main() {
    let stdin = io::stdin();
    let stdout = Arc::new(Mutex::new(io::stdout()));
    let notification_output = Arc::clone(&stdout);
    let _session = prism_rust::daemon_client::start_session(move |notification| {
        let notification = match notification {
            SessionNotification::DaemonStatus(status) => {
                serde_json::json!({ "method": "daemon.statusChanged", "params": status })
            }
            SessionNotification::RunningGames(state) => {
                serde_json::json!({ "method": "games.runningChanged", "params": state })
            }
            SessionNotification::TuningState(state) => {
                serde_json::json!({ "method": "gpu.tuningStateChanged", "params": state })
            }
        };
        let _ = writeln!(notification_output.lock().unwrap(), "{notification}");
    });
    let reader = BufReader::new(stdin.lock());

    for line in reader.lines() {
        let stdout = Arc::clone(&stdout);
        // Keep reading stdin while a request waits for a GPU. EOF must still close the session.
        std::thread::spawn(move || {
            let response = match line {
                Ok(payload) if payload.trim().is_empty() => return,
                Ok(payload) => match serde_json::from_str::<BridgeRequest>(&payload) {
                    Ok(request) => handle_request(request),
                    Err(error) => invalid_request_response(error.to_string()),
                },
                Err(error) => invalid_request_response(error.to_string()),
            };

            if writeln!(
                stdout.lock().unwrap(),
                "{}",
                serde_json::to_string(&response).expect("serialize bridge response")
            )
            .is_err()
            {
                return;
            }
        });
    }
}
