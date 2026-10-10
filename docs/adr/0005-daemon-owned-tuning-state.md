# The daemon owns tuning state, split by lifetime

For [Define profile persistence, compatibility, and failure contracts](https://github.com/tobytaken/prism-electron/issues/8), the root host daemon is the single store for GPU tuning profiles, game associations, each user's automatic switching opt-in, startup behavior, and startup profiles. ADR-0002 and ADR-0003 require the daemon to act on the latest profile values with no app running, and a Flatpak app's files live inside its sandbox, so the daemon holds the only copy instead of mirroring the app's. Each item belongs to the caller identified by the daemon's peer credentials, keyed by uid with the username recorded so that a reused uid doesn't inherit another account's profiles. Users see only their own profiles, except the host-wide startup profile and current profile, which are shown to others by name. The app becomes a client, importing its existing per-user profile file once.

_Amended 2026-10-10:_ [#121](https://github.com/tobytaken/prism-electron/issues/121) removed the one-time profile-file import. Prism was never released, so there are no profile files to carry over.

_Amended 2026-10-10:_ [#123](https://github.com/tobytaken/prism-electron/issues/123) changed the tuning store's keys to match the daemon's Rust field names, without a migration. A store written before that change is reported unhealthy and preserved untouched, as for any store the daemon can't read; since Prism was never released, only development stores are affected, and they are deleted by hand.

Daemon state is split by how long it must live:

- **Persistent** (survives reboots): profiles, associations, opt-ins, startup behavior and startup profiles, current-profile references, last applied tuning values, explicit automatic switching pauses, and the record used to detect an unclean shutdown.
- **Boot-scoped** (survives daemon restarts, cleared at reboot): restoration baselines, manual overrides, failure pauses and their recovery context, automatic switching status, and the marker that startup behavior has already run. Keeping it on a tmpfs stamped with the kernel boot id makes "once per host boot" and "a previous boot's baseline is never a restoration target" hold by construction.
- **Connection-scoped**: edit holds belong to a long-lived app session connection and are released the moment it closes, including after a crash.

Persistent writes are atomic and versioned, and the daemon fails closed. If saved state can't be read, or was written by a newer version, it is preserved untouched, and the daemon runs no startup behavior and makes no automatic writes; manual live tuning keeps working. This replaces today's behavior of silently starting from empty state, which would lose a restoration baseline or reapply the wrong values.

## Considered options

- **The app keeps its profile file and pushes copies to the daemon**: rejected because the two copies drift when edits happen while either side is down, and the Flatpak sandbox hides the file from the host.
- **Lease-based edit holds over one-shot requests**: rejected because a crashed app would defer automatic switching until the lease expired, and the daemon needs a long-lived channel anyway to push automatic switching status.
- **One persistent state file for everything**: rejected because boot-scoped recovery state would then need explicit invalidation at each boot, and a mistake there could restore a previous boot's baseline.
