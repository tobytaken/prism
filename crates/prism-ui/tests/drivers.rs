//! Drivers: the recommended driver plan with its blockers and warnings, Run and Reboot only after a
//! confirmation, the driver job's progress, log and outcome as prismd pushes them, Cancel while
//! it's active, a notification when it ends, and the reboot or manual step it leaves outstanding as
//! a problem in Activity.

use std::time::Duration;

use prism_drivers::{
    DriverAction, DriverJob, DriverJobRequest, DriverJobStatus, DriverStatusResponse,
    StartDriverActionRequest,
};
use prism_gpu::{GpuCommandError, GpuCommandErrorCode};
use prism_protocol::{
    DaemonCommandResponse, DaemonEvent, DaemonRequest, DaemonResponse, DaemonSnapshot,
    ProtocolVersion, SessionMessage, Welcome,
};
use prism_ui::{
    ActivityAction, AppearanceMode, CommandId, Content, Core, Dialog, DriverConfirmation,
    DriversSection, Effect, Input, Intent, Notification, Section, SessionFailure,
};

fn snapshot() -> DaemonSnapshot {
    let message: SessionMessage = serde_json::from_str(include_str!(
        "../../prism-protocol/tests/contracts/daemon-snapshot.json"
    ))
    .unwrap();
    let SessionMessage::Snapshot { snapshot } = message else {
        panic!("the contract holds a snapshot")
    };
    snapshot
}

/// The host's driver status from the shared contract: Ubuntu without an NVIDIA driver loaded.
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

fn welcome(can_tune: bool) -> Welcome {
    Welcome {
        protocol: ProtocolVersion::CURRENT,
        prismd_version: "0.1.0".into(),
        can_tune,
        reason: (!can_tune).then(|| "bob isn't in group wheel".into()),
        authorization: None,
    }
}

fn launched() -> Core {
    let mut core = Core::default();
    core.handle(Input::Launched {
        appearance: AppearanceMode::System,
        started_unix_ms: 0,
    });
    core
}

/// Prism connected to prismd from `snapshot`, with tuning access as `can_tune` says and its window
/// without focus.
fn connected_from(snapshot: DaemonSnapshot, can_tune: bool) -> (Core, Vec<Effect>) {
    let mut core = launched();
    let effects = core.handle(Input::PrismdConnected {
        welcome: welcome(can_tune),
        snapshot,
        socket: "/run/prism/prismd.sock".into(),
    });
    (core, effects)
}

fn connected_with(can_tune: bool) -> Core {
    connected_from(snapshot(), can_tune).0
}

fn connected() -> Core {
    connected_with(true)
}

fn intent(core: &mut Core, intent: Intent) -> Vec<Effect> {
    core.handle(Input::Intent(intent))
}

fn push(core: &mut Core, job: DriverJob) -> Vec<Effect> {
    core.handle(Input::PrismdEvent {
        event: DaemonEvent::DriverJobChanged(job),
        received_at: Duration::from_secs(2),
    })
}

fn answer(core: &mut Core, id: CommandId, response: DaemonResponse) -> Vec<Effect> {
    core.handle(Input::PrismdAnswered { id, response })
}

/// The one command among `effects`, with its id.
fn command(effects: &[Effect]) -> (CommandId, DaemonRequest) {
    let commands: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::CommandPrismd { id, request } => Some((*id, request.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(commands.len(), 1, "{effects:?}");
    commands[0].clone()
}

fn commands(effects: &[Effect]) -> Vec<DaemonRequest> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::CommandPrismd { request, .. } => Some(request.clone()),
            _ => None,
        })
        .collect()
}

fn notifications(effects: &[Effect]) -> Vec<Notification> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Notify(notification) => Some(notification.clone()),
            _ => None,
        })
        .collect()
}

/// Opens Drivers, and answers prismd's read of the host's driver status with `status`.
fn open_drivers_with(core: &mut Core, status: DriverStatusResponse) {
    let effects = intent(core, Intent::GoTo(Section::Drivers));
    let (id, request) = command(&effects);
    assert_eq!(request, DaemonRequest::GetDriverStatus);
    answer(core, id, DaemonResponse::DriverStatus(status));
}

fn open_drivers(core: &mut Core) {
    open_drivers_with(core, driver_status());
}

