//! Automatic switching status and control (ADRs 0003 and 0004). prismd switches profiles as games
//! run and pushes each GPU's automatic switching status; Prism explains it in the status bar for
//! the selected GPU and in each GPU's workspace, with Pause and Resume, and Retry on a failure
//! pause. A failure pause also shows as PAUSED in the sidebar, as a problem in Activity and in a
//! desktop notification. What's shown changes only with prismd's pushes; answers only say why a
//! request was refused.

use std::collections::BTreeMap;

use prism_protocol::{
    AutomaticSwitchingStatus, DaemonRequest, DaemonResponse, GpuSwitchingState, RunningGamesState,
    SetGpuSwitchingPausedRequest, StartupGpuRequest, SwitchingCondition,
};

use crate::activity::{ActivityAction, sentence};
use crate::core::{CommandId, Core, Effect, Selection};
use crate::drafts::not_applied;
use crate::notifications::{GpuConcern, Subject};
use crate::palette::Command;
use crate::view::{ActivityItem, SwitchingStatus, gpu_label};

/// What prismd last said about automatic switching, and the requests it hasn't answered.
#[derive(Debug, Default)]
pub(crate) struct Switching {
    /// The caller's automatic switching status as prismd last pushed it, while there is a
    /// session.
    status: Option<AutomaticSwitchingStatus>,
    /// Each Pause, Resume or Retry prismd hasn't answered, with its GPU's id.
    sent: BTreeMap<CommandId, (String, SwitchingRequest)>,
    /// Why prismd refused the latest request on each GPU, by GPU id, until the next one or the
    /// GPU's status changes.
    outcomes: BTreeMap<String, String>,
}

/// A request about a GPU's automatic switching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SwitchingRequest {
    Pause,
    Resume,
    Retry,
}

impl SwitchingRequest {
    /// Pause, or with `paused` false, Resume.
    fn setting_paused(paused: bool) -> Self {
        if paused { Self::Pause } else { Self::Resume }
    }

    /// What the button says while prismd hasn't answered it.
    fn waiting(self) -> &'static str {
        match self {
            Self::Pause => "Pausing…",
            Self::Resume => "Resuming…",
            Self::Retry => "Retrying…",
        }
    }

    /// What prismd's refusal says.
    fn refused(self) -> &'static str {
        match self {
            Self::Pause => "Not paused",
            Self::Resume => "Not resumed",
            Self::Retry => "Not retried",
        }
    }
}

/// What brings a GPU out of a failure pause.
const UNTIL_RETRY: &str = "Automatic switching won't change this GPU again until you retry.";

/// The GPU's state in `status`, if it lists the GPU.
fn state_in<'a>(
    status: Option<&'a AutomaticSwitchingStatus>,
    gpu_id: &str,
) -> Option<&'a GpuSwitchingState> {
    status?
        .gpus
        .iter()
        .find(|gpu| gpu.gpu_id == gpu_id)
        .map(|gpu| &gpu.state)
}

/// Whether `state` is a failure pause, which only Retry ends.
fn in_failure_pause(state: Option<&GpuSwitchingState>) -> bool {
    matches!(
        state,
        Some(GpuSwitchingState::Failed { .. } | GpuSwitchingState::RecoveryRequired { .. })
    )
}

/// Each line of prismd's message, as a sentence.
fn sentences(message: &str) -> impl Iterator<Item = String> {
    message
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(sentence)
}

