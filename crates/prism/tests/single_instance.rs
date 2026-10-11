//! The single-instance bus on a private session bus: a later launch hands its activation token
//! over to the first Prism and exits.

mod support;

use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use prism::Launches;
use prism::single_instance::{BusLaunches, Claim, claim_on};

use support::bus::PrivateBus;

/// Launching Prism on the test's bus.
trait Launch {
    fn launch(&self, activation_token: Option<&str>) -> Claim;
    fn launch_first(&self) -> BusLaunches;
}

impl Launch for PrivateBus {
    fn launch(&self, activation_token: Option<&str>) -> Claim {
        claim_on(self.connect(), activation_token.map(String::from)).unwrap()
    }

    fn launch_first(&self) -> BusLaunches {
        match self.launch(None) {
            Claim::First(launches) => launches,
            Claim::HandedOver => panic!("the first launch should own the name"),
        }
    }
}

/// The activation tokens of later launches, as the first Prism hears of them.
fn listen(launches: &BusLaunches) -> mpsc::Receiver<Option<String>> {
    let (sender, receiver) = mpsc::channel();
    launches.listen(Box::new(move |token| {
        let _ = sender.send(token);
    }));
    receiver
}

const WAIT: Duration = Duration::from_secs(5);

#[test]
fn a_later_launch_hands_its_activation_token_to_the_first_and_exits() {
    let bus = PrivateBus::start();
    let first = bus.launch_first();
    let launched = listen(&first);

    let later = bus.launch(Some("token-from-the-launcher"));

    assert!(matches!(later, Claim::HandedOver));
    assert_eq!(
        launched.recv_timeout(WAIT).unwrap().as_deref(),
        Some("token-from-the-launcher")
    );
}

#[test]
fn a_later_launch_without_a_token_still_reaches_the_first() {
    let bus = PrivateBus::start();
    let first = bus.launch_first();
    let launched = listen(&first);

    assert!(matches!(bus.launch(None), Claim::HandedOver));
    assert_eq!(launched.recv_timeout(WAIT).unwrap(), None);
}

#[test]
fn a_launch_before_the_first_prism_listens_is_kept_for_it() {
    let bus = PrivateBus::start();
    let first = bus.launch_first();

    assert!(matches!(bus.launch(Some("early")), Claim::HandedOver));
    let launched = listen(&first);

    assert_eq!(
        launched.recv_timeout(WAIT).unwrap().as_deref(),
        Some("early")
    );
}

#[test]
fn once_the_first_prism_quits_the_next_launch_is_the_first() {
    let bus = PrivateBus::start();
    drop(bus.launch_first());

    assert!(matches!(bus.launch(None), Claim::First(_)));
}

#[test]
fn launching_prism_again_hands_over_and_exits() {
    let bus = PrivateBus::start();
    let first = bus.launch_first();
    let launched = listen(&first);

    let output = Command::new(env!("CARGO_BIN_EXE_prism"))
        .env("DBUS_SESSION_BUS_ADDRESS", &bus.address)
        .env("XDG_ACTIVATION_TOKEN", "token-from-the-launcher")
        .env_remove("DESKTOP_STARTUP_ID")
        .env_remove("PRISM_SIMULATE")
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        launched.recv_timeout(WAIT).unwrap().as_deref(),
        Some("token-from-the-launcher")
    );
}