/// The confirmation over the window, as `kind` picks it out.
fn confirmation(core: &Core, kind: fn(Dialog) -> Option<DriverConfirmation>) -> DriverConfirmation {
    kind(core.dialog().expect("a dialog is open")).expect("the dialog is the confirmation")
}

fn run_confirmation(dialog: Dialog) -> Option<DriverConfirmation> {
    match dialog {
        Dialog::RunDriverPlan(confirmation) => Some(confirmation),
        _ => None,
    }
}

fn reboot_confirmation(dialog: Dialog) -> Option<DriverConfirmation> {
    match dialog {
        Dialog::Reboot(confirmation) => Some(confirmation),
        _ => None,
    }
}

/// Runs the palette's command titled `title`, after checking it's listed.
fn run_palette(core: &mut Core, title: &str) -> Vec<Effect> {
    intent(core, Intent::OpenPalette);
    intent(core, Intent::FilterPalette(title.into()));
    let index = palette_entry_index(core, title);
    intent(core, Intent::RunPaletteEntry(index))
}

fn palette_entry_index(core: &Core, title: &str) -> usize {
    core.palette()
        .expect("the palette is open")
        .entries
        .iter()
        .position(|entry| entry.title == title)
        .unwrap_or_else(|| panic!("the palette lists {title:?}"))
}

/// The palette's entry titled `title`, with why it can't run, if it's listed.
fn palette_entry(core: &mut Core, title: &str) -> Option<Option<String>> {
    intent(core, Intent::OpenPalette);
    let entry = core
        .palette()
        .expect("the palette is open")
        .entries
        .into_iter()
        .find(|entry| entry.title == title)
        .map(|entry| entry.unavailable);
    intent(core, Intent::Dismiss);
    entry
}

fn drivers(core: &Core) -> DriversSection {
    core.drivers()
}

#[test]
fn opening_drivers_reads_and_shows_the_recommended_driver_plan() {
    let mut core = connected();
    assert_eq!(
        drivers(&core).status.as_deref(),
        Some("Checking the host's NVIDIA driver…")
    );

    open_drivers(&mut core);

    let section = drivers(&core);
    assert_eq!(section.read_only, None);
    assert_eq!(section.status, None);
    let facts: Vec<_> = section
        .host
        .iter()
        .map(|fact| (fact.label.as_str(), fact.value.as_str()))
        .collect();
    assert_eq!(
        facts,
        [
            ("Distribution", "ubuntu 24.04"),
            ("Kernel", "6.8.0-45-generic"),
            ("Package manager", "APT"),
            ("Secure Boot", "On"),
            ("NVIDIA driver", "Not loaded"),
            ("NVIDIA GPUs", "NVIDIA GeForce RTX 5090"),
        ]
    );
    assert_eq!(
        section.warnings,
        ["NVML could not be loaded: libnvidia-ml.so.1 was not found."]
    );
    let plan = section.plan.expect("the recommended plan");
    assert_eq!(plan.title, "Install recommended NVIDIA driver");
    assert_eq!(plan.blockers, Vec::<String>::new());
    assert_eq!(plan.warnings.len(), 2);
    assert_eq!(
        plan.manual_steps,
        ["Reboot after installation before expecting the new driver to load."]
    );
    assert_eq!(plan.run_unavailable, None);
    assert_eq!(section.job, None);
}

#[test]
fn drivers_reads_the_plan_again_each_time_it_opens() {
    let mut core = connected();
    open_drivers(&mut core);
    intent(&mut core, Intent::GoTo(Section::Games));

    let effects = intent(&mut core, Intent::GoTo(Section::Drivers));

    assert_eq!(commands(&effects), [DaemonRequest::GetDriverStatus]);
}

#[test]
fn without_prismd_drivers_says_why_and_reads_nothing() {
    let mut core = launched();
    core.handle(Input::PrismdNotConnected(SessionFailure::Unavailable {
        message: "No such file or directory (os error 2)".into(),
    }));

    let effects = intent(&mut core, Intent::GoTo(Section::Drivers));

    assert_eq!(commands(&effects), []);
    let section = drivers(&core);
    assert_eq!(
        section.status.as_deref(),
        Some("Driver management needs prismd · not running")
    );
    assert_eq!(section.read_only, None);
    assert_eq!(
        palette_entry(&mut core, "Run recommended driver plan…"),
        None
    );
}

