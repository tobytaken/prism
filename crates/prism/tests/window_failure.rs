//! Prism started where it can't create its window, here a display with no OpenGL: it says why
//! and exits, rather than running on invisibly and turning every later launch away (#166).

mod support;

use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use prism::single_instance::{Claim, claim_on};

use support::bus::PrivateBus;
use support::display::PrivateDisplay;

/// Well past how long Prism takes to notice it has no window.
const WAIT: Duration = Duration::from_secs(30);

#[test]
fn prism_without_opengl_says_why_and_exits_for_the_next_launch() {
    let bus = PrivateBus::start();
    let display = PrivateDisplay::without_opengl();
    let config = tempfile::tempdir().unwrap();

    let prism = display
        .show(&mut Command::new(env!("CARGO_BIN_EXE_prism")))
        .env("DBUS_SESSION_BUS_ADDRESS", &bus.address)
        .env("XDG_CONFIG_HOME", config.path())
        .env_remove("PRISM_SIMULATE")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let output = exited_within(prism, WAIT);

    assert!(!output.status.success(), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("prism: can't create the window"),
        "{stderr}"
    );
    assert!(matches!(claim_on(bus.connect(), None), Ok(Claim::First(_))));
}

/// What `child` printed once it exited, failing the test if it runs for longer than `wait`.
fn exited_within(mut child: Child, wait: Duration) -> Output {
    let deadline = Instant::now() + wait;
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!("prism was still running after {wait:?}: {output:?}");
        }
        thread::sleep(Duration::from_millis(100));
    }
    child.wait_with_output().unwrap()
}
