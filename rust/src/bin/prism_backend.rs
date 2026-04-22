use std::io::{self, BufRead, BufReader, Write};

use prism_rust::bridge::{handle_request, invalid_request_response, BridgeRequest};

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    let reader = BufReader::new(stdin.lock());

    for line in reader.lines() {
        let response = match line {
            Ok(payload) if payload.trim().is_empty() => continue,
            Ok(payload) => match serde_json::from_str::<BridgeRequest>(&payload) {
                Ok(request) => handle_request(request),
                Err(error) => invalid_request_response(error.to_string()),
            },
            Err(error) => invalid_request_response(error.to_string()),
        };

        if writeln!(
            stdout,
            "{}",
            serde_json::to_string(&response).expect("serialize bridge response")
        )
        .is_err()
        {
            break;
        }
    }
}
