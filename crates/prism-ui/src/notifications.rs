//! Desktop notifications' policy (ADR 0012): none while Prism's window has focus, at most one live
//! about each thing, a newer one replacing it, and none with write actions. Only the running GUI
//! notifies, so the core's triggers are all there is.

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
    Gpu(String),
}

/// The notifications Prism has live, and whether its window has focus.
#[derive(Debug, Default)]
pub(crate) struct Notifications {
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

    /// The notification about `subject`, in place of the live one about it, unless the window
    /// has focus, so the user is already looking.
    pub(crate) fn send(
        &mut self,
        subject: Subject,
        summary: String,
        body: String,
    ) -> Option<Notification> {
        if self.focused {
            return None;
        }
        self.next += 1;
        let id = NotificationId(self.next);
        let replaces = self.take(&subject);
        self.live.push((subject, id));
        Some(Notification {
            id,
            replaces,
            summary,
            body,
        })
    }

    /// The live notification about `subject` to withdraw, if any.
    pub(crate) fn withdraw(&mut self, subject: &Subject) -> Option<NotificationId> {
        self.take(subject)
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

    fn take(&mut self, subject: &Subject) -> Option<NotificationId> {
        let index = self.live.iter().position(|(live, _)| live == subject)?;
        Some(self.live.remove(index).1)
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
        Subject::Gpu(id.into())
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
}