/// Names joined into a list, such as "Dota 2 and Cyberpunk 2077".
fn listed(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [name] => (*name).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

impl Core {
    /// The GPU's state as prismd last pushed it, if prismd lists the GPU.
    fn switching_state(&self, gpu_id: &str) -> Option<&GpuSwitchingState> {
        state_in(self.switching.status.as_ref(), gpu_id)
    }

    /// Whether automatic switching is in a failure pause on the GPU.
    pub(crate) fn switching_failed(&self, gpu_id: &str) -> bool {
        in_failure_pause(self.switching_state(gpu_id))
    }

    /// Whether the GPU has an explicit automatic switching pause.
    fn switching_paused(&self, gpu_id: &str) -> bool {
        matches!(
            self.switching_state(gpu_id),
            Some(GpuSwitchingState::Paused {})
        )
    }

    /// Whether automatic switching is paused on the GPU, explicitly or by a manual override, so
    /// Resume brings it back.
    fn switching_resumable(&self, gpu_id: &str) -> bool {
        matches!(
            self.switching_state(gpu_id),
            Some(GpuSwitchingState::Paused {} | GpuSwitchingState::ManualOverride { .. })
        )
    }

    /// What the sidebar says about the GPU's failure pause, such as "Automatic switching failed".
    pub(crate) fn switching_failure(&self, gpu_id: &str) -> Option<&'static str> {
        match self.switching_state(gpu_id)? {
            GpuSwitchingState::Failed { .. } => Some("Automatic switching failed"),
            GpuSwitchingState::RecoveryRequired { .. } => {
                Some("Automatic switching needs recovery")
            }
            _ => None,
        }
    }

    /// The GPU's automatic switching status as its workspace explains it, while there is a
    /// session.
    pub(crate) fn switching_status(&self, gpu_id: &str) -> Option<SwitchingStatus> {
        let status = self.switching.status.as_ref()?;
        let (summary, detail) = match self.switching_state(gpu_id) {
            Some(state) => self.explained(gpu_id, state),
            None => explained_condition(&status.condition),
        };
        let resumable = self.switching_resumable(gpu_id);
        let pause = SwitchingRequest::setting_paused(!resumable);
        Some(SwitchingStatus {
            summary,
            detail,
            failed: self.switching_failed(gpu_id),
            resumable,
            pause_unavailable: self.switching_unavailable(gpu_id, pause),
            retry_unavailable: self.switching_unavailable(gpu_id, SwitchingRequest::Retry),
            outcome: self.switching.outcomes.get(gpu_id).cloned(),
        })
    }

    /// What drives automatic switching on the GPU, or why it doesn't, in short and explained.
    fn explained(&self, gpu_id: &str, state: &GpuSwitchingState) -> (String, Vec<String>) {
        match state {
            GpuSwitchingState::Active {
                game_name, profile, ..
            } => (
                format!("{game_name} → {}", profile.name),
                vec![
                    format!(
                        "{game_name} is running, so automatic switching applied {}.",
                        profile.name
                    ),
                    "The pre-game tuning values come back when it ends.".into(),
                ],
            ),
            GpuSwitchingState::WaitingForEdits { profile } => (
                format!("Waiting for edits → {}", profile.name),
                vec![
                    format!("A game linked to {} is running.", profile.name),
                    "Automatic switching applies it once this GPU's unapplied tuning changes are \
                     applied or discarded."
                        .into(),
                ],
            ),
            GpuSwitchingState::Paused {} => (
                "Paused".into(),
                vec![
                    "Automatic switching is paused on this GPU, keeping its current tuning \
                     values, until you resume it."
                        .into(),
                ],
            ),
            GpuSwitchingState::AnotherUser {} => (
                "Another user's game".into(),
                vec!["Another user's running game drives this GPU.".into()],
            ),
            GpuSwitchingState::Restored {} => (
                "Pre-game values restored".into(),
                vec![
                    "The game ended, and automatic switching restored this GPU's pre-game \
                     tuning values."
                        .into(),
                ],
            ),
            GpuSwitchingState::ManualOverride {
                changed_outside_prism,
            } => {
                let games = self.running_linked_games(gpu_id);
                let who = if *changed_outside_prism {
                    "A change made outside Prism"
                } else {
                    "Your own tuning change"
                };
                let (summary, why) = match games.as_slice() {
                    // prismd keeps a change made outside Prism after the games end, until a
                    // linked game starts again.
                    [] if *changed_outside_prism => (
                        "Overridden".to_owned(),
                        format!(
                            "{who} replaced the game's tuning, so automatic switching waits for \
                             the next linked game."
                        ),
                    ),
                    [] => (
                        "Overridden".to_owned(),
                        format!(
                            "{who} paused automatic switching on this GPU until its linked \
                             games end."
                        ),
                    ),
                    games => {
                        let ends = if games.len() == 1 { "ends" } else { "end" };
                        let games = listed(games);
                        (
                            format!("Overridden until {games} {ends}"),
                            format!(
                                "{who} paused automatic switching on this GPU until {games} \
                                 {ends}."
                            ),
                        )
                    }
                };
                (summary, vec![why, "Resume to switch again now.".into()])
            }
            GpuSwitchingState::Failed { message } => (
                "Failed".into(),
                sentences(message).chain([UNTIL_RETRY.into()]).collect(),
            ),
            GpuSwitchingState::RecoveryRequired { message } => (
                "Needs recovery".into(),
                sentences(message)
                    .chain([match self.switching_blocked() {
                        // prismd can't retry until what blocks it is fixed.
                        Some(reason) => sentence(reason),
                        None => UNTIL_RETRY.into(),
                    }])
                    .collect(),
            ),
        }
    }

    /// Why automatic switching is blocked on every GPU, when it is.
    fn switching_blocked(&self) -> Option<&str> {
        match &self.switching.status.as_ref()?.condition {
            SwitchingCondition::Blocked { reason } => Some(reason),
            _ => None,
        }
    }

    /// The names of the caller's running games linked to the GPU, earliest started first.
    fn running_linked_games(&self, gpu_id: &str) -> Vec<&str> {
        let Some(RunningGamesState::Available { games: running }) = &self.running_games else {
            return Vec::new();
        };
        let mut running: Vec<_> = running.iter().collect();
        running.sort_by_key(|game| game.start_time_ticks);
        let mut names = Vec::new();
        for game in running {
            let linked = self
                .games
                .games
                .iter()
                .find(|entry| entry.game == game.game && entry.profiles.contains_key(gpu_id));
            if let Some(entry) = linked
                && !names.contains(&entry.name.as_str())
            {
                names.push(entry.name.as_str());
            }
        }
        names
    }

    /// The status bar's automatic switching item, such as "AUTO · CYBERPUNK 2077 → QUIET": for
    /// the selected GPU, or else whether automatic switching is on. None without a session.
    pub(crate) fn switching_item(&self) -> Option<String> {
        let status = self.switching.status.as_ref()?;
        let summary = match &self.selection {
            Some(Selection::Gpu(gpu_id)) => self.switching_status(gpu_id)?.summary,
            _ => match &status.condition {
                SwitchingCondition::Ready {} => "On".into(),
                condition => explained_condition(condition).0,
            },
        };
        Some(format!("AUTO · {}", summary.to_uppercase()))
    }

    /// Why `request` can't be sent for the GPU now, when it can't.
    fn switching_unavailable(&self, gpu_id: &str, request: SwitchingRequest) -> Option<String> {
        self.switching.status.as_ref()?;
        let applies = match request {
            SwitchingRequest::Pause => !self.switching_paused(gpu_id),
            SwitchingRequest::Resume => self.switching_resumable(gpu_id),
            SwitchingRequest::Retry => self.switching_failed(gpu_id),
        };
        if !applies {
            return Some(
                match request {
                    SwitchingRequest::Pause => "Already paused",
                    SwitchingRequest::Resume => "Not paused",
                    SwitchingRequest::Retry => "Automatic switching hasn't failed",
                }
                .into(),
            );
        }
        if request == SwitchingRequest::Retry && self.switching_blocked().is_some() {
            return Some("Automatic switching is blocked".into());
        }
        if let Some(reason) = self.tuning_access().reason() {
            return Some(format!("Needs tuning access · {reason}"));
        }
        // Pause and Resume share a button, which waits for either; Retry waits for its own.
        let retry = request == SwitchingRequest::Retry;
        self.switching
            .sent
            .values()
            .find(|(sent, kind)| sent == gpu_id && (*kind == SwitchingRequest::Retry) == retry)
            .map(|(_, sent)| sent.waiting().into())
    }

    /// Why the palette's command can't run now, when it can't; none when `command` isn't one.
    pub(crate) fn switching_command_unavailable(
        &self,
        command: &Command,
    ) -> Option<Option<String>> {
        match command {
            Command::SetSwitchingPaused { gpu_id, paused } => {
                Some(self.switching_unavailable(gpu_id, SwitchingRequest::setting_paused(*paused)))
            }
            Command::RetrySwitching(gpu_id) => {
                Some(self.switching_unavailable(gpu_id, SwitchingRequest::Retry))
            }
            _ => None,
        }
    }

    /// Pause, Resume and Retry for each GPU they apply to, as the palette lists them. Retry is
    /// called "Retry ‹GPU›", unless the GPU's cooling fault already has that title.
    pub(crate) fn switching_commands(&self) -> Vec<(Command, String)> {
        if self.switching.status.is_none() {
            return Vec::new();
        }
        let cooling: Vec<_> = self
            .retry_commands()
            .into_iter()
            .map(|(_, title)| title)
            .collect();
        let mut commands = Vec::new();
        for (index, gpu) in self.gpus().iter().enumerate() {
            let label = gpu_label(index, gpu);
            let id = &gpu.id;
            if !self.switching_paused(id) {
                commands.push((
                    Command::SetSwitchingPaused {
                        gpu_id: id.clone(),
                        paused: true,
                    },
                    format!("Pause automatic switching on {label}"),
                ));
            }
            if self.switching_resumable(id) {
                commands.push((
                    Command::SetSwitchingPaused {
                        gpu_id: id.clone(),
                        paused: false,
                    },
                    format!("Resume automatic switching on {label}"),
                ));
            }
            if self.switching_failed(id) {
                let retry = format!("Retry {label}");
                let title = if cooling.contains(&retry) {
                    format!("Retry automatic switching on {label}")
                } else {
                    retry
                };
                commands.push((Command::RetrySwitching(id.clone()), title));
            }
        }
        commands
    }

    /// Asks prismd to pause or resume automatic switching on the GPU.
    pub(crate) fn set_switching_paused(&mut self, gpu_id: String, paused: bool) -> Vec<Effect> {
        self.send_switching(
            gpu_id.clone(),
            SwitchingRequest::setting_paused(paused),
            DaemonRequest::SetGpuSwitchingPaused(SetGpuSwitchingPausedRequest { gpu_id, paused }),
        )
    }

    /// Asks prismd to try the GPU's failed automatic switching again.
    pub(crate) fn retry_switching(&mut self, gpu_id: String) -> Vec<Effect> {
        self.send_switching(
            gpu_id.clone(),
            SwitchingRequest::Retry,
            DaemonRequest::RetryAutomaticSwitching(StartupGpuRequest { gpu_id }),
        )
    }

    fn send_switching(
        &mut self,
        gpu_id: String,
        request: SwitchingRequest,
        daemon_request: DaemonRequest,
    ) -> Vec<Effect> {
        if self.switching_unavailable(&gpu_id, request).is_some() {
            return Vec::new();
        }
        let id = self.drafts.next_command();
        self.switching.outcomes.remove(&gpu_id);
        self.switching.sent.insert(id, (gpu_id, request));
        vec![Effect::CommandPrismd {
            id,
            request: daemon_request,
        }]
    }

    /// Takes in prismd's answer to a Pause, Resume or Retry, if `command` was one, keeping why
    /// it was refused. Says whether it was one.
    pub(crate) fn switching_answered(
        &mut self,
        command: CommandId,
        response: &DaemonResponse,
    ) -> bool {
        let Some((gpu_id, request)) = self.switching.sent.remove(&command) else {
            return false;
        };
        let why = match response {
            DaemonResponse::Error(error) => Some(error.message.clone()),
            DaemonResponse::Command(answer) if !answer.success => {
                Some(answer.error.clone().unwrap_or_default())
            }
            _ => None,
        };
        if let Some(why) = why {
            let why = (!why.is_empty()).then(|| sentence(&why));
            self.switching
                .outcomes
                .insert(gpu_id, not_applied(request.refused(), why.as_deref()));
        }
        true
    }

    /// Takes in a new session's automatic switching status, without notifying about a failure
    /// pause already there.
    pub(crate) fn switching_connected(&mut self, status: AutomaticSwitchingStatus) {
        self.switching.status = Some(status);
    }

    /// Takes in prismd's push of the caller's automatic switching status. A new failure pause
    /// notifies, and one that ended takes its notification back.
    pub(crate) fn switching_pushed(&mut self, status: AutomaticSwitchingStatus) -> Vec<Effect> {
        let before = self.switching.status.replace(status);
        let gpu_ids: Vec<_> = self.gpus().iter().map(|gpu| gpu.id.clone()).collect();
        let mut effects = Vec::new();
        for gpu_id in gpu_ids {
            let was = state_in(before.as_ref(), &gpu_id);
            if self.switching_state(&gpu_id) != was {
                self.switching.outcomes.remove(&gpu_id);
            }
            let subject = Subject::Gpu(gpu_id.clone(), GpuConcern::Switching);
            match (in_failure_pause(was), self.switching_failed(&gpu_id)) {
                (false, true) => {
                    let summary = format!(
                        "{} on {}",
                        self.switching_failure(&gpu_id).unwrap_or_default(),
                        self.gpu_name(&gpu_id)
                    );
                    let body = self
                        .switching_status(&gpu_id)
                        .map(|status| status.detail.join(" "))
                        .unwrap_or_default();
                    effects.extend(
                        self.notifications
                            .send(subject, summary, body)
                            .map(Effect::Notify),
                    );
                }
                (true, false) => {
                    effects.extend(
                        self.notifications
                            .withdraw(&subject)
                            .map(Effect::WithdrawNotification),
                    );
                }
                _ => {}
            }
        }
        effects
    }

    /// Each GPU's failure pause, as Activity lists it among the problems.
    pub(crate) fn switching_problems(&self) -> Vec<ActivityItem> {
        self.gpus()
            .iter()
            .filter_map(|gpu| {
                let title = self.switching_failure(&gpu.id)?;
                let status = self.switching_status(&gpu.id)?;
                Some(ActivityItem {
                    title: title.into(),
                    detail: [self.gpu_name(&gpu.id)]
                        .into_iter()
                        .chain(status.detail)
                        .chain(status.outcome)
                        .collect(),
                    when: None,
                    action: Some(
                        self.button("Retry", ActivityAction::RetrySwitching(gpu.id.clone())),
                    ),
                })
            })
            .collect()
    }

    /// The session ended: the status goes with it, and requests it didn't answer may or may not
    /// have happened.
    pub(crate) fn switching_lost_session(&mut self) {
        self.switching = Switching::default();
    }
}

/// Why automatic switching doesn't drive a GPU prismd doesn't list, in short and explained.
fn explained_condition(condition: &SwitchingCondition) -> (String, Vec<String>) {
    match condition {
        SwitchingCondition::Ready {} => (
            "No linked game running".into(),
            vec!["Automatic switching applies a game's linked profile while the game runs.".into()],
        ),
        SwitchingCondition::Off {} => (
            "Off".into(),
            vec!["Automatic switching is off. Turn it on in Games.".into()],
        ),
        SwitchingCondition::Blocked { reason } => ("Blocked".into(), vec![sentence(reason)]),
        SwitchingCondition::RecognitionUnavailable { reason } => (
            "Can't recognize games".into(),
            vec![
                sentence(reason),
                "Current tuning values are held until recognition works again.".into(),
            ],
        ),
    }
}