#[test]
fn a_read_only_session_shows_the_plan_but_cant_run_it() {
    let mut core = connected_with(false);
    open_drivers(&mut core);

    let reason = Some("Needs tuning access · bob isn't in group wheel".to_owned());
    assert_eq!(drivers(&core).read_only, reason);
    assert_eq!(drivers(&core).plan.unwrap().run_unavailable, reason);
    assert_eq!(
        palette_entry(&mut core, "Run recommended driver plan…"),
        Some(reason)
    );
    assert_eq!(intent(&mut core, Intent::RunDriverPlan), []);
    assert_eq!(core.dialog(), None);
}

#[test]
fn a_blocked_plan_shows_its_blockers_and_cant_run() {
    let mut core = connected();
    let mut status = driver_status();
    for plan in &mut status.nvidia.plans {
        plan.blockers = vec!["An NVIDIA runfile installation is present.".into()];
    }
    open_drivers_with(&mut core, status);

    let plan = drivers(&core).plan.unwrap();
    assert_eq!(
        plan.blockers,
        ["An NVIDIA runfile installation is present."]
    );
    let blocked = Some("Blocked · An NVIDIA runfile installation is present.".to_owned());
    assert_eq!(plan.run_unavailable, blocked);
    assert_eq!(
        palette_entry(&mut core, "Run recommended driver plan…"),
        Some(blocked)
    );
}

#[test]
fn run_asks_first_and_then_starts_the_plans_driver_action() {
    let mut core = connected();
    open_drivers(&mut core);

    assert_eq!(commands(&intent(&mut core, Intent::RunDriverPlan)), []);

    let asked = confirmation(&core, run_confirmation);
    assert_eq!(asked.title, "Install recommended NVIDIA driver?");
    assert!(
        asked.message.contains("prismd runs APT as root"),
        "{asked:?}"
    );
    assert_eq!(
        asked.details.last().map(String::as_str),
        Some("Reboot after installation before expecting the new driver to load.")
    );
    assert_eq!(asked.details.len(), 3);
    assert_eq!(asked.confirm_text, "Run");
    assert_eq!(asked.confirm_unavailable, None);

    let (id, request) = command(&intent(&mut core, Intent::ConfirmDriverDialog));
    assert_eq!(
        request,
        DaemonRequest::StartDriverAction(StartDriverActionRequest {
            action: DriverAction::InstallRecommended
        })
    );
    assert_eq!(
        confirmation(&core, run_confirmation)
            .confirm_unavailable
            .as_deref(),
        Some("Starting…")
    );
    assert_eq!(
        commands(&intent(&mut core, Intent::ConfirmDriverDialog)),
        []
    );

    answer(
        &mut core,
        id,
        DaemonResponse::DriverJob(job(DriverJobStatus::Queued)),
    );
    assert_eq!(core.dialog(), None);
    // The job shows once prismd pushes it.
    assert_eq!(drivers(&core).job, None);
    push(&mut core, job(DriverJobStatus::Queued));
    assert_eq!(drivers(&core).job.unwrap().state, "Queued");
}

#[test]
fn dismissing_the_run_confirmation_starts_nothing() {
    let mut core = connected();
    open_drivers(&mut core);
    intent(&mut core, Intent::RunDriverPlan);

    assert_eq!(commands(&intent(&mut core, Intent::Dismiss)), []);

    assert_eq!(core.dialog(), None);
}

#[test]
fn the_palette_runs_the_plan_only_through_its_confirmation() {
    let mut core = connected();
    assert!(matches!(
        core.content(),
        Content::Searching | Content::Gpu(_)
    ));

    let effects = run_palette(&mut core, "Run recommended driver plan…");

    // It shows Drivers, reads the plan and waits for it before Run can be confirmed.
    assert_eq!(core.content(), Content::Section(Section::Drivers));
    let (id, request) = command(&effects);
    assert_eq!(request, DaemonRequest::GetDriverStatus);
    let waiting = confirmation(&core, run_confirmation);
    assert_eq!(
        waiting.confirm_unavailable.as_deref(),
        Some("Checking the driver plan…")
    );
    assert_eq!(
        commands(&intent(&mut core, Intent::ConfirmDriverDialog)),
        []
    );

    answer(&mut core, id, DaemonResponse::DriverStatus(driver_status()));
    let asked = confirmation(&core, run_confirmation);
    assert_eq!(asked.title, "Install recommended NVIDIA driver?");
    assert_eq!(asked.confirm_unavailable, None);
}

