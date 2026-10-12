//! View tests for Drivers: the recommended driver plan with its blockers and warnings, Run and
//! Reboot only through their confirmations, also from the palette, the driver job's progress, log,
//! Cancel and outcome as prismd pushes them, and one notification when a job ends while the window
//! doesn't have focus.

mod support;

use std::time::Duration;

use i_slint_backend_testing::AccessibleRole;
use prism_drivers::{
    DriverAction, DriverJob, DriverJobRequest, DriverJobStatus, DriverStatusResponse,
    StartDriverActionRequest,
};
use prism_gpu::GpuTelemetryProvider;
use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_protocol::{DaemonCommandResponse, DaemonEvent, DaemonRequest, DaemonResponse};
use slint::platform::Key;

use support::prismd::DaemonState;
use support::{Harness, texts};

use AccessibleRole::{Button, Groupbox, List, ListItem, Text};

/// Prism connected to prismd, with tuning access as `daemon` gives it, and one reading of the
/// tunable GPU.
fn connected_to(daemon: DaemonState) -> Harness {
    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, daemon);
    harness.prismd_tick(
        SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO).performance(),
    );
    harness
}

fn connected() -> Harness {
    connected_to(DaemonState::Running)
}

fn driver_status() -> DriverStatusResponse {
    serde_json::from_str(include_str!(
        "../../prism-protocol/tests/contracts/driver-status.json"
    ))
    .unwrap()
}

/// The contract's driver job in `status`.
fn job(status: DriverJobStatus) -> DriverJob {
    let jobs: Vec<DriverJob> = serde_json::from_str(include_str!(
        "../../prism-protocol/tests/contracts/driver-jobs.json"
    ))
    .unwrap();
    jobs.into_iter()
        .find(|job| job.status == status)
        .expect("the contract has a job in each status")
}

fn push(harness: &Harness, job: DriverJob) {
    harness.prismd.push(DaemonEvent::DriverJobChanged(job));
    harness.app.receive_posted();
}

fn answer(harness: &Harness, response: DaemonResponse) {
    harness.prismd.answer(response);
    harness.app.receive_posted();
}

/// Opens Drivers from the sidebar, with prismd answering its read of the host's driver status
/// with `status`.
fn open_drivers_with(harness: &Harness, status: DriverStatusResponse) {
    harness
        .find(ListItem, "Drivers")
        .invoke_accessible_default_action();
    answer(harness, DaemonResponse::DriverStatus(status));
}

fn open_drivers(harness: &Harness) {
    open_drivers_with(harness, driver_status());
}

fn press(harness: &Harness, role: AccessibleRole, label: &str) {
    harness.find(role, label).invoke_accessible_default_action();
}

/// The requests Prism sent about driver plans, jobs and restarts.
fn driver_requests(harness: &Harness) -> Vec<DaemonRequest> {
    harness
        .prismd
        .requests()
        .into_iter()
        .filter(|request| {
            matches!(
                request,
                DaemonRequest::StartDriverAction(_)
                    | DaemonRequest::CancelDriverJob(_)
                    | DaemonRequest::RequestHostReboot
            )
        })
        .collect()
}

fn install() -> DaemonRequest {
    DaemonRequest::StartDriverAction(StartDriverActionRequest {
        action: DriverAction::InstallRecommended,
    })
}

fn value(harness: &Harness, label: &str) -> String {
    harness
        .find(Text, label)
        .accessible_value()
        .unwrap_or_default()
        .into()
}

/// The lines of the list labeled `label`.
fn lines(harness: &Harness, label: &str) -> Vec<String> {
    texts(&harness.find(List, label))
        .into_iter()
        .filter(|line| line != label)
        .collect()
}

fn enabled(harness: &Harness, label: &str) -> bool {
    harness.find(Button, label).accessible_enabled() == Some(true)
}

/// Runs the palette command titled `title`.
fn run_from_palette(harness: &Harness, title: &str) {
    harness.ctrl("k");
    harness.type_text(title);
    harness.press(&[], Key::Return);
    harness.app.receive_posted();
}

