//! Activity: the one place to see what happened and what's wrong. It lists current problems
//! first, then this boot's startup outcomes not yet dismissed, then recent events: prismd's
//! boot-scoped Activity, which includes what it did while Prism was closed, and Prism's own
//! entries, such as a short prismd connection loss. The status bar's problem indicator counts the
//! problems and opens it (ADR 0010).
//!
//! Each source of current problems, such as a failure pause, a cooling fault or a driver job's
//! outcome, adds its problems in [`Core::problems`], with an [`ActivityAction`] for its button.

use std::collections::BTreeMap;
use std::time::Duration;

use prism_gpu::{TuningOutcome, TuningOutcomeStatus};
use prism_protocol::{
    ActivityEntry, ActivityEvent, DaemonRequest, DaemonResponse, DaemonSnapshot,
    ProfileApplicationResponse, ProfileSettingKey, StartupGpuRequest, StartupOutcome,
    StartupOutcomeStatus, StartupState, TuningStoreHealth, TuningStoreProblem, trim_activity,
};

use crate::core::{CommandId, Core, Effect};
use crate::drafts::{not_applied, setting_name};
use crate::palette::Command;
use crate::view::{ActivityButton, ActivityItem, ActivityPanel};

/// How many of Prism's own entries Activity keeps.
const OWN_LIMIT: usize = 50;

/// Something an Activity item's button does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivityAction {
    /// Show this GPU, by id, in the workspace.
    OpenGpu(String),
    /// Dismiss this GPU's startup outcome, by GPU id, for the caller.
    DismissStartupOutcome(String),
}

/// What Activity knows, apart from what other parts of the core report as problems.
#[derive(Debug, Default)]
pub(crate) struct ActivityState {
    pub(crate) open: bool,
    /// The wall clock when Prism started, in Unix milliseconds.
    started_unix_ms: u64,
    /// The latest time, since Prism started, an input arrived at.
    now: Duration,
    /// prismd's entries the caller may see, by id, as prismd last told them.
    daemon: Vec<ActivityEntry>,
    /// Prism's own entries, oldest first.
    own: Vec<OwnEntry>,
    /// Whether prismd can use its tuning store, while there is a session.
    store_health: Option<TuningStoreHealth>,
    /// The startup choices and the caller's startup outcomes, while there is a session.
    startup: Option<StartupState>,
    /// When the session was lost, while it hasn't come back.
    lost_at: Option<Duration>,
    /// Each dismissal sent to prismd that it hasn't answered, with its GPU's id.
    dismissing: BTreeMap<CommandId, String>,
    /// Why prismd refused the latest action, until the next one.
    outcome: Option<String>,
}

/// One of Prism's own entries.
#[derive(Debug, Clone)]
struct OwnEntry {
    at_unix_ms: u64,
    title: String,
    detail: Vec<String>,
}

impl ActivityState {
    /// Prism started at this wall clock time.
    pub(crate) fn started(&mut self, started_unix_ms: u64) {
        self.started_unix_ms = started_unix_ms;
    }

    /// An input arrived this long after Prism started.
    pub(crate) fn arrived(&mut self, at: Duration) {
        self.now = self.now.max(at);
    }

    fn now_unix_ms(&self) -> u64 {
        self.started_unix_ms
            .saturating_add(self.now.as_millis().try_into().unwrap_or(u64::MAX))
    }
}

impl Core {
    /// Takes in what a new session's snapshot says Activity shows; a lost session that came back
    /// gets its own entry.
    pub(crate) fn activity_connected(&mut self, snapshot: &DaemonSnapshot) {
        let activity = &mut self.activity;
        activity.store_health = Some(snapshot.store_health.clone());
        activity.startup = Some(snapshot.startup.clone());
        activity.daemon = snapshot.activity.clone();
        if let Some(lost_at) = activity.lost_at.take() {
            let lost_for = activity.now.saturating_sub(lost_at);
            self.record_own(
                format!("Lost the prismd connection for {}", duration(lost_for)),
                vec!["Tuning was read-only until Prism reconnected.".into()],
            );
        }
    }