#[test]
fn a_refused_run_says_why_in_its_confirmation() {
    let mut core = connected();
    open_drivers(&mut core);
    intent(&mut core, Intent::RunDriverPlan);
    let (id, _) = command(&intent(&mut core, Intent::ConfirmDriverDialog));

    answer(
        &mut core,
        id,
        DaemonResponse::Error(GpuCommandError::new(
            GpuCommandErrorCode::InvalidArgument,
            "A driver install or update is already running",
        )),
    );

    let asked = confirmation(&core, run_confirmation);
    assert_eq!(
        asked.error.as_deref(),
        Some("A driver install or update is already running.")
    );
    assert_eq!(asked.confirm_unavailable, None);
}

#[test]
fn a_driver_jobs_progress_log_and_outcome_render_from_events() {
    let mut core = connected();
    open_drivers(&mut core);

    push(&mut core, job(DriverJobStatus::Running));
    let running = drivers(&core).job.unwrap();
    assert_eq!(running.title, "Install recommended NVIDIA driver");
    assert_eq!(running.state, "Running");
    assert!(running.active);
    assert_eq!(
        running.step.as_deref(),
        Some("Installing Ubuntu's recommended NVIDIA driver")
    );
    assert_eq!(
        running.log.last().unwrap(),
        "Running ubuntu-drivers install"
    );
    assert_eq!(running.cancel_unavailable, None);
    assert!(!running.reboot);
    assert_eq!(
        drivers(&core).plan.unwrap().run_unavailable.as_deref(),
        Some("A driver job is running")
    );

    let expected = [
        (
            DriverJobStatus::RebootRequired,
            "Reboot required",
            vec![
                "Package operations finished.",
                "Restart the host to load the NVIDIA driver.",
            ],
        ),
        (
            DriverJobStatus::ManualStepRequired,
            "Manual step required",
            vec![
                "Package operations finished, with a step left for you.",
                "Machine Owner Key enrollment may be required.",
                "Restart the host, and complete the step when the firmware asks for it. The \
                 NVIDIA driver loads after that.",
            ],
        ),
        (
            DriverJobStatus::Failed,
            "Failed",
            vec![
                "apt-get update exited with status exit status: 100.",
                "Check the log, then run the driver plan again.",
            ],
        ),
        (
            DriverJobStatus::Canceled,
            "Canceled",
            vec![
                "The driver job stopped before its next step. Steps that finished keep their \
                 changes.",
            ],
        ),
        (
            DriverJobStatus::Succeeded,
            "Done",
            vec!["Package operations finished, and nothing else is needed."],
        ),
    ];
    for (index, (status, state, explanation)) in expected.into_iter().enumerate() {
        // Each ending is a job of its own: a job that ended never changes again.
        let pushed = DriverJob {
            id: format!("driver-{index}"),
            ..job(status)
        };
        push(&mut core, pushed.clone());

        let shown = drivers(&core).job.unwrap();
        assert_eq!(shown.state, state);
        assert!(!shown.active);
        assert_eq!(shown.step, None, "{state}");
        assert_eq!(shown.explanation, explanation);
        assert_eq!(shown.log, pushed.log_tail);
        assert_eq!(
            shown.reboot,
            matches!(
                status,
                DriverJobStatus::RebootRequired | DriverJobStatus::ManualStepRequired
            )
        );
    }
}

#[test]
fn cancel_asks_prismd_and_the_job_ends_only_with_its_push() {
    let mut core = connected();
    open_drivers(&mut core);
    let running = job(DriverJobStatus::Running);
    push(&mut core, running.clone());

    let (id, request) = command(&intent(&mut core, Intent::CancelDriverJob));
    assert_eq!(
        request,
        DaemonRequest::CancelDriverJob(DriverJobRequest {
            job_id: running.id.clone()
        })
    );
    let canceling = Some("Canceling…".to_owned());
    assert_eq!(drivers(&core).job.unwrap().cancel_unavailable, canceling);
    answer(&mut core, id, DaemonResponse::DriverJob(running.clone()));
    assert_eq!(drivers(&core).job.unwrap().state, "Running");
    assert_eq!(drivers(&core).job.unwrap().cancel_unavailable, canceling);
    assert_eq!(commands(&intent(&mut core, Intent::CancelDriverJob)), []);

    let effects = push(&mut core, job(DriverJobStatus::Canceled));

    assert_eq!(drivers(&core).job.unwrap().state, "Canceled");
    // The user canceled it, so there's nothing to tell them.
    assert_eq!(notifications(&effects), []);
}

