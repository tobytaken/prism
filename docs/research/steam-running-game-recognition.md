# Recognizing running Steam games from the daemon

Research for [#23](https://github.com/tobytaken/prism-electron/issues/23), a child of the map [#2](https://github.com/tobytaken/prism-electron/issues/2). Background: the [resolution of #5](https://github.com/tobytaken/prism-electron/issues/5#issuecomment-5969896382) and ADR 0003 (daemon-driven game switching).

**Question.** How can the root host daemon `prismd` recognize, within a few seconds, that a specific user's Steam **game** is a **running game**, identified by its Steam app ID? The answer must cover native and Proton games under distribution-package and Flatpak Steam, and should cover Snap Steam and gamescope sessions where possible.

This document reports facts and how confident we are in each. It does not decide product behaviour.

**Method and limits.** Sources are Valve's Steam Runtime documentation (`steam-runtime-tools`), Valve's Proton and gamescope source, Valve's `steam-for-linux` tracker (comments from Valve staff and Valve contractors are marked as such), Flatpak and snapd source and documentation, Wine documentation, and Linux kernel source, documentation, and man pages. **Steam is not installed on the research host** (Ubuntu 26.04, kernel 7.2), so no Steam process tree was observed directly. Everything about live Steam process trees comes from Valve documentation, Valve source code that parses those trees, or command lines that users logged on Valve's tracker.

Confidence levels:

- **High**: stated by the owning project's documentation or source.
- **Medium**: consistent observations on Valve's tracker, or a strong inference from primary sources.
- **Low**: a single report, or not verified.

## Summary

| Question | Answer | Confidence |
|---|---|---|
| What carries the app ID? | The `reaper` process's command line (`reaper SteamLaunch AppId=<id> -- …`). The `SteamAppId`, `SteamGameId`, and `STEAM_COMPAT_APP_ID` environment variables of every process Steam starts for the game. On SteamOS only, a cgroup named `app-steam-app<id>-<reaperpid>.scope`. | High |
| How does the **launcher** differ from the game? | Steam client processes have no reaper ancestor and no `SteamAppId`. Game processes have one or both. | High (native Steam), Medium (Flatpak Steam) |
| Can the daemon see processes inside Flatpak and Snap sandboxes? | Yes. Root in the initial PID namespace sees every process with its host PID and can read its `/proc/<pid>/{cmdline,environ,status,cgroup}`. | High (Flatpak), Medium (Snap) |
| How is the owning user found? | The real UID in `/proc/<pid>/status`, which the kernel reports in the daemon's (initial) user namespace. The cgroup path `user.slice/user-<uid>.slice/…` corroborates it. | High |
| How is the end of a game detected? | The reaper is a subreaper and outlives every descendant, so its exit marks the end. Under Flatpak Steam the game also runs in a separate systemd scope that becomes empty when the game ends. Proton's `wineserver` stays alive for about 3 s after the last Wine process exits. | High (mechanism), Medium (Flatpak details) |
| Can the **game launch** phase be told apart? | Partly. Pre-launch shader processing and install scripts are distinguishable. Steam Linux Runtime container setup and Proton prefix setup/upgrade run inside the main reaper with the app ID set, so they can't be separated by app ID. | Medium |
| What latency is achievable? | Kernel proc-connector exec events detect the reaper as soon as it starts. Polling detects it within one interval; a full `/proc` scan cost about 4 ms on this host. | High |

## 1. What carries the Steam app ID

### 1.1 The `reaper` process and its command line

Valve documents that every Steam game is wrapped in a `steam-launch-wrapper` process and a `reaper` process. The reaper "sets itself as a subreaper using PR_SET_CHILD_SUBREAPER". This applies to native Linux games, Windows games under Proton, and non-Steam shortcuts ([steam-compat-tool-interface.md, "Native Linux Steam games", "Windows games…", "Non-Steam games"](https://gitlab.steamos.cloud/steamrt/steam-runtime-tools/-/blob/main/docs/steam-compat-tool-interface.md)). Valve also states that every game on Steam runs under some compatibility tool, with "no option for running directly on the host system" ([slr-for-game-developers.md](https://gitlab.steamos.cloud/steamrt/steam-runtime-tools/-/blob/main/docs/slr-for-game-developers.md)). **High.**

The reaper's command line carries the app ID. Valve's compositor gamescope recognizes a window's game by walking up the window's process tree to a process whose `comm` is `reaper` and parsing `SteamLaunch` and `AppId=<n>` from its `/proc/<pid>/cmdline`, stopping at `--` ([gamescope `src/Utils/Process.cpp`, `GetAppIdFromReaper`](https://github.com/ValveSoftware/gamescope/blob/master/src/Utils/Process.cpp)). Command lines that users logged from Steam's `console_log.txt` show the same shape across clients and years. **High.**

- Legacy 32-bit client, 2023: `…/ubuntu12_32/reaper SteamLaunch AppId=570 -- …/ubuntu12_32/steam-launch-wrapper -- '…/SteamLinuxRuntime_sniper'/_v2-entry-point --verb=waitforexitandrun -- '…/dota 2 beta/game/dota.sh' …` ([steam-for-linux#9835](https://github.com/ValveSoftware/steam-for-linux/issues/9835), [#10143](https://github.com/ValveSoftware/steam-for-linux/issues/10143) for Proton 8 + sniper).
- Experimental 64-bit "SteamRT3" client, August 2026, logged by Valve contractor smcv: `steam-runtime-launch-client --alongside-steam … steam-runtime-supervisor … /bin/sh -c …/steamrt64/steam-launch-wrapper -- …/steamrt64/reaper SteamLaunch AppId=312990 -- '…/Proton 5.0'/proton waitforexitandrun '…/Expendabros.exe'` ([steam-for-linux#13520](https://github.com/ValveSoftware/steam-for-linux/issues/13520)). Two things differ in this client: the wrapper and the reaper are swapped, and games are started "alongside Steam" through a launcher service, outside the client's container. The `reaper SteamLaunch AppId=` marker is unchanged.

A Valve employee (TTimo) confirmed that the reaper is also used to run install scripts before the game starts. The reporter observed that the install-script reaper carries an extra `Install=1` argument and has a different PID from the reaper of the game itself ([steam-for-linux#7876](https://github.com/ValveSoftware/steam-for-linux/issues/7876)). The format is undocumented, so Valve can change it at any time; the reporter warned of this too. **Medium.**

### 1.2 Environment variables

Valve's documentation for compatibility tools ([Environment section](https://gitlab.steamos.cloud/steamrt/steam-runtime-tools/-/blob/main/docs/steam-compat-tool-interface.md)) says:

- `STEAM_COMPAT_APP_ID` is "the decimal app-ID of the game". It is set for Steam games, including install-script runs. It is `0` for non-Steam shortcuts and is not set in the legacy scout `LD_LIBRARY_PATH` runtime.
- `SteamAppId` is "the same as `STEAM_COMPAT_APP_ID`, but only when running the actual game; not set when running install scripts or other setup commands. The Steamworks API assumes that every process with this environment variable set is part of the actual game." It is not set for non-Steam shortcuts.

**High.**

`SteamGameId` is also set. Proton uses it interchangeably with the app ID for per-game fixes; for example, it compares `SteamGameId` with `"311210"` and names its log `steam-<SteamGameId>.log`. Proton reads `SteamAppId` and `STEAM_COMPAT_APP_ID` as well ([Proton `proton` script](https://github.com/ValveSoftware/Proton/blob/proton_10.0/proton)). For ordinary Steam apps, `SteamGameId` equals the app ID. For shortcuts and mods it is a different, wider identifier. **Medium** (inferred from how Proton uses it; not documented by Valve).

Child processes inherit the environment, so it reaches every descendant, including processes inside the Steam Linux Runtime (pressure-vessel) container. pressure-vessel reads `STEAM_COMPAT_APP_ID` and `SteamAppId` itself ([pressure-vessel `wrap.1.md`, ENVIRONMENT](https://gitlab.steamos.cloud/steamrt/steam-runtime-tools/-/blob/main/pressure-vessel/wrap.1.md)). Proton runs inside the sniper container and branches on these variables. **High.**

Reading these variables from another process: `/proc/<pid>/environ` holds "the initial environment that was set when the currently executing program was started via execve(2)", and later `putenv` calls are not reflected. Access is governed by a `PTRACE_MODE_READ_FSCREDS` check ([proc_pid_environ(5)](https://man7.org/linux/man-pages/man5/proc_pid_environ.5.html)).

- Root with `CAP_SYS_PTRACE` passes that check.
- Yama's `ptrace_scope` restricts only `PTRACE_MODE_ATTACH` ([`security/yama/yama_lsm.c`](https://github.com/torvalds/linux/blob/master/security/yama/yama_lsm.c)), so it doesn't affect environment reads.
- AppArmor skips the check on the target's side when the reading process is unconfined ([`security/apparmor/task.c`, `profile_tracee_perm`](https://github.com/torvalds/linux/blob/master/security/apparmor/task.c)).

**High.** A user's launch options are inserted inside the command line Steam builds (`%command%`; [Launch options](https://gitlab.steamos.cloud/steamrt/steam-runtime-tools/-/blob/main/docs/steam-compat-tool-interface.md)), so a user could deliberately unset or override these variables for their own game. That would be unusual. **Medium.**

### 1.3 cgroups

- **SteamOS / gamescope:** Steam places each game in a systemd scope named `app-steam-app<appid>-<reaperpid>.scope`. gamescope reads the app ID from `/proc/<pid>/cgroup` first and falls back to the reaper walk "for cases where the Steam client doesn't put games it launches in a scope cgroup (currently the case on desktop)" ([gamescope commit 565397f, May 2026](https://github.com/ValveSoftware/gamescope/commit/565397f8cc)). **High.**
- **Desktop Steam (distribution package):** games stay in the Steam client's cgroup (per the same commit), so on desktop **the cgroup does not separate the launcher from the game**. pressure-vessel's `--systemd-scope` option, which creates scopes named like `app-steam-app975370-12345.scope`, is off by default ([wrap.1.md](https://gitlab.steamos.cloud/steamrt/steam-runtime-tools/-/blob/main/pressure-vessel/wrap.1.md)). **High.**
- **Flatpak Steam:** see §3.1. The game's sub-sandbox gets its own scope, `app-flatpak-com.valvesoftware.Steam-<instance>.scope`, whose name **does not carry the app ID**.

### 1.4 Steam's own state files

- `registry.vdf` → `RunningAppID` has been always 0 or missing on Linux since the 2023 UI rewrite. The bug is still open, with confirmation from July 2026 ([steam-for-linux#9672](https://github.com/ValveSoftware/steam-for-linux/issues/9672)). **Not usable.** **High.**
- Steam's `console_log.txt` logs a `Game process added : AppID 570 "<reaper command line>"` line ([#9835](https://github.com/ValveSoftware/steam-for-linux/issues/9835)). The format is undocumented, the file lives in each user's Steam directory (inside `~/.var/app/…` for Flatpak), and we could not confirm a matching "removed" line. **Low** as a recognition source.

## 2. Telling the launcher apart from the game

- Steam client processes (`steam`, `steamwebhelper`, the client's `pressure-vessel`/`srt-*` helpers) are not descendants of a `reaper SteamLaunch AppId=` process and are not started with `SteamAppId`. The reaper and the game process tree are separate processes started per launch ([compat-tool interface](https://gitlab.steamos.cloud/steamrt/steam-runtime-tools/-/blob/main/docs/steam-compat-tool-interface.md); gamescope relies on the same distinction). **High.**
- The Steam overlay helper `gameoverlayui` is started once per running game, but it is a child of the Steam client: Steam's own assertion reads "reaping pid: … -- gameoverlayui" from `src/steamexe/main.cpp` ([#9003](https://github.com/ValveSoftware/steam-for-linux/issues/9003), [#13682](https://github.com/ValveSoftware/steam-for-linux/issues/13682)). We don't know whether its environment contains `SteamAppId`. **Low.** It ends with its game, so counting it would rarely change the outcome.
- "Everything Steam starts for that app ID", including in-game publisher launchers, anti-cheat helpers, and Proton's Wine services (`wineserver`, `services.exe`, `explorer.exe`), sits under the reaper and inherits `SteamAppId`. The one exception is the Flatpak sub-sandbox case in §3.1, where ancestry is broken but the environment survives. **High.**
- **Non-Steam shortcuts** are also wrapped in a reaper, with `STEAM_COMPAT_APP_ID=0` and no `SteamAppId` ([compat-tool interface, "Non-Steam games"](https://gitlab.steamos.cloud/steamrt/steam-runtime-tools/-/blob/main/docs/steam-compat-tool-interface.md)). **High.** What the reaper's `AppId=` value is for a shortcut is unverified. **Low.** The two environment variables are enough to exclude shortcuts, which #5 routes to the manual fallback.

## 3. Visibility from a root daemon by Steam install

### 3.1 Flatpak Steam (`com.valvesoftware.Steam`)

- Flatpak runs apps with `bwrap --unshare-pid` ([`common/flatpak-run.c`](https://github.com/flatpak/flatpak/blob/main/common/flatpak-run.c)). A PID namespace hides host processes from the sandbox, but processes in a child PID namespace remain visible from the initial namespace under their host PIDs. `/proc/<pid>/status` lists the PID in each nested namespace in `NSpid`/`NStgid` ([proc_pid_status(5)](https://man7.org/linux/man-pages/man5/proc_pid_status.5.html)). A root daemon on the host therefore sees the reaper and the game normally. **High.**
- Inside the sandbox, `/.flatpak-info` identifies the app and instance ([flatpak-metadata(5)](https://github.com/flatpak/flatpak/blob/main/doc/flatpak-metadata.xml)). From the host, the daemon can read it as `/proc/<pid>/root/.flatpak-info`. Flatpak puts each instance in a systemd user scope named `app-flatpak-<app-id>-<instance-id>.scope` ([`flatpak-run.c`](https://github.com/flatpak/flatpak/blob/main/common/flatpak-run.c)). **High.**
- **Sub-sandboxes break ancestry.** Inside Flatpak, pressure-vessel "cannot create new containers directly"; instead it "asks the Flatpak service on the host system … to launch new containers on its behalf" ([slr-for-game-developers.md](https://gitlab.steamos.cloud/steamrt/steam-runtime-tools/-/blob/main/docs/slr-for-game-developers.md); [distro-assumptions.md](https://gitlab.steamos.cloud/steamrt/steam-runtime-tools/-/blob/main/docs/distro-assumptions.md)). The service, `org.freedesktop.portal.Flatpak.Spawn`, takes the new process's environment as an explicit `envs` argument ([D-Bus interface](https://github.com/flatpak/flatpak/blob/main/data/org.freedesktop.portal.Flatpak.xml)). It launches `flatpak run --sandbox …` via `g_spawn_async` from the user's `flatpak-portal` process ([`portal/flatpak-portal.c`](https://github.com/flatpak/flatpak/blob/main/portal/flatpak-portal.c)).
  - On the host, the actual game is therefore a descendant of `flatpak-portal`, **not of Steam's reaper**. **High.**
  - Native Linux games (scout-on-soldier and newer) and Proton ≥ 5.13 all run through pressure-vessel, so this is the normal case under Flatpak Steam. **Medium-high.**
  - pressure-vessel says Flatpak "will always try to put the game in a scope with a name like `app-flatpak-com.valvesoftware.Steam-12345.scope`, separate from the scope used to run Steam itself" ([wrap.1.md, `--systemd-scope`](https://gitlab.steamos.cloud/steamrt/steam-runtime-tools/-/blob/main/pressure-vessel/wrap.1.md)). **High.**
  - Consequence: walking up the process tree to the reaper (gamescope's fallback method) fails for the game itself under Flatpak Steam. The environment variables still identify the game, because they are forwarded through `envs`. Proton's per-game fixes, which depend on them, work under Flatpak Steam. **Medium-high.** We infer that the reaper inside Steam's sandbox stays alive while its child `pressure-vessel` waits for the spawned sub-sandbox to exit (the portal reports `SpawnExited`). **Medium**; not observed.

### 3.2 Snap Steam (`steam` snap)

- snapd's `snap-confine` creates mount namespaces and applies AppArmor and seccomp. Its source has no `CLONE_NEWPID` (the only matches in the snapd repository are in `snap-seccomp` and a test), so snap processes share the host PID namespace. **Medium.**
- AppArmor doesn't stop an unconfined root reader (§1.2). **High.**
- snapd tracks apps in transient systemd scopes. The naming scheme has changed over time ([snapd#7825](https://github.com/canonical/snapd/pull/7825), [snapd#16015](https://github.com/canonical/snapd/pull/16015)), and the scope identifies the Steam snap, not the game. **Medium.**
- The Steam client inside the snap is the same Valve client, so the reaper and environment variables should look the same. **Low**; no Snap Steam process tree was observed.

### 3.3 gamescope sessions

- Under gamescope, Steam still launches games through the reaper. gamescope itself depends on this: it identifies each window's game by walking to the reaper or reading the `app-steam-app<id>` scope ([`Process.cpp`](https://github.com/ValveSoftware/gamescope/blob/master/src/Utils/Process.cpp)). **High.**
- On SteamOS, Steam creates `app-steam-app<appid>-<reaperpid>.scope` per game, which gives a cgroup that carries the app ID and becomes empty when the game ends. **High.**
- Whether third-party desktop gamescope sessions (for example on Bazzite or a `gamescope-session` package) get those scopes is unverified. **Low.**
- If a user adds `gamescope -- %command%` to a game's launch options, gamescope and its Xwayland run inside the reaper with the game's environment, so they count as part of the running game. **Medium.**

## 4. Owning user

- `/proc/<pid>/status` `Uid:` gives the real, effective, saved, and filesystem UIDs ([proc_pid_status(5)](https://man7.org/linux/man-pages/man5/proc_pid_status.5.html)). The kernel maps them into the reader's user namespace. For a daemon in the initial user namespace, these are host UIDs even for processes inside Flatpak's or pressure-vessel's user namespaces, because Flatpak maps the user to the same UID inside and outside the sandbox. **High.**
- The proc connector's UID events are also reported in the initial user namespace (`from_kuid_munged(&init_user_ns, …)` in [`drivers/connector/cn_proc.c`](https://github.com/torvalds/linux/blob/master/drivers/connector/cn_proc.c)). **High.**
- The cgroup path (`user.slice/user-<uid>.slice/user@<uid>.service/app.slice/…`) gives the same answer for processes in user scopes, including Flatpak sub-sandboxes, which are started by the user's `flatpak-portal`. **High.**
- Multiple users can run Steam at the same time (for example with fast user switching). Each user's reaper and game carry that user's UID, so the UID distinguishes them. **High.**

## 5. When a running game ends

- **Reaper lifetime.** A subreaper adopts orphaned descendants. Valve's documentation says that background processes started by a game "will be reparented to have the subreaper as their parent" ([compat-tool interface](https://gitlab.steamos.cloud/steamrt/steam-runtime-tools/-/blob/main/docs/steam-compat-tool-interface.md)). Valve's equivalent documented tool waits "until all such descendant processes have exited" ([`steam-runtime-supervisor --subreaper`](https://gitlab.steamos.cloud/steamrt/steam-runtime-tools/-/blob/main/bin/supervisor.md)). The reaper therefore outlives helpers and children that outlive the main game process. Watching the reaper's exit, for example with a `pidfd` (Linux ≥ 5.3, see pidfd_open(2)), marks the end of the running game. **High** for the mechanism. That Steam's closed-source `reaper` waits for every descendant is **Medium**; it is the reason the reaper exists, and Valve recommended watching it in [#7876](https://github.com/ValveSoftware/steam-for-linux/issues/7876).
- **Lingering processes keep the game "running".** A helper that never exits keeps the reaper, and therefore the running game, alive. **Medium.**
- **Proton.** `wineserver` stays up for "the default value [of] 3 seconds" after the last Wine client exits ([wineserver(1)](https://gitlab.winehq.org/wine/wine/-/blob/master/server/wineserver.man.in)). The `waitforexitandrun` verb first waits for any previous wineserver to exit (`wineserver -w`), then runs the game through Proton's built-in `steam.exe` ([Proton `proton`](https://github.com/ValveSoftware/Proton/blob/proton_10.0/proton)). Expect the end of a Proton running game to be detected about 3 s after the game's window closes. **High.**
- **Flatpak sub-sandbox.** Game processes are outside the reaper's tree, so they are not reaped by it. Their dedicated `app-flatpak-com.valvesoftware.Steam-<n>.scope` cgroup reports `populated 0` in `cgroup.events` when empty; see the [cgroup v2 docs](https://docs.kernel.org/admin-guide/cgroup-v2.html). The file can be watched with poll or inotify. Alternatively, track every process that carries the game's `SteamAppId` until none remain. **Medium-high.**
- **Install scripts and multiple reapers.** A first launch can produce an install-script reaper (`Install=1`, no `SteamAppId`) that exits before the main reaper starts, with a gap between them ([#7876](https://github.com/ValveSoftware/steam-for-linux/issues/7876)). A detector keyed on the main reaper sees one start and one end per game. A detector keyed on `STEAM_COMPAT_APP_ID` alone could see two. **Medium.**
- **PID reuse.** Identify a process by PID plus start time (`/proc/<pid>/stat` field 22) or a pidfd, not by PID alone. **High.**

## 6. Can the game launch phase be separated?

| Launch step | Where it runs | Separable? | Confidence |
|---|---|---|---|
| "Processing Vulkan shaders" before launch (`fossilize_replay`) | Started by the Steam client before the game command ([#9432](https://github.com/ValveSoftware/steam-for-linux/issues/9432), [#13622](https://github.com/ValveSoftware/steam-for-linux/issues/13622)) | Yes: no reaper for the game exists yet | Medium (process tree not observed) |
| Install scripts (Steamworks redistributables, `installscript.vdf`) | Separate run of the compatibility tools; reaper with `Install=1`; `STEAM_COMPAT_APP_ID` set, `SteamAppId` not set ([compat-tool interface](https://gitlab.steamos.cloud/steamrt/steam-runtime-tools/-/blob/main/docs/steam-compat-tool-interface.md); [#7876](https://github.com/ValveSoftware/steam-for-linux/issues/7876)) | Yes, by `SteamAppId` or the `Install=1` argument | High (env) / Medium (argument) |
| Steam Linux Runtime container setup (pressure-vessel) | Inside the main reaper, with `SteamAppId` set | Not by app ID or ancestry | High |
| Proton prefix creation or upgrade (`setup_prefix`, `wineboot`), and waiting for a previous `wineserver` | Inside the `proton` script under the main reaper, before `steam.exe` launches the game executable ([`proton`](https://github.com/ValveSoftware/Proton/blob/proton_10.0/proton)) | Not by app ID. Only heuristically, for example by waiting for a Wine process other than `wineboot`, `wineserver`, `services.exe`, or `steam.exe` | Medium |
| In-game publisher launchers, anti-cheat bootstrap | Inside the reaper with `SteamAppId` | No (and #5 counts them as the running game anyway) | High |

As a result, a recognizer keyed on the main reaper or on `SteamAppId` starts counting the running game at the start of container and prefix setup. On a warm prefix that is usually a short interval; on first launch, or after a Proton upgrade, it can take tens of seconds. #5 accepts this ("may count if it can't be separated"). The duration estimate is **Low**; the placement of each step is **High**.

## 7. Detection mechanisms and latency

- **Polling `/proc`.**
  - A full scan reading `stat` and `cmdline` for all 347 processes on this host took a median of 3.8 ms (max 4.5 ms) from Python. Measured here on an i9-9880H; a Rust implementation should be cheaper.
  - Reading `environ` only for newly seen PIDs, or only under `comm == reaper`, keeps the cost proportional to new processes.
  - Detection latency is at most one poll interval, for example 1–2 s. Very short-lived processes can be missed, but the reaper lives for the whole running game, so that doesn't matter here.
  - **High.**
- **Proc connector (netlink `NETLINK_CONNECTOR`, `CN_IDX_PROC`).**
  - It delivers `PROC_EVENT_FORK`, `EXEC`, `UID`, `COMM`, `EXIT`, and other events almost as they happen ([`include/uapi/linux/cn_proc.h`](https://github.com/torvalds/linux/blob/master/include/uapi/linux/cn_proc.h)).
  - Newer kernels allow non-root processes to bind the proc group; otherwise `CAP_NET_ADMIN` is required ([`drivers/connector/connector.c`, `cn_bind`](https://github.com/torvalds/linux/blob/master/drivers/connector/connector.c)).
  - The listener must be in the initial user and PID namespaces ([`cn_proc.c`](https://github.com/torvalds/linux/blob/master/drivers/connector/cn_proc.c)). A root systemd service is, unless the unit adds namespacing.
  - Events are sent with `GFP_NOWAIT`, and "if cn_netlink_send() fails, the data is not sent". Events can therefore be lost, and a consumer needs a periodic `/proc` rescan as a backstop.
  - It requires `CONFIG_PROC_EVENTS`, which is enabled on this Ubuntu 26.04 kernel (`CONFIG_PROC_EVENTS=y`, `CONFIG_CONNECTOR=y`).
  - With exec events, a reaper is recognized within milliseconds of starting, which is at the start of the game launch.
  - **High.**
- **End of game.** A pidfd on the reaper, or `cgroup.events` for Flatpak's game scope or SteamOS's `app-steam-app…` scope, notifies on exit without polling. The exit-event path of the proc connector also works. **High.**
- **Current `prismd` unit.** `contrib/systemd/prismd.service` runs as root with no sandboxing directives, so `/proc` is fully visible. Adding `ProtectProc=invisible`, `PrivatePIDs=`, a `hidepid=` mount of `/proc`, or a network/user namespace would break `/proc` visibility or the proc connector. **High.**

## Open uncertainties

1. **No live Steam observation.** Nothing here was checked against a running Steam on this host. The Flatpak sub-sandbox ancestry, the `gameoverlayui` environment, and the `fossilize_replay` parentage should be checked on a machine with Steam before an implementation relies on them.
2. **Undocumented reaper argument format.** `reaper SteamLaunch AppId=<n>` and `Install=1` are not part of Valve's documented interface. Valve's own gamescope parses them, and they have been stable from 2021 to 2026. The documented signals are the environment variables.
3. **The 64-bit SteamRT3 client** (experimental opt-in as of August 2026) changes the launch chain: games are started by a launcher service "alongside Steam", outside the client's container. The reaper marker is still present. Its combination with Flatpak Steam, and the process ancestry when it becomes the default, are unknown.
4. **Snap Steam** is assessed from snapd source only. No Snap Steam process tree was observed.
5. **Desktop scopes:** Valve may extend the SteamOS `app-steam-app<id>-<pid>.scope` behaviour to desktop. If it does, cgroups become the simplest signal. gamescope's May 2026 commit says this is not yet the case.
6. **Launch-phase duration** for SLR container setup and Proton prefix setup was not measured.

## Sources

Valve / Steam Runtime: [steam-compat-tool-interface.md](https://gitlab.steamos.cloud/steamrt/steam-runtime-tools/-/blob/main/docs/steam-compat-tool-interface.md) · [slr-for-game-developers.md](https://gitlab.steamos.cloud/steamrt/steam-runtime-tools/-/blob/main/docs/slr-for-game-developers.md) · [distro-assumptions.md](https://gitlab.steamos.cloud/steamrt/steam-runtime-tools/-/blob/main/docs/distro-assumptions.md) · [pressure-vessel wrap.1.md](https://gitlab.steamos.cloud/steamrt/steam-runtime-tools/-/blob/main/pressure-vessel/wrap.1.md) · [pv-adverb adverb.1.md](https://gitlab.steamos.cloud/steamrt/steam-runtime-tools/-/blob/main/pressure-vessel/adverb.1.md) · [steam-runtime-supervisor](https://gitlab.steamos.cloud/steamrt/steam-runtime-tools/-/blob/main/bin/supervisor.md) · [Proton `proton`](https://github.com/ValveSoftware/Proton/blob/proton_10.0/proton) · [gamescope `Process.cpp`](https://github.com/ValveSoftware/gamescope/blob/master/src/Utils/Process.cpp) · [gamescope commit 565397f](https://github.com/ValveSoftware/gamescope/commit/565397f8cc)

steam-for-linux issues: [#7876](https://github.com/ValveSoftware/steam-for-linux/issues/7876) · [#9003](https://github.com/ValveSoftware/steam-for-linux/issues/9003) · [#9432](https://github.com/ValveSoftware/steam-for-linux/issues/9432) · [#9672](https://github.com/ValveSoftware/steam-for-linux/issues/9672) · [#9835](https://github.com/ValveSoftware/steam-for-linux/issues/9835) · [#10143](https://github.com/ValveSoftware/steam-for-linux/issues/10143) · [#13520](https://github.com/ValveSoftware/steam-for-linux/issues/13520) · [#13622](https://github.com/ValveSoftware/steam-for-linux/issues/13622) · [#13682](https://github.com/ValveSoftware/steam-for-linux/issues/13682)

Flatpak / Snap: [`flatpak-run.c`](https://github.com/flatpak/flatpak/blob/main/common/flatpak-run.c) · [`flatpak-portal.c`](https://github.com/flatpak/flatpak/blob/main/portal/flatpak-portal.c) · [org.freedesktop.portal.Flatpak](https://github.com/flatpak/flatpak/blob/main/data/org.freedesktop.portal.Flatpak.xml) · [flatpak-metadata(5)](https://github.com/flatpak/flatpak/blob/main/doc/flatpak-metadata.xml) · [Flatpak sandbox permissions](https://docs.flatpak.org/en/latest/sandbox-permissions.html) · [snapd#7825](https://github.com/canonical/snapd/pull/7825) · [snapd#16015](https://github.com/canonical/snapd/pull/16015)

Kernel / Wine: [proc_pid_environ(5)](https://man7.org/linux/man-pages/man5/proc_pid_environ.5.html) · [proc_pid_status(5)](https://man7.org/linux/man-pages/man5/proc_pid_status.5.html) · [`cn_proc.c`](https://github.com/torvalds/linux/blob/master/drivers/connector/cn_proc.c) · [`cn_proc.h`](https://github.com/torvalds/linux/blob/master/include/uapi/linux/cn_proc.h) · [`connector.c`](https://github.com/torvalds/linux/blob/master/drivers/connector/connector.c) · [Yama](https://github.com/torvalds/linux/blob/master/security/yama/yama_lsm.c) · [AppArmor `task.c`](https://github.com/torvalds/linux/blob/master/security/apparmor/task.c) · [cgroup v2](https://docs.kernel.org/admin-guide/cgroup-v2.html) · [wineserver(1)](https://gitlab.winehq.org/wine/wine/-/blob/master/server/wineserver.man.in)
