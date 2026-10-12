//! Seam 1: driver jobs reach every session, in its snapshot and as `DriverJobChanged` with each
//! step, log line and outcome. A job belongs to the host boot it ran in.

use prism_drivers::DriverJobStatus;
use serde_json::{Value, json};

use super::harness::{FakeGpuBackend, FakeSession, Harness, tunable_gpu};

fn daemon() -> Harness {
    Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]))
}

/// The driver jobs pushed to the session since it last looked.
fn pushed_jobs(session: &FakeSession) -> Vec<Value> {
    session
        .pushed_events_of("driver_job_changed")
        .into_iter()
        .map(|event| event["payload"].clone())
        .collect()
}

/// Each pushed job's status and current step.
fn steps(jobs: &[Value]) -> Vec<(String, Value)> {
    jobs.iter()
        .map(|job| {
            (
                job["status"].as_str().unwrap().to_owned(),
                job["current_step"].clone(),
            )
        })
        .collect()
}

fn install(session: &FakeSession) -> Value {
    session.send_json(json!({
        "type": "start_driver_action",
        "payload": { "action": "install_recommended" }
    }))
}

fn snapshot_job(session: &FakeSession) -> Value {
    serde_json::to_value(&session.snapshot().driver_job).unwrap()
}

#[test]
fn a_driver_job_streams_its_progress_and_outcome_to_every_session() {
    let daemon = daemon();
    let alice = daemon.user(1000, "alice");
    let bob = daemon.read_only_user(1001, "bob");

    let started = install(&alice);
    assert_eq!(started["type"], "driver_job", "{started}");
    assert_eq!(started["payload"]["status"], "queued");
    daemon.wait_for_driver_job(DriverJobStatus::Running);
    daemon.drivers.progress(
        "Installing Ubuntu's recommended NVIDIA driver",
        &["Setting up nvidia-driver-595 (595.58.03-0ubuntu1) ..."],
    );
    daemon.drivers.finish_needing_reboot();
    let ended = daemon.wait_for_driver_job(DriverJobStatus::RebootRequired);

    for session in [&alice, &bob] {
        let jobs = pushed_jobs(session);
        let installing = json!("Installing Ubuntu's recommended NVIDIA driver");
        assert_eq!(
            steps(&jobs),
            [
                ("queued".into(), json!("Queued")),
                ("running".into(), json!("Preparing driver action")),
                ("running".into(), installing.clone()),
                ("running".into(), installing),
                ("reboot_required".into(), json!("Reboot required")),
            ]
        );
        assert!(jobs.iter().all(|job| job["id"] == started["payload"]["id"]));
        assert_eq!(
            jobs.last().unwrap()["log_tail"],
            json!([
                "Driver job queued.",
                "Driver job started.",
                "Running Installing Ubuntu's recommended NVIDIA driver",
                "Setting up nvidia-driver-595 (595.58.03-0ubuntu1) ...",
                "Package operation finished. Reboot to load the NVIDIA driver."
            ])
        );
        assert_eq!(jobs.last().unwrap(), &serde_json::to_value(&ended).unwrap());
    }
}

#[test]
fn a_session_starts_from_the_latest_driver_job_of_this_boot() {
    let mut daemon = daemon();
    assert_eq!(snapshot_job(&daemon.user(1000, "alice")), Value::Null);

    install(&daemon.user(1000, "alice"));
    daemon.wait_for_driver_job(DriverJobStatus::Running);
    daemon
        .drivers
        .fail("apt-get update exited with status exit status: 100");
    let failed = daemon.wait_for_driver_job(DriverJobStatus::Failed);

    let later = daemon.read_only_user(1001, "bob");
    assert_eq!(snapshot_job(&later), serde_json::to_value(&failed).unwrap());
    assert_eq!(
        snapshot_job(&later)["error"],
        "apt-get update exited with status exit status: 100"
    );

    daemon.reboot();
    assert_eq!(snapshot_job(&daemon.user(1000, "alice")), Value::Null);
}

#[test]
fn a_driver_job_cut_off_by_a_daemon_restart_reads_as_failed() {
    let mut daemon = daemon();
    install(&daemon.user(1000, "alice"));
    daemon.wait_for_driver_job(DriverJobStatus::Running);

    daemon.crash_daemon();

    let alice = daemon.user(1000, "alice");
    let job = snapshot_job(&alice);
    assert_eq!(job["status"], "failed");
    assert_eq!(
        job["error"],
        "prismd stopped before the driver job finished."
    );
    // Nothing holds the next one back.
    assert_eq!(install(&alice)["payload"]["status"], "queued");
}

#[test]
fn canceling_a_running_driver_job_ends_it_canceled() {
    let daemon = daemon();
    let alice = daemon.user(1000, "alice");
    let job_id = install(&alice)["payload"]["id"].clone();
    daemon.wait_for_driver_job(DriverJobStatus::Running);
    alice.pushed_events();

    let answer = alice.send_json(json!({
        "type": "cancel_driver_job",
        "payload": { "job_id": job_id }
    }));
    daemon.wait_for_driver_job(DriverJobStatus::Canceled);

    assert_eq!(answer["type"], "driver_job", "{answer}");
    let jobs = pushed_jobs(&alice);
    assert_eq!(
        steps(&jobs),
        [
            ("running".into(), json!("Preparing driver action")),
            ("canceled".into(), Value::Null),
        ]
    );
    assert_eq!(jobs.last().unwrap()["error"], "Driver job was canceled.");
}

#[test]
fn a_blocked_driver_plan_starts_no_job() {
    let daemon = daemon();
    daemon.drivers.block_plans("No NVIDIA GPU was detected.");
    let alice = daemon.user(1000, "alice");

    let refused = install(&alice);

    assert_eq!(refused["type"], "error", "{refused}");
    assert_eq!(refused["payload"]["message"], "No NVIDIA GPU was detected.");
    assert_eq!(pushed_jobs(&alice), Vec::<Value>::new());
    assert_eq!(snapshot_job(&daemon.user(1000, "alice")), Value::Null);
}

#[test]
fn only_a_session_that_can_tune_asks_the_host_to_reboot() {
    let daemon = daemon();
    let bob = daemon.read_only_user(1001, "bob");

    let refused = bob.send_json(json!({ "type": "request_host_reboot" }));
    assert_eq!(refused["payload"]["code"], "no_permission", "{refused}");
    assert_eq!(daemon.drivers.reboots(), 0);

    let answer = daemon
        .user(1000, "alice")
        .send_json(json!({ "type": "request_host_reboot" }));
    assert_eq!(
        answer,
        json!({ "type": "command", "payload": { "success": true, "error": null } })
    );
    assert_eq!(daemon.drivers.reboots(), 1);
}