    /// There's no session: what prismd said about its problems no longer holds, and actions it
    /// didn't answer may or may not have happened. Its events stay what happened.
    pub(crate) fn activity_lost_session(&mut self, lost: bool) {
        let activity = &mut self.activity;
        activity.store_health = None;
        activity.startup = None;
        activity.dismissing.clear();
        if lost {
            activity.lost_at = Some(activity.now);
        }
    }

    pub(crate) fn store_health_pushed(&mut self, health: TuningStoreHealth) {
        self.activity.store_health = Some(health);
    }

    pub(crate) fn startup_pushed(&mut self, startup: StartupState) {
        self.activity.startup = Some(startup);
    }

    /// prismd recorded an entry; one already known, such as from the snapshot, shows once.
    pub(crate) fn activity_recorded(&mut self, entry: ActivityEntry) {
        let daemon = &mut self.activity.daemon;
        if let Err(index) = daemon.binary_search_by_key(&entry.id, |known| known.id) {
            daemon.insert(index, entry);
        }
        // Kept as prismd keeps them.
        trim_activity(daemon, |entry| entry);
    }

    fn record_own(&mut self, title: String, detail: Vec<String>) {
        let at_unix_ms = self.activity.now_unix_ms();
        let own = &mut self.activity.own;
        own.push(OwnEntry {
            at_unix_ms,
            title,
            detail,
        });
        let excess = own.len().saturating_sub(OWN_LIMIT);
        own.drain(..excess);
    }

    pub fn activity_open(&self) -> bool {
        self.activity.open
    }

    pub(crate) fn open_activity(&mut self) -> Vec<Effect> {
        self.activity.open = true;
        Vec::new()
    }

    pub(crate) fn close_activity(&mut self) {
        self.activity.open = false;
    }

    /// The open Activity panel, if it's open.
    pub fn activity(&self) -> Option<ActivityPanel> {
        if !self.activity.open {
            return None;
        }
        let (problems, startup) = self.startup_items();
        let problems: Vec<_> = self.store_problem().into_iter().chain(problems).collect();
        let recent = self.recent_items();
        let nothing = problems.is_empty() && startup.is_empty() && recent.is_empty();
        Some(ActivityPanel {
            problems,
            startup,
            recent,
            outcome: self.activity.outcome.clone(),
            empty: nothing.then(|| "Nothing has happened yet this boot.".into()),
        })
    }

    /// The current problems, as Activity lists them and the problem indicator counts them.
    pub(crate) fn problems(&self) -> Vec<ActivityItem> {
        self.store_problem()
            .into_iter()
            .chain(self.startup_items().0)
            .collect()
    }

    pub(crate) fn run_activity_action(&mut self, action: ActivityAction) -> Vec<Effect> {
        match action {
            ActivityAction::OpenGpu(gpu_id) => {
                self.close_activity();
                self.perform(Command::GoToGpu(gpu_id))
            }
            ActivityAction::DismissStartupOutcome(gpu_id) => {
                if self
                    .action_unavailable(&ActivityAction::DismissStartupOutcome(gpu_id.clone()))
                    .is_some()
                {
                    return Vec::new();
                }
                let id = self.drafts.next_command();
                self.activity.outcome = None;
                self.activity.dismissing.insert(id, gpu_id.clone());
                vec![Effect::CommandPrismd {
                    id,
                    request: DaemonRequest::DismissStartupOutcome(StartupGpuRequest { gpu_id }),
                }]
            }
        }
    }

    /// Takes in prismd's answer to a dismissal, if `command` was one; the outcome goes only once
    /// prismd pushes the startup state without it. Says whether it was one.
    pub(crate) fn activity_answered(
        &mut self,
        command: CommandId,
        response: &DaemonResponse,
    ) -> bool {
        if self.activity.dismissing.remove(&command).is_none() {
            return false;
        }
        if let DaemonResponse::Error(error) = response {
            self.activity.outcome = Some(not_applied("Not dismissed", Some(&error.message)));
        }
        true
    }

    fn action_unavailable(&self, action: &ActivityAction) -> Option<String> {
        match action {
            ActivityAction::OpenGpu(_) => None,
            ActivityAction::DismissStartupOutcome(gpu_id) => {
                if let Some(reason) = self.tuning_access().reason() {
                    return Some(format!("Needs tuning access · {reason}"));
                }
                self.activity
                    .dismissing
                    .values()
                    .any(|dismissing| dismissing == gpu_id)
                    .then(|| "Dismissing…".into())
            }
        }
    }