#[test]
fn the_recommended_plan_renders_with_its_warnings_and_the_hosts_facts() {
    let harness = connected();

    open_drivers(&harness);

    assert_eq!(
        harness.prismd.requests().last(),
        Some(&DaemonRequest::GetDriverStatus)
    );
    assert_eq!(
        value(&harness, "Driver plan"),
        "Install recommended NVIDIA driver"
    );
    assert_eq!(
        lines(&harness, "WARNINGS"),
        driver_status().nvidia.plans[0].warnings
    );
    assert_eq!(
        lines(&harness, "AFTERWARD"),
        ["Reboot after installation before expecting the new driver to load."]
    );
    assert_eq!(harness.count(List, "BLOCKED BY"), 0);
    assert_eq!(value(&harness, "NVIDIA driver"), "Not loaded");
    assert_eq!(value(&harness, "Secure Boot"), "On");
    assert_eq!(
        lines(&harness, "HOST WARNINGS"),
        ["NVML could not be loaded: libnvidia-ml.so.1 was not found."]
    );
    assert!(enabled(&harness, "Run recommended driver plan…"));
    assert_eq!(harness.count(Text, "Driver job"), 0);
}

#[test]
fn a_blocked_plan_shows_its_blockers_and_run_says_why_it_cant() {
    let harness = connected();
    let mut status = driver_status();
    for plan in &mut status.nvidia.plans {
        plan.blockers = vec!["No NVIDIA GPU was detected.".into()];
    }

    open_drivers_with(&harness, status);

    assert_eq!(
        lines(&harness, "BLOCKED BY"),
        ["No NVIDIA GPU was detected."]
    );
    let run = harness.find(Button, "Run recommended driver plan…");
    assert_eq!(run.accessible_enabled(), Some(false));
    assert_eq!(
        run.accessible_description().as_deref(),
        Some("Blocked · No NVIDIA GPU was detected.")
    );
}

#[test]
fn without_tuning_access_drivers_is_read_only_with_the_reason() {
    let harness = connected_to(DaemonState::ReadOnly);

    open_drivers(&harness);
    push(&harness, job(DriverJobStatus::RebootRequired));
    // The job ending reads the plan again.
    answer(&harness, DaemonResponse::DriverStatus(driver_status()));

    let reason = harness.app.core().drivers().read_only.expect("read-only");
    assert_eq!(value(&harness, "Read-only"), reason);
    for button in ["Run recommended driver plan…", "Reboot now…"] {
        let button = harness.find(Button, button);
        assert_eq!(button.accessible_enabled(), Some(false));
        assert_eq!(
            button.accessible_description().as_deref(),
            Some(reason.as_str())
        );
    }
}

#[test]
fn run_asks_first_then_the_jobs_progress_log_cancel_and_outcome_render_from_events() {
    let harness = connected();
    open_drivers(&harness);

    press(&harness, Button, "Run recommended driver plan…");

    let dialog = harness.find(Groupbox, "Install recommended NVIDIA driver?");
    assert!(
        dialog
            .accessible_description()
            .unwrap()
            .contains("prismd runs APT as root")
    );
    assert_eq!(driver_requests(&harness), []);
    press(&harness, Button, "Run");
    assert_eq!(driver_requests(&harness), [install()]);
    answer(
        &harness,
        DaemonResponse::DriverJob(job(DriverJobStatus::Queued)),
    );
    assert_eq!(
        harness.count(Groupbox, "Install recommended NVIDIA driver?"),
        0
    );
    // Nothing shows until prismd pushes the job.
    assert_eq!(harness.count(Text, "Driver job"), 0);

    let running = job(DriverJobStatus::Running);
    push(&harness, running.clone());
    assert_eq!(
        value(&harness, "Driver job"),
        "Install recommended NVIDIA driver · Running"
    );
    assert_eq!(
        value(&harness, "Step"),
        "Installing Ubuntu's recommended NVIDIA driver"
    );
    assert_eq!(lines(&harness, "Driver job log"), running.log_tail);
    assert!(!enabled(&harness, "Run recommended driver plan…"));

    press(&harness, Button, "Cancel driver job");
    assert_eq!(
        driver_requests(&harness)[1..],
        [DaemonRequest::CancelDriverJob(DriverJobRequest {
            job_id: running.id.clone()
        })]
    );
    assert!(!enabled(&harness, "Cancel driver job"));

    let ended = job(DriverJobStatus::RebootRequired);
    push(&harness, ended.clone());
    assert_eq!(
        value(&harness, "Driver job"),
        "Install recommended NVIDIA driver · Reboot required"
    );
    assert_eq!(harness.count(Text, "Step"), 0);
    assert_eq!(harness.count(Button, "Cancel driver job"), 0);
    assert_eq!(
        value(&harness, "Outcome"),
        "Package operations finished. Restart the host to load the NVIDIA driver."
    );
    assert_eq!(lines(&harness, "Driver job log"), ended.log_tail);
    assert!(enabled(&harness, "Reboot now…"));
    assert_eq!(harness.app.core().status_bar().problems, 1);
}

