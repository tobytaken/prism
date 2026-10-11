//! A session bus of the test's own, run by `dbus-daemon`.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use zbus::blocking::Connection;
use zbus::blocking::connection::Builder;

pub struct PrivateBus {
    daemon: Child,
    pub address: String,
    _directory: tempfile::TempDir,
}

impl PrivateBus {
    pub fn start() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("bus");
        let mut daemon = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address"])
            .arg(format!("--address=unix:path={}", socket.display()))
            .stdout(Stdio::piped())
            .spawn()
            .expect("run dbus-daemon, from the dbus package, for the test's session bus");
        let mut address = String::new();
        BufReader::new(daemon.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        Self {
            daemon,
            address: address.trim().to_owned(),
            _directory: directory,
        }
    }

    pub fn connect(&self) -> Connection {
        Builder::address(self.address.as_str())
            .unwrap()
            .build()
            .unwrap()
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}