    fn button(&self, title: &str, action: ActivityAction) -> ActivityButton {
        ActivityButton {
            title: title.into(),
            unavailable: self.action_unavailable(&action),
            action,
        }
    }

    /// An unhealthy tuning store, explained.
    fn store_problem(&self) -> Option<ActivityItem> {
        let Some(TuningStoreHealth::Unhealthy {
            problem,
            path,
            detail,
        }) = &self.activity.store_health
        else {
            return None;
        };
        let why = match problem {
            TuningStoreProblem::Unreadable => "It can't be read",
            TuningStoreProblem::NewerVersion => "It was written by a newer version of Prism",
        };
        Some(ActivityItem {
            title: "Prism's tuning store can't be used".into(),
            detail: vec![
                sentence(&format!("{why}: {detail}")),
                "Startup behavior and automatic switching are suspended. Profile applications \
                 and live tuning changes keep working."
                    .into(),
                format!("Prism leaves {path} untouched."),
            ],
            when: None,
            action: None,
        })
    }

    /// The caller's startup outcomes not yet dismissed: those that didn't apply in full, which
    /// are problems, and those that did. Only an outcome prismd found worth recording in Activity
    /// is shown, so a GPU startup had nothing to do with isn't.
    fn startup_items(&self) -> (Vec<ActivityItem>, Vec<ActivityItem>) {
        let outcomes = self
            .activity
            .startup
            .iter()
            .flat_map(|startup| &startup.outcomes)
            .filter(|outcome| {
                self.activity.daemon.iter().any(|entry| {
                    entry.gpu_id == outcome.gpu_id
                        && matches!(entry.event, ActivityEvent::StartupRan { .. })
                })
            });
        let (applied, problems): (Vec<_>, Vec<_>) =
            outcomes.partition(|outcome| outcome.status == StartupOutcomeStatus::Applied);
        let item = |outcome: &StartupOutcome| {
            let (title, lines) = startup_lines(outcome);
            ActivityItem {
                title,
                detail: [self.gpu_name(&outcome.gpu_id)]
                    .into_iter()
                    .chain(lines)
                    .collect(),
                when: None,
                action: Some(self.button(
                    "Dismiss",
                    ActivityAction::DismissStartupOutcome(outcome.gpu_id.clone()),
                )),
            }
        };
        (
            problems.into_iter().map(item).collect(),
            applied.into_iter().map(item).collect(),
        )
    }

    /// prismd's events and Prism's own, newest first.
    fn recent_items(&self) -> Vec<ActivityItem> {
        let now = self.activity.now_unix_ms();
        let daemon = self.activity.daemon.iter().map(|entry| {
            let (title, lines) = match &entry.event {
                ActivityEvent::ProfileApplied {
                    profile,
                    application,
                } => {
                    let name = profile.as_ref().map_or("Tuning changes", |p| &p.name);
                    outcome_lines(name, "applied", application)
                }
                ActivityEvent::GameProfileApplied {
                    game_name,
                    profile,
                    application,
                    ..
                } => outcome_lines(
                    &format!("{game_name} started · {}", profile.name),
                    "applied",
                    application,
                ),
                ActivityEvent::PreGameValuesRestored { application } => {
                    outcome_lines("Pre-game values", "restored", application)
                }
                ActivityEvent::StartupRan { outcome } => startup_lines(outcome),
            };
            let known = self.gpus().iter().any(|gpu| gpu.id == entry.gpu_id);
            (
                entry.at_unix_ms,
                ActivityItem {
                    title,
                    detail: [self.gpu_name(&entry.gpu_id)]
                        .into_iter()
                        .chain(lines)
                        .collect(),
                    when: Some(ago(now, entry.at_unix_ms)),
                    action: known.then(|| {
                        self.button("Open", ActivityAction::OpenGpu(entry.gpu_id.clone()))
                    }),
                },
            )
        });
        let own = self.activity.own.iter().map(|entry| {
            (
                entry.at_unix_ms,
                ActivityItem {
                    title: entry.title.clone(),
                    detail: entry.detail.clone(),
                    when: Some(ago(now, entry.at_unix_ms)),
                    action: None,
                },
            )
        });
        let mut items: Vec<_> = daemon.chain(own).collect();
        // Stable, so entries at the same time stay in the order they happened.
        items.sort_by_key(|(at, _)| *at);
        items.into_iter().rev().map(|(_, item)| item).collect()
    }
}

