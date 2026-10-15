use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use prism_gpu::{GpuCommandError, GpuCommandErrorCode};
use prism_protocol::{Authorization, DaemonEvent, DaemonRequest};

/// Resolved by the socket adapter from SO_PEERCRED, never from client JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPeer {
    pub uid: u32,
    pub username: String,
    pub groups: Vec<String>,
    pub primary_group: String,
    /// The caller's home directory on the host, from the user database.
    pub home: PathBuf,
}

/// Who may change GPU tuning: root, the admin user and members of the admin group. Anyone else
/// may still read.
#[derive(Debug, Clone)]
pub struct SessionPolicy {
    pub admin_group: String,
    pub admin_user: Option<String>,
}

impl SessionPolicy {
    /// Allows the caller to change GPU tuning, or says why not, naming the group they lack.
    pub fn allow_tuning(&self, peer: &SessionPeer) -> Result<(), GpuCommandError> {
        match self.authorization(peer) {
            Authorization::NotInAdminGroup { user, group } => Err(GpuCommandError::new(
                GpuCommandErrorCode::NoPermission,
                format!("{user} isn't in group {group}"),
            )),
            Authorization::Root {}
            | Authorization::AdminUser { .. }
            | Authorization::AdminGroup { .. } => Ok(()),
        }
    }

    /// How the caller is allowed to tune, or that they aren't.
    pub fn authorization(&self, peer: &SessionPeer) -> Authorization {
        let user = peer.username.clone();
        if peer.uid == 0 {
            Authorization::Root {}
        } else if self.admin_user.as_deref() == Some(&peer.username) {
            Authorization::AdminUser { user }
        } else if peer.groups.contains(&self.admin_group) {
            Authorization::AdminGroup {
                user,
                group: self.admin_group.clone(),
            }
        } else {
            Authorization::NotInAdminGroup {
                user,
                group: self.admin_group.clone(),
            }
        }
    }

    /// Allows one request from the caller: anyone may read, and only a caller allowed to tune may
    /// write.
    pub fn authorize(
        &self,
        peer: &SessionPeer,
        request: &DaemonRequest,
    ) -> Result<(), GpuCommandError> {
        if writes(request) {
            self.allow_tuning(peer)
        } else {
            Ok(())
        }
    }
}

/// Whether the request changes anything: GPU controls, the tuning store, games, startup, driver
/// jobs, the host or the session's edit holds.
fn writes(request: &DaemonRequest) -> bool {
    match request {
        DaemonRequest::Hello(_)
        | DaemonRequest::SubscribeTelemetry
        | DaemonRequest::GetInventory
        | DaemonRequest::GetDaemonStatus
        | DaemonRequest::PreviewFanCurve(_)
        | DaemonRequest::GetStartupState
        | DaemonRequest::ListProfiles
        | DaemonRequest::GetTuningStates
        | DaemonRequest::ListSteamApps
        | DaemonRequest::ListRunningPrograms
        | DaemonRequest::GetGames
        | DaemonRequest::GetRunningGames
        | DaemonRequest::GetAutomaticSwitchingStatus
        | DaemonRequest::GetDriverStatus
        | DaemonRequest::GetDriverJob(_) => false,
        DaemonRequest::SetEditHolds(_)
        | DaemonRequest::ApplyProfile(_)
        | DaemonRequest::SetStartupBehavior(_)
        | DaemonRequest::SetStartupProfile(_)
        | DaemonRequest::DismissStartupOutcome(_)
        | DaemonRequest::SaveProfileAsNew(_)
        | DaemonRequest::SaveToCurrentProfile(_)
        | DaemonRequest::RenameProfile(_)
        | DaemonRequest::DeleteProfile(_)
        | DaemonRequest::ApplyProfileById(_)
        | DaemonRequest::CorrectAndApplyProfile(_)
        | DaemonRequest::AddProgram(_)
        | DaemonRequest::AddSteamGame(_)
        | DaemonRequest::SetGameProfile(_)
        | DaemonRequest::RemoveGame(_)
        | DaemonRequest::SetAutomaticSwitching(_)
        | DaemonRequest::SetGpuSwitchingPaused(_)
        | DaemonRequest::RetryAutomaticSwitching(_)
        | DaemonRequest::RetryCooling(_)
        | DaemonRequest::StartDriverAction(_)
        | DaemonRequest::CancelDriverJob(_)
        | DaemonRequest::RequestHostReboot => true,
    }
}

