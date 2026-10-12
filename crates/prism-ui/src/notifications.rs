//! Desktop notifications' policy (ADR 0012): none while the user has them off or Prism's window has
//! focus, at most one live about each thing, a newer one replacing it, and none with write actions.
//! Only the running GUI notifies, so the core's triggers are all there is.

/// A desktop notification for the caller to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub id: NotificationId,
    /// The live notification this one takes the place of, if any.
    pub replaces: Option<NotificationId>,
    pub summary: String,
    pub body: String,
}

/// Prism's own name for a notification it sent, which the caller maps to the server's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NotificationId(pub u64);

/// What a notification is about, which also decides where clicking it leads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Subject {
    /// The prismd connection, at Settings → Daemon.
    Prismd,
    /// A GPU, by id, at its workspace, such as its failure pause.
    Gpu(String, GpuConcern),
    /// A driver job that ended, at Drivers.
    Drivers,
}

/// What about a GPU a notification is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GpuConcern {
    /// A cooling fault, which its Fans track explains.
    Cooling,
    /// Automatic switching's failure pause, which the workspace explains.
    Switching,
}

impl Subject {
    /// Whether a notification about `self` takes the place of one about `other`: one about the
    /// same thing, or about the same GPU, which has at most one live.
    fn replaces(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Gpu(gpu, _), Self::Gpu(other, _)) => gpu == other,
            _ => self == other,
        }
    }
}

/// The notifications Prism has live, whether the user has them on, and whether its window has
/// focus.
#[derive(Debug, Default)]
pub(crate) struct Notifications {
    /// The user turned desktop notifications off. They're on by default.
    off: bool,
    /// A window that never said otherwise counts as unfocused, so nothing is missed.
    focused: bool,
    /// At most one for each subject.
    live: Vec<(Subject, NotificationId)>,
    next: u64,
}

impl Notifications {
    pub(crate) fn focus_changed(&mut self, focused: bool) {
        self.focused = focused;
    }

    /// Whether the user has desktop notifications on.
    pub(crate) fn enabled(&self) -> bool {
        !self.off
    }

    /// Turns desktop notifications on or off: the live ones to withdraw, all of them when they go
    /// off.
    pub(crate) fn set_enabled(&mut self, enabled: bool) -> Vec<NotificationId> {
        self.off = !enabled;
        if enabled {
            Vec::new()
        } else {
            self.withdraw_all()
        }
    }

    /// The notification about `subject`, in place of the live one about it, unless the user has
    /// them off, or the window has focus, so the user is already looking.
    pub(crate) fn send(
        &mut self,
        subject: Subject,
        summary: String,
        body: String,
    ) -> Option<Notification> {
        if self.off || self.focused {
            return None;
        }
        self.next += 1;
        let id = NotificationId(self.next);
        let replaces = self
            .live
            .iter()
            .position(|(live, _)| subject.replaces(live))
            .map(|index| self.live.remove(index).1);
        self.live.push((subject, id));
        Some(Notification {
            id,
            replaces,
            summary,
            body,
        })
    }

    /// The live notification about `subject` to withdraw, if any: not one about something else
    /// that took its place.
    pub(crate) fn withdraw(&mut self, subject: &Subject) -> Option<NotificationId> {
        let index = self.live.iter().position(|(live, _)| live == subject)?;
        Some(self.live.remove(index).1)
    }

    /// Every live notification, to withdraw them all.
    pub(crate) fn withdraw_all(&mut self) -> Vec<NotificationId> {
        self.live.drain(..).map(|(_, id)| id).collect()
    }

    /// The server closed it: the user dismissed or clicked it, or it expired. What it was about,
    /// if it was live.
    pub(crate) fn closed(&mut self, id: NotificationId) -> Option<Subject> {
        let index = self.live.iter().position(|(_, live)| *live == id)?;
        Some(self.live.remove(index).0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send(notifications: &mut Notifications, subject: Subject) -> Notification {
        notifications
            .send(subject, "Summary".into(), "Body".into())
            .expect("unfocused, so it's sent")
    }

    fn gpu(id: &str) -> Subject {
        Subject::Gpu(id.into(), GpuConcern::Cooling)
    }

    #[test]
    fn a_newer_notification_about_a_gpu_replaces_the_live_one_about_it() {
        let mut notifications = Notifications::default();
        let first = send(&mut notifications, gpu("GPU-0"));

        let second = send(&mut notifications, gpu("GPU-0"));

        assert_eq!(first.replaces, None);
        assert_eq!(second.replaces, Some(first.id));
        assert_ne!(second.id, first.id);
        assert_eq!(notifications.withdraw_all(), [second.id]);
    }

    #[test]
    fn a_notification_about_something_else_on_the_gpu_replaces_it_too() {
        let mut notifications = Notifications::default();
        let cooling = send(&mut notifications, gpu("GPU-0"));
        let switching = Subject::Gpu("GPU-0".into(), GpuConcern::Switching);

        let newer = send(&mut notifications, switching.clone());

        assert_eq!(newer.replaces, Some(cooling.id));
        // The cooling fault ending leaves the newer one alone.
        assert_eq!(notifications.withdraw(&gpu("GPU-0")), None);
        assert_eq!(notifications.withdraw(&switching), Some(newer.id));
    }

    #[test]
    fn each_gpu_has_its_own_live_notification() {
        let mut notifications = Notifications::default();
        let first = send(&mut notifications, gpu("GPU-0"));

        let other = send(&mut notifications, gpu("GPU-1"));
        let prismd = send(&mut notifications, Subject::Prismd);

        assert_eq!(other.replaces, None);
        assert_eq!(prismd.replaces, None);
        assert_eq!(
            notifications.withdraw_all(),
            [first.id, other.id, prismd.id]
        );
    }

    #[test]
    fn a_closed_notification_isnt_replaced() {
        let mut notifications = Notifications::default();
        let first = send(&mut notifications, gpu("GPU-0"));

        assert_eq!(notifications.closed(first.id), Some(gpu("GPU-0")));
        assert_eq!(notifications.closed(first.id), None);

        assert_eq!(send(&mut notifications, gpu("GPU-0")).replaces, None);
    }

    #[test]
    fn nothing_is_sent_while_the_window_has_focus() {
        let mut notifications = Notifications::default();
        let live = send(&mut notifications, gpu("GPU-0"));
        notifications.focus_changed(true);

        assert_eq!(
            notifications.send(gpu("GPU-0"), "Summary".into(), "Body".into()),
            None
        );
        // The live one stays until it's closed or withdrawn.
        assert_eq!(notifications.withdraw(&gpu("GPU-0")), Some(live.id));
    }

    #[test]
    fn nothing_is_sent_while_desktop_notifications_are_off() {
        let mut notifications = Notifications::default();
        let live = send(&mut notifications, gpu("GPU-0"));

        assert_eq!(notifications.set_enabled(false), [live.id]);
        assert!(!notifications.enabled());
        for subject in [gpu("GPU-0"), Subject::Prismd, Subject::Drivers] {
            assert_eq!(
                notifications.send(subject, "Summary".into(), "Body".into()),
                None
            );
        }

        assert_eq!(notifications.set_enabled(true), []);
        assert_eq!(send(&mut notifications, gpu("GPU-0")).replaces, None);
    }
}