#[test]
fn a_refused_cancel_says_why_in_drivers() {
    let mut core = connected();
    open_drivers(&mut core);
    push(&mut core, job(DriverJobStatus::Running));
    let (id, _) = command(&intent(&mut core, Intent::CancelDriverJob));

    answer(
        &mut core,
        id,
        DaemonResponse::Error(GpuCommandError::new(
            GpuCommandErrorCode::NotFound,
            "Driver job driver-1791804900000 was not found",
        )),
    );

    assert_eq!(
        drivers(&core).outcome.as_deref(),
        Some("Driver job not canceled: Driver job driver-1791804900000 was not found.")
    );
    assert_eq!(drivers(&core).job.unwrap().cancel_unavailable, None);
}

#[test]
fn reboot_is_offered_only_when_the_host_needs_it_and_always_asks_first() {
    let mut core = connected();
    open_drivers(&mut core);
    push(&mut core, job(DriverJobStatus::Running));
    assert_eq!(palette_entry(&mut core, "Reboot now…"), None);
    assert_eq!(intent(&mut core, Intent::RebootHost), []);
    assert_eq!(core.dialog(), None);

    push(&mut core, job(DriverJobStatus::RebootRequired));
    assert_eq!(drivers(&core).job.unwrap().reboot_unavailable, None);
    assert_eq!(palette_entry(&mut core, "Reboot now…"), Some(None));
    assert_eq!(
        palette_entry(&mut core, "Run recommended driver plan…"),
        Some(Some("Restart the host first".into()))
    );

    assert_eq!(commands(&run_palette(&mut core, "Reboot now…")), []);
    let asked = confirmation(&core, reboot_confirmation);
    assert_eq!(asked.title, "Reboot now?");
    assert_eq!(asked.confirm_text, "Reboot");

    let (id, request) = command(&intent(&mut core, Intent::ConfirmDriverDialog));
    assert_eq!(request, DaemonRequest::RequestHostReboot);
    assert_eq!(
        confirmation(&core, reboot_confirmation)
            .confirm_unavailable
            .as_deref(),
        Some("Restarting…")
    );
    answer(
        &mut core,
        id,
        DaemonResponse::Command(DaemonCommandResponse {
            success: true,
            error: None,
        }),
    );
    assert_eq!(core.dialog(), None);
}

#[test]
fn a_refused_reboot_says_why_in_its_confirmation() {
    let mut core = connected();
    push(&mut core, job(DriverJobStatus::ManualStepRequired));
    intent(&mut core, Intent::RebootHost);
    let (id, _) = command(&intent(&mut core, Intent::ConfirmDriverDialog));

    answer(
        &mut core,
        id,
        DaemonResponse::Command(DaemonCommandResponse {
            success: false,
            error: Some("systemctl reboot exited with status exit status: 1".into()),
        }),
    );

    assert_eq!(
        confirmation(&core, reboot_confirmation).error.as_deref(),
        Some("systemctl reboot exited with status exit status: 1.")
    );
}

#[test]
fn a_read_only_session_cant_cancel_or_reboot() {
    let mut core = connected_with(false);
    push(&mut core, job(DriverJobStatus::Running));
    let reason = Some("Needs tuning access · bob isn't in group wheel".to_owned());
    assert_eq!(drivers(&core).job.unwrap().cancel_unavailable, reason);
    assert_eq!(intent(&mut core, Intent::CancelDriverJob), []);

    push(&mut core, job(DriverJobStatus::RebootRequired));
    assert_eq!(drivers(&core).job.unwrap().reboot_unavailable, reason);
    assert_eq!(palette_entry(&mut core, "Reboot now…"), Some(reason));
    assert_eq!(intent(&mut core, Intent::RebootHost), []);
    assert_eq!(core.dialog(), None);
}

