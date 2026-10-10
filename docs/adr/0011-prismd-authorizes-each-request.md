# prismd accepts any local connection and authorizes each request

[How does Prism onboard a host prismd and handshake its protocol version?](https://github.com/tobytaken/prism-electron/issues/112) decided how a Prism GUI gets access to `prismd`. Before, there were two gates: the socket file (`0660`, owned by the admin group) and the daemon's own check of the caller's `SO_PEERCRED` user against the host user database. A sandboxed Flatpak GUI could fail the file check even when the daemon would have allowed it, so a user in `wheel` still got a bare `EACCES` that the GUI couldn't explain. The socket is now always `0666`, and the daemon is the only gate. It still uses the group model (`admin_group`, defaulting to `wheel`, then `sudo`, then `root`, plus an optional `admin_user`). It checks each **request**, not each connection: a user who isn't allowed gets a **read-only session** with the snapshot, events, telemetry and Activity, and every write request is refused with `NoPermission`. The GUI shows this as *no tuning access*, naming the user and the group it lacks, while monitoring still comes from `prismd` and not from the more fragile in-process NVML path (ADR-0010).

## Considered Options

- **Keep the `0660` group socket and have Flatpak users set `admin_user`.** Rejected because the file check adds a failure mode the daemon already covers, and it gives a worse error.
- **Authorize with a polkit action (`allow_active=yes`).** This needs no configuration, but it adds a D-Bus/polkit dependency to the daemon and a policy file to every package. Rejected for v1 because the first user on the target distros is already in `wheel` or `sudo`. The GUI only sees "allowed or not, and why", so polkit can replace the group model later without any GUI change.
- **Refuse the whole session for users who aren't allowed.** Rejected because monitoring comes first, and none of what the daemon shows is secret: any user can already read it with `nvidia-smi`.

## Consequences

- Any local process can connect to `prismd` and read GPU state. Writes still need the admin group or `admin_user`.
- Authorization moves from session setup into request handling, so every write request has to be checked.
- The first exchange on a connection is the version handshake (`Hello`/`Welcome` with protocol `major.minor`). `Welcome` also tells the GUI whether the session is allowed to tune.