#[test]
fn a_manual_step_left_outstanding_is_explained() {
    let harness = connected();
    open_drivers(&harness);

    push(&harness, job(DriverJobStatus::ManualStepRequired));

    assert!(value(&harness, "Outcome").contains("Machine Owner Key enrollment may be required."),);
    assert!(enabled(&harness, "Reboot now…"));
}

#[test]
fn reboot_asks_first_and_then_asks_prismd() {
    let harness = connected();
    open_drivers(&harness);
    push(&harness, job(DriverJobStatus::RebootRequired));
    // The job ending reads the plan again.
    answer(&harness, DaemonResponse::DriverStatus(driver_status()));

    press(&harness, Button, "Reboot now…");
    harness.find(Groupbox, "Reboot now?");
    assert_eq!(driver_requests(&harness), []);
    press(&harness, Button, "Cancel");
    assert_eq!(harness.count(Groupbox, "Reboot now?"), 0);
    assert_eq!(driver_requests(&harness), []);

    press(&harness, Button, "Reboot now…");
    press(&harness, Button, "Reboot");
    assert_eq!(
        driver_requests(&harness),
        [DaemonRequest::RequestHostReboot]
    );
    answer(
        &harness,
        DaemonResponse::Command(DaemonCommandResponse {
            success: true,
            error: None,
        }),
    );
    assert_eq!(harness.count(Groupbox, "Reboot now?"), 0);
}

#[test]
fn the_palette_runs_the_plan_and_reboots_only_through_their_confirmations() {
    let harness = connected();

    run_from_palette(&harness, "Run recommended driver plan…");
    answer(&harness, DaemonResponse::DriverStatus(driver_status()));

    assert_eq!(
        harness.find(ListItem, "Drivers").accessible_item_selected(),
        Some(true)
    );
    harness.find(Groupbox, "Install recommended NVIDIA driver?");
    harness.press(&[], Key::Escape);
    assert_eq!(
        harness.count(Groupbox, "Install recommended NVIDIA driver?"),
        0
    );

    push(&harness, job(DriverJobStatus::RebootRequired));
    run_from_palette(&harness, "Reboot now…");
    harness.find(Groupbox, "Reboot now?");
    assert_eq!(driver_requests(&harness), []);
}

#[test]
fn a_driver_job_ending_notifies_once_while_unfocused_and_the_click_opens_drivers() {
    let harness = connected();
    harness.activation.set_focused(false);
    push(&harness, job(DriverJobStatus::Running));
    assert_eq!(harness.notifier.sent(), []);

    push(&harness, job(DriverJobStatus::Failed));
    push(&harness, job(DriverJobStatus::Failed));

    let [notification] = harness
        .notifier
        .sent()
        .try_into()
        .expect("one notification");
    assert_eq!(notification.summary, "NVIDIA driver job failed");
    harness
        .notifier
        .click(notification.id, Some("token-from-the-server"));
    harness.tick();
    assert_eq!(
        harness.activation.tokens(),
        [Some("token-from-the-server".to_owned())]
    );
    assert_eq!(
        harness.find(ListItem, "Drivers").accessible_item_selected(),
        Some(true)
    );
}