#[test]
fn a_driver_job_ending_notifies_once_while_the_window_lacks_focus() {
    let mut core = connected();
    push(&mut core, job(DriverJobStatus::Queued));
    assert_eq!(
        notifications(&push(&mut core, job(DriverJobStatus::Running))),
        []
    );

    let effects = push(&mut core, job(DriverJobStatus::RebootRequired));

    let sent = notifications(&effects);
    assert_eq!(sent.len(), 1, "{effects:?}");
    assert_eq!(sent[0].summary, "NVIDIA driver job done · Reboot required");
    assert_eq!(
        sent[0].body,
        "Install recommended NVIDIA driver. Package operations finished. Restart the host to \
         load the NVIDIA driver."
    );
    // The same push again, such as after a snapshot, says nothing new.
    assert_eq!(
        notifications(&push(&mut core, job(DriverJobStatus::RebootRequired))),
        []
    );

    // Clicking it leads to Drivers.
    let effects = core.handle(Input::NotificationClicked {
        id: sent[0].id,
        activation_token: Some("token".into()),
    });
    assert_eq!(core.content(), Content::Section(Section::Drivers));
    assert!(effects.contains(&Effect::RaiseWindow {
        activation_token: Some("token".into())
    }));
}

#[test]
fn every_ending_but_a_cancel_notifies() {
    let expected = [
        (DriverJobStatus::Succeeded, Some("NVIDIA driver job done")),
        (
            DriverJobStatus::ManualStepRequired,
            Some("NVIDIA driver job done · Manual step required"),
        ),
        (DriverJobStatus::Failed, Some("NVIDIA driver job failed")),
        (DriverJobStatus::Canceled, None),
    ];
    for (status, summary) in expected {
        let mut core = connected();
        push(&mut core, job(DriverJobStatus::Running));

        let sent = notifications(&push(&mut core, job(status)));

        assert_eq!(
            sent.iter()
                .map(|notification| notification.summary.as_str())
                .collect::<Vec<_>>(),
            summary.into_iter().collect::<Vec<_>>(),
            "{status:?}"
        );
    }
}

#[test]
fn a_driver_job_ending_doesnt_notify_while_the_window_has_focus() {
    let mut core = connected();
    core.handle(Input::WindowFocusChanged(true));
    push(&mut core, job(DriverJobStatus::Running));

    assert_eq!(
        notifications(&push(&mut core, job(DriverJobStatus::Failed))),
        []
    );
}

#[test]
fn a_session_starts_from_its_latest_driver_job_without_notifying() {
    let (mut core, effects) = connected_from(
        DaemonSnapshot {
            driver_job: Some(job(DriverJobStatus::RebootRequired)),
            ..snapshot()
        },
        true,
    );

    assert_eq!(notifications(&effects), []);
    assert_eq!(drivers(&core).job.unwrap().state, "Reboot required");
    assert_eq!(
        notifications(&push(&mut core, job(DriverJobStatus::RebootRequired))),
        []
    );
}

#[test]
fn a_driver_job_ending_reads_the_plan_again_while_drivers_shows_it() {
    let mut core = connected();
    open_drivers(&mut core);
    push(&mut core, job(DriverJobStatus::Running));

    let effects = push(&mut core, job(DriverJobStatus::Failed));

    assert_eq!(commands(&effects), [DaemonRequest::GetDriverStatus]);
    // The plan read before stays shown meanwhile.
    let plan = drivers(&core).plan.expect("the plan read before");
    assert_eq!(plan.title, "Install recommended NVIDIA driver");
    assert_eq!(
        plan.run_unavailable.as_deref(),
        Some("Checking the driver plan…")
    );
}

#[test]
fn a_reboot_or_manual_step_left_outstanding_is_a_problem_in_activity() {
    let mut core = connected();
    push(&mut core, job(DriverJobStatus::Running));
    assert_eq!(core.status_bar().problems, 0);

    push(&mut core, job(DriverJobStatus::ManualStepRequired));

    assert_eq!(core.status_bar().problems, 1);
    intent(&mut core, Intent::OpenActivity);
    let activity = core.activity().expect("Activity is open");
    let problem = &activity.problems[0];
    assert_eq!(problem.title, "Manual step required");
    assert_eq!(problem.detail[0], "Install recommended NVIDIA driver");
    assert_eq!(
        problem.detail[2],
        "Machine Owner Key enrollment may be required."
    );
    let button = problem.action.as_ref().unwrap();
    assert_eq!(button.title, "Open Drivers");
    assert_eq!(button.action, ActivityAction::OpenDrivers);

    let effects = intent(
        &mut core,
        Intent::RunActivityAction(ActivityAction::OpenDrivers),
    );

    assert_eq!(core.activity(), None);
    assert_eq!(core.content(), Content::Section(Section::Drivers));
    assert_eq!(commands(&effects), [DaemonRequest::GetDriverStatus]);
}