#[derive(Default)]
pub(super) struct SessionEvents(Mutex<Subscribers>);

#[derive(Default)]
struct Subscribers {
    next_id: u64,
    senders: HashMap<u64, Subscriber>,
}

/// An open session, as the daemon pushes events to it.
struct Subscriber {
    peer: SessionPeer,
    sender: SyncSender<DaemonEvent>,
    /// The GPUs it has unapplied tuning changes for.
    holds: BTreeSet<String>,
    /// Whether it subscribed to telemetry.
    telemetry: bool,
}

impl SessionEvents {
    pub fn subscribe(self: &Arc<Self>, peer: SessionPeer, policy: SessionPolicy) -> DaemonSession {
        let (sender, events) = mpsc::sync_channel(64);
        let mut subscribers = self.0.lock().unwrap();
        subscribers.next_id += 1;
        let id = subscribers.next_id;
        subscribers.senders.insert(
            id,
            Subscriber {
                peer: peer.clone(),
                sender,
                holds: BTreeSet::new(),
                telemetry: false,
            },
        );
        DaemonSession {
            id,
            peer,
            policy,
            events: Some(events),
            subscribers: Arc::downgrade(self),
        }
    }

    pub fn held_gpus(&self) -> BTreeSet<String> {
        self.0
            .lock()
            .unwrap()
            .senders
            .values()
            .flat_map(|subscriber| subscriber.holds.iter().cloned())
            .collect()
    }

    pub fn publish_for_users(&self, event: impl Fn(&SessionPeer) -> Option<DaemonEvent>) {
        self.0.lock().unwrap().senders.retain(|_, subscriber| {
            event(&subscriber.peer).is_none_or(|event| subscriber.sender.try_send(event).is_ok())
        });
    }

    pub fn publish(&self, event: DaemonEvent) {
        // Disconnect a stalled session instead of holding up the serialized store owner.
        self.0
            .lock()
            .unwrap()
            .senders
            .retain(|_, subscriber| subscriber.sender.try_send(event.clone()).is_ok());
    }

    /// Whether any open session subscribed to telemetry.
    pub fn telemetry_subscribed(&self) -> bool {
        self.0
            .lock()
            .unwrap()
            .senders
            .values()
            .any(|subscriber| subscriber.telemetry)
    }

    /// Pushes the event to every session that subscribed to telemetry.
    pub fn publish_telemetry(&self, event: DaemonEvent) {
        self.0.lock().unwrap().senders.retain(|_, subscriber| {
            !subscriber.telemetry || subscriber.sender.try_send(event.clone()).is_ok()
        });
    }
}

/// Dropping the session releases its registration immediately, including on a client crash.
pub struct DaemonSession {
    id: u64,
    peer: SessionPeer,
    /// Checked on each of the session's requests.
    policy: SessionPolicy,
    events: Option<Receiver<DaemonEvent>>,
    subscribers: Weak<SessionEvents>,
}

impl DaemonSession {
    pub fn set_edit_holds(&self, gpu_ids: Vec<String>) {
        self.update(|subscriber| subscriber.holds = gpu_ids.into_iter().collect());
    }

    /// Opts the session in to the telemetry pushed on each tick.
    pub fn subscribe_telemetry(&self) {
        self.update(|subscriber| subscriber.telemetry = true);
    }

    fn update(&self, change: impl FnOnce(&mut Subscriber)) {
        if let Some(subscribers) = self.subscribers.upgrade()
            && let Some(subscriber) = subscribers.0.lock().unwrap().senders.get_mut(&self.id)
        {
            change(subscriber);
        }
    }

    pub fn peer(&self) -> &SessionPeer {
        &self.peer
    }

    pub fn policy(&self) -> &SessionPolicy {
        &self.policy
    }

    /// Allows one of the session's requests, or says why its caller may not make it.
    pub fn authorize(&self, request: &DaemonRequest) -> Result<(), GpuCommandError> {
        self.policy.authorize(&self.peer, request)
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<DaemonEvent, RecvTimeoutError> {
        self.events
            .as_ref()
            .expect("session event receiver already taken")
            .recv_timeout(timeout)
    }

    pub(super) fn take_events(&mut self) -> Receiver<DaemonEvent> {
        self.events.take().unwrap()
    }
}

impl Drop for DaemonSession {
    fn drop(&mut self) {
        if let Some(subscribers) = self.subscribers.upgrade() {
            subscribers.0.lock().unwrap().senders.remove(&self.id);
        }
    }
}