/// What applying, or restoring, `name` put into effect, such as "Quiet applied in part", and each
/// setting's outcome, such as "Power limit applied".
fn outcome_lines(
    name: &str,
    verb: &str,
    application: &ProfileApplicationResponse,
) -> (String, Vec<String>) {
    let outcomes = [
        (
            ProfileSettingKey::PowerLimitWatts,
            application.power.as_ref(),
        ),
        (
            ProfileSettingKey::GraphicsOffsetMhz,
            application.graphics.as_ref(),
        ),
        (
            ProfileSettingKey::MemoryOffsetMhz,
            application.memory.as_ref(),
        ),
    ]
    .into_iter()
    .filter_map(|(key, outcome)| Some((setting_name(key), outcome?)))
    .chain(
        application
            .cooling
            .as_ref()
            .map(|cooling| ("Cooling", &cooling.outcome)),
    );
    let mut succeeded = 0;
    let mut lines = Vec::new();
    for (setting, outcome) in outcomes {
        lines.push(setting_outcome(setting, verb, outcome));
        if outcome.status == TuningOutcomeStatus::Succeeded {
            succeeded += 1;
        }
    }
    let title = if succeeded == lines.len() {
        format!("{name} {verb}")
    } else if succeeded > 0 {
        format!("{name} {verb} in part")
    } else {
        format!("{name} not {verb}")
    };
    (title, lines)
}

fn setting_outcome(setting: &str, verb: &str, outcome: &TuningOutcome) -> String {
    if outcome.status == TuningOutcomeStatus::Succeeded {
        format!("{setting} {verb}")
    } else {
        not_applied(&format!("{setting} not {verb}"), outcome.error.as_deref())
    }
}

/// What startup put into effect on a GPU, such as "Startup applied Quiet in part", and each
/// setting's outcome, or why nothing was applied.
fn startup_lines(outcome: &StartupOutcome) -> (String, Vec<String>) {
    let name = outcome
        .profile
        .as_ref()
        .map_or("last applied tuning values", |profile| &profile.name);
    let mut lines = Vec::new();
    let any_applied = match &outcome.application {
        Some(application) => {
            let (_, settings) = outcome_lines(name, "applied", application);
            lines.extend(settings);
            application
                .power
                .iter()
                .chain(&application.graphics)
                .chain(&application.memory)
                .chain(application.cooling.as_ref().map(|cooling| &cooling.outcome))
                .any(|setting| setting.status == TuningOutcomeStatus::Succeeded)
        }
        None => {
            lines.extend(outcome.reason.clone());
            false
        }
    };
    lines.extend(outcome.previous_failure.clone());
    let title = match outcome.status {
        StartupOutcomeStatus::Applied => format!("Startup applied {name}"),
        StartupOutcomeStatus::Failed if any_applied => format!("Startup applied {name} in part"),
        _ if outcome.behavior.is_off() => "Startup behavior is off".into(),
        StartupOutcomeStatus::Failed | StartupOutcomeStatus::NotAttempted => {
            format!("Startup didn't apply {name}")
        }
    };
    (title, lines)
}

/// How long ago `at` was at `now`, both in Unix milliseconds, such as "5 min ago".
fn ago(now: u64, at: u64) -> String {
    let seconds = now.saturating_sub(at) / 1000;
    match seconds {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", seconds / 60),
        3600..86_400 => format!("{} h ago", seconds / 3600),
        _ => format!("{} d ago", seconds / 86_400),
    }
}

/// A length of time, such as "5 s" or "2 min".
fn duration(length: Duration) -> String {
    let seconds = length.as_secs();
    match seconds {
        0..60 => format!("{seconds} s"),
        60..3600 => format!("{} min", seconds / 60),
        _ => format!("{} h", seconds / 3600),
    }
}

/// `text` ending in a period.
fn sentence(text: &str) -> String {
    if text.ends_with('.') {
        text.into()
    } else {
        format!("{text}.")
    }
}