#[test]
fn a_lost_session_closes_the_confirmation_and_forgets_the_job() {
    let mut core = connected();
    open_drivers(&mut core);
    push(&mut core, job(DriverJobStatus::RebootRequired));
    intent(&mut core, Intent::RebootHost);
    command(&intent(&mut core, Intent::ConfirmDriverDialog));

    core.handle(Input::PrismdNotConnected(SessionFailure::Unavailable {
        message: "Connection reset by peer (os error 104)".into(),
    }));

    assert_eq!(core.dialog(), None);
    let section = drivers(&core);
    assert_eq!(section.job, None);
    assert_eq!(section.plan, None);
    assert_eq!(
        section.outcome.as_deref(),
        Some(
            "The prismd connection ended before restarting the host was answered. Check Drivers \
             once Prism reconnects."
        )
    );
    assert_eq!(core.status_bar().problems, 0);
}

#[test]
fn reconnecting_while_drivers_shows_reads_the_plan_again() {
    let mut core = connected();
    open_drivers(&mut core);
    core.handle(Input::PrismdNotConnected(SessionFailure::Unavailable {
        message: "Connection reset by peer (os error 104)".into(),
    }));

    let effects = core.handle(Input::PrismdConnected {
        welcome: welcome(true),
        snapshot: snapshot(),
        socket: "/run/prism/prismd.sock".into(),
    });

    assert_eq!(commands(&effects), [DaemonRequest::GetDriverStatus]);
}

#[test]
fn a_driver_job_that_ended_while_the_session_was_down_notifies_once_it_comes_back() {
    let mut core = connected();
    push(&mut core, job(DriverJobStatus::Running));
    core.handle(Input::PrismdNotConnected(SessionFailure::Unavailable {
        message: "Connection reset by peer (os error 104)".into(),
    }));

    let effects = core.handle(Input::PrismdConnected {
        welcome: welcome(true),
        snapshot: DaemonSnapshot {
            driver_job: Some(job(DriverJobStatus::Failed)),
            ..snapshot()
        },
        socket: "/run/prism/prismd.sock".into(),
    });

    let sent = notifications(&effects);
    assert_eq!(
        sent.iter()
            .map(|notification| notification.summary.as_str())
            .collect::<Vec<_>>(),
        ["NVIDIA driver job failed"]
    );
    assert_eq!(
        notifications(&push(&mut core, job(DriverJobStatus::Failed))),
        []
    );
}

#[test]
fn a_push_older_than_the_snapshot_doesnt_bring_an_ended_job_back() {
    let (mut core, _) = connected_from(
        DaemonSnapshot {
            driver_job: Some(job(DriverJobStatus::RebootRequired)),
            ..snapshot()
        },
        true,
    );

    push(&mut core, job(DriverJobStatus::Running));

    assert_eq!(drivers(&core).job.unwrap().state, "Reboot required");
    assert_eq!(
        notifications(&push(&mut core, job(DriverJobStatus::RebootRequired))),
        []
    );
}

#[test]
fn a_newer_driver_job_withdraws_the_last_ones_notification() {
    let mut core = connected();
    push(&mut core, job(DriverJobStatus::Running));
    let [ended] = notifications(&push(&mut core, job(DriverJobStatus::Failed)))
        .try_into()
        .expect("one notification");

    let effects = push(
        &mut core,
        DriverJob {
            id: "driver-1791805200000".into(),
            ..job(DriverJobStatus::Queued)
        },
    );

    assert!(
        effects.contains(&Effect::WithdrawNotification(ended.id)),
        "{effects:?}"
    );
}

#[test]
fn the_palette_reads_a_plan_that_couldnt_be_read_again() {
    let mut core = connected();
    let (id, _) = command(&intent(&mut core, Intent::GoTo(Section::Drivers)));
    answer(
        &mut core,
        id,
        DaemonResponse::Error(GpuCommandError::new(
            GpuCommandErrorCode::Io,
            "nvidia-smi timed out",
        )),
    );

    let effects = run_palette(&mut core, "Run recommended driver plan…");

    assert_eq!(commands(&effects), [DaemonRequest::GetDriverStatus]);
    assert_eq!(
        confirmation(&core, run_confirmation)
            .confirm_unavailable
            .as_deref(),
        Some("Checking the driver plan…")
    );
}
