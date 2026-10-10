use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use prism_gpu::{GpuCommandError, GpuCommandErrorCode};
use prism_protocol::DaemonEvent;

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

#[derive(Debug, Clone)]
pub struct SessionPolicy {
    pub admin_group: String,
    pub admin_user: Option<String>,
}

impl SessionPolicy {
    pub fn authorize(&self, peer: &SessionPeer) -> Result<(), GpuCommandError> {
        if peer.uid == 0
            || self.admin_user.as_deref() == Some(&peer.username)
            || peer.groups.contains(&self.admin_group)
        {
            return Ok(());
        }
        Err(GpuCommandError::new(
            GpuCommandErrorCode::NoPermission,
            format!(
                "User {} is not allowed to access prismd (primary group: {})",
                peer.username, peer.primary_group
            ),
        ))
    }
}

#[derive(Default)]
pub(super) struct SessionEvents(Mutex<Subscribers>);

#[derive(Default)]
struct Subscribers {
    next_id: u64,
    senders: HashMap<u64, (SessionPeer, SyncSender<DaemonEvent>, BTreeSet<String>)>,
}

impl SessionEvents {
    pub fn subscribe(self: &Arc<Self>, peer: SessionPeer, initial: DaemonEvent) -> DaemonSession {
        let (sender, events) = mpsc::sync_channel(64);
        sender.try_send(initial).unwrap();
        let mut subscribers = self.0.lock().unwrap();
        subscribers.next_id += 1;
        let id = subscribers.next_id;
        subscribers
            .senders
            .insert(id, (peer.clone(), sender, BTreeSet::new()));
        DaemonSession {
            id,
            peer,
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
            .flat_map(|(_, _, holds)| holds.iter().cloned())
            .collect()
    }

    pub fn publish_for_users(&self, event: impl Fn(&SessionPeer) -> Option<DaemonEvent>) {
        self.0
            .lock()
            .unwrap()
            .senders
            .retain(|_, (peer, sender, _)| {
                event(peer).is_none_or(|event| sender.try_send(event).is_ok())
            });
    }

    pub fn publish(&self, event: DaemonEvent) {
        // Disconnect a stalled session instead of holding up the serialized store owner.
        self.0
            .lock()
            .unwrap()
            .senders
            .retain(|_, (_, sender, _)| sender.try_send(event.clone()).is_ok());
    }
}

/// Dropping the session releases its registration immediately, including on a client crash.
pub struct DaemonSession {
    id: u64,
    peer: SessionPeer,
    events: Option<Receiver<DaemonEvent>>,
    subscribers: Weak<SessionEvents>,
}

impl DaemonSession {
    pub fn set_edit_holds(&self, gpu_ids: Vec<String>) {
        if let Some(subscribers) = self.subscribers.upgrade() {
            if let Some((_, _, holds)) = subscribers.0.lock().unwrap().senders.get_mut(&self.id) {
                *holds = gpu_ids.into_iter().collect();
            }
        }
    }

    pub fn peer(&self) -> &SessionPeer {
        &self.peer
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
