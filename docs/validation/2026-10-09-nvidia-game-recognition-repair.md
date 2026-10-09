# NVIDIA game recognition repair and replay

This follow-up to the [failed live pass](2026-10-09-nvidia-game-recognition.md)
covers the recognition repairs, support wording, regression checks and another
local native/Proton lifecycle pass. The agreed scope excludes actual GPU tuning
writes and the wider Flatpak validation matrix. [Issue #49](https://github.com/tobytaken/prism-electron/issues/49)
remains open; automatic-switching release still requires those validations.

## Repair contracts

- Skip positively identified kernel threads and exited zombie/dead tasks, rather
  than treating their missing userspace environment as a failed observation of a
  game. A read failure is an exit only when the process directory is confirmed
  absent. An unreadable live process or reused PID remains unknown, preventing
  false game-end restoration.
- Obtain mapped-file identities through the mapping-specific `map_files` link.
  The textual device number in `maps` is not substituted for a stat identity,
  and a displayed pathname or inode alone is never proof of a match. Normalize
  the address range: `maps` zero-pads low addresses; `map_files` does not.
- Preserve Wine system-directory and runtime-directory context even when a
  host path cannot be recovered. Demote `srt-bwrap` and `pv-adverb` along with known Wine helpers;
  an unrelated game named `services.exe` remains an ordinary program.
- Qualify README and automatic-switching UI claims while the release gate is
  incomplete. Keep the existing off-by-default opt-in and manual-program caveat.

The replay also exposed an unrelated AppImage's FUSE mount denying full executable
metadata even to root. With a manual association configured, that denial blocked
the complete scan. A successful device-only query can now exclude an executable
on a different filesystem from every candidate; if it could share a candidate's
device, full identity is still required and errors remain unknown. Wine mappings
are checked separately even when the loader itself cannot match a candidate.
This changes neither host permissions nor the daemon's credentials.

The distinction is supported by Linux's [statx interface](https://man7.org/linux/man-pages/man2/statx.2.html)
and [FUSE getattr implementation](https://github.com/torvalds/linux/blob/master/fs/fuse/dir.c),
which allows a zero-mask query to return the containing device without granting
access to owner-controlled inode attributes. The [map_files interface](https://man7.org/linux/man-pages/man5/proc_pid_map_files.5.html)
provides links to the actual mapped files, including across mount namespaces.

## Verification

The recorded lifecycle replay is summarized below. Local raw observations and
screenshots are kept in the ignored `artifacts/recognition-repair/` directory.

The lifecycle replay used a release daemon built from the repair working tree on base
`57f4037ccc68815c852003c256ff880c2c79bfe8`. Its SHA-256 is
`cd491b2f8f7d9451ed324c7378d9a480c61e76ac00cd131470876154603b7152`.
No installed daemon or system service was replaced. Each test daemon used fresh
persistent and boot-scoped directories under a private `/run/prism-recognition-repair-*`
directory, with startup off and automatic switching off. Both manual program
associations had no linked GPU profiles.

With Node 24.3.0, `bun run check` passed lint, TypeScript, 244 renderer tests,
65 Electron tests, 342 Rust unit tests, 12 sidecar integration tests and all three
cooling-supervision checks. After the final mapping-address and helper-directory corrections, the
complete Rust suite passed again. Formatting and whitespace checks passed.
The first full run hit an existing GPU-polling timeout; its targeted retry and
subsequent full runs passed. No production changes were made for that timeout.

Regression coverage now includes non-userspace tasks with unreadable environments,
a verified process exit during a scan, PID reuse during a scan, btrfs maps/stat
device disagreement, zero-padded maps addresses, misleading namespace paths,
inaccessible mapping links retaining game tuning, anonymous mappings, and helper
classification without a recovered host path. The FUSE exclusion was exercised
on the live AppImage mount; the fixture suite does not mount FUSE filesystems.


## Recorded lifecycle results

On the original CachyOS/btrfs/RTX 5090 host, both games reached their main menus.
Both manual associations were present before the idle samples, with no linked
profiles and automatic switching disabled. The final lifecycle trace contains
456 samples, all `available`, collected about every 0.5 seconds against the
production daemon's `get_running_games` request.

| Phase | Samples | Duration | Observation |
| --- | ---: | ---: | --- |
| Idle | 20 | 9.56 s | No running games |
| Native launch | 32 | 15.60 s | Steam app 457140 and native executable appeared |
| Native running | 122 | 60.92 s | Both identities remained recognized |
| Native exit | 23 | 11.07 s | Both entries cleared; no false surviving game |
| Proton launch | 64 | 31.71 s | Steam app 2379780 and mapped Balatro.exe appeared |
| Proton running | 154 | 77.59 s | Both identities remained recognized |
| Proton exit | 41 | 20.17 s | Both entries cleared; no false surviving game |

The native picker returned the real Oxygen Not Included executable with launcher
Steam. The Proton picker returned the real Balatro.exe host path with launcher
Steam, `helper: false` and no recognition warning. This directly repairs the
previous null-path result on this btrfs host. The unrelated AppImage remained
running throughout; its unreadable inode metadata no longer invalidated scans
of these games on another filesystem.

The lifecycle build still exposed Wine helpers through their actual
`lib/wine/x86_64-windows` runtime paths rather than the prefix's `system32` aliases.
The final picker-only correction adds both x86_64 and i386 Wine runtime directories;
it does not alter recognition. Its release binary SHA-256 is
`945ac86c140b8430dcaf503ba9e83efe1e592ad69dcf992c5164de68fc518e59`.

A fresh isolated daemon using that final binary passed the live picker recheck:
`services.exe`, `explorer.exe`, `winedevice.exe`, `rpcss.exe`, `srt-bwrap` and
`pv-adverb` were all marked as helpers, and every helper followed ordinary
programs. Balatro kept its verified host path, no recognition warning, and
`helper: false`. Both its Steam and manual-program identities were recognized
on this final build and cleared within about 1.5 seconds after closure.

[Selected machine-readable evidence](2026-10-09-nvidia-game-recognition-repair.json)
preserves the lifecycle counts, representative responses, final helper entries,
binary hashes and before/after checks. The earlier failed diagnostic replays
remain in local artifacts; they are not included in the successful lifecycle
count.

## Cleanup and remaining gate

Both games and every test daemon were closed; the final game and daemon PIDs were
verified absent. Each test daemon removed its private runtime directory. The
original tuning store retained its pre-test modification time. Before and after
readings remained 540 W power limit, +240 MHz graphics offset, +450 MHz memory
offset, and automatic cooling on both fans. Startup behavior and automatic
switching stayed off in the isolated stores, with no profiles linked to games.

This pass establishes recognition and picker behavior for the tested native
Oxygen Not Included and Proton Balatro forms on this host. It does not certify
actual profile application/restoration, startup/reboot behavior, cooling recovery,
Flatpak Steam, plain Wine, umu/Heroic/Lutris, document-portal recovery, Snap or
gamescope. Keep #49 open and automatic-switching release blocked until the
remaining required forms and actual automatic tuning pass their live gates.
