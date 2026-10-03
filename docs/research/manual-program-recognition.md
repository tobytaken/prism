# Recognizing manually identified programs

Research for [#25](https://github.com/tobytaken/prism-electron/issues/25), part of map [#2](https://github.com/tobytaken/prism-electron/issues/2). Background: the [resolution of #5](https://github.com/tobytaken/prism-electron/issues/5#issuecomment-5969896382) and ADR-0003 (daemon-driven game switching).

**Question.** Can the daemon recognize a manually identified program by its executable's location, natively and under Wine/Proton? What do `/proc/<pid>/exe`, `cmdline`, and related files show natively, under Wine, Proton (inside and outside Steam, e.g. umu, Heroic, Lutris), and with Flatpak-packaged launchers? Which location is stable enough to match, how does a browsed file map to it, what is the false-match risk, how should running programs be shown for picking, and what can't be recognized reliably?

This note reports facts and confidence levels. It does not decide product behaviour.

Sources were read on 2026-10-03. Wine is at `master` (latest tag wine-11.19), Proton at branch `proton_10.0`, umu-launcher, Lutris and Heroic at their default branches, and the Linux kernel at `master`. On this host I ran a read-only experiment with native processes only. No Wine, Steam or Flatpak is installed here, so the Wine and sandbox findings come from source code, not observation.

Confidence: **High** means stated by the owning documentation or source and/or observed here. **Medium** means read from source but not observed running. **Low** means inferred.

---

## 1. What `/proc` exposes (all processes)

| File | What it holds | Key caveats | Source |
|---|---|---|---|
| `/proc/<pid>/exe` | Symlink to "the actual pathname of the executed command". Dereferencing it opens the executable. | Appends ` (deleted)` if the file was unlinked. Needs a ptrace `PTRACE_MODE_READ_FSCREDS` check (root daemon: fine; ordinary user: other users' processes fail). | [proc_pid_exe(5)](https://man7.org/linux/man-pages/man5/proc_pid_exe.5.html) |
| `/proc/<pid>/cmdline` | argv strings as they currently sit in process memory. | "Think of this file as the command line that the process wants you to see." Processes can overwrite it. Empty for zombies and kernel threads. | [proc_pid_cmdline(5)](https://man7.org/linux/man-pages/man5/proc_pid_cmdline.5.html) |
| `/proc/<pid>/comm` | Command name. | Truncated to 15 bytes. Any thread can rewrite it; per-thread values live in `task/<tid>/comm`. | [proc_pid_comm(5)](https://man7.org/linux/man-pages/man5/proc_pid_comm.5.html) |
| `/proc/<pid>/maps` | Every mapped region with `dev`, `inode` and `pathname`. | ptrace read check. | [proc_pid_maps(5)](https://man7.org/linux/man-pages/man5/proc_pid_maps.5.html) |
| `/proc/<pid>/map_files/` | Symlinks to each mapped file. | Reading them needs `CAP_SYS_ADMIN` or `CAP_CHECKPOINT_RESTORE` (root daemon: fine). | [proc_pid_map_files(5)](https://man7.org/linux/man-pages/man5/proc_pid_map_files.5.html) |
| `/proc/<pid>/root/` | The process's own view of the filesystem, "including namespaces and the set of per-process mounts". | ptrace read check. | [proc_pid_root(5)](https://man7.org/linux/man-pages/man5/proc_pid_root.5.html) |
| `/proc/<pid>/mountinfo` | The mounts in the process's mount namespace, including each bind mount's source root. | Can be used to translate sandbox paths. | [proc_pid_mountinfo(5)](https://man7.org/linux/man-pages/man5/proc_pid_mountinfo.5.html) |
| `/proc/<pid>/environ` | Initial environment. | ptrace read check. | [proc_pid_environ(5)](https://man7.org/linux/man-pages/man5/proc_pid_environ.5.html) |

**The host daemon can see every process** (High). "A process is visible to other processes in its PID namespace, and to the processes in each direct ancestor PID namespace going back to the root PID namespace" ([pid_namespaces(7)](https://man7.org/linux/man-pages/man7/pid_namespaces.7.html)). Processes in Flatpak, bwrap or pressure-vessel sandboxes therefore appear in the host `/proc` with host PIDs. The reverse does not hold: a Flatpak build of the app cannot list host processes. ADR-0003 already relies on this.

**Paths of sandboxed processes are shown from the sandbox's point of view** (High, kernel source). The kernel builds `exe` and `maps` paths with `d_path()`. It walks up the mount tree of the file's own mount until it reaches that mount namespace's root, and does not stop at the reader's root ([fs/d_path.c `__prepend_path`](https://github.com/torvalds/linux/blob/master/fs/d_path.c)). So a process inside a Flatpak or bwrap sandbox shows paths such as `/app/bin/game` or `/home/u/.local/share/Steam/...` as that sandbox sees them, even though the host file sits somewhere else. Two things still resolve to the real file in every namespace: `stat()` through the magic link `/proc/<pid>/exe`, and the `dev`/`inode` columns of `maps`.

### Native observation on this host (High, observed)

I copied `/usr/bin/bash` into a temporary directory and ran it in several ways:

| Case | `exe` | `comm` | `cmdline` |
|---|---|---|---|
| Ran through a symlink `link-to-game` → `My-Very-Long-Game-Binary-Name` | `/tmp/…/My-Very-Long-Game-Binary-Name` (symlink resolved) | `link-to-game` (the invoked name) | `./link-to-game …` (relative, as typed) |
| Ran through a symlinked directory `alias/` → `real/` | `/tmp/…/real/game` (resolved) | | `./alias/game …` |
| Path containing a space, `with space/Game.exe` | `/tmp/…/with space/Game.exe` | `Game.exe` | |
| Shell script `script2.sh` | `/usr/bin/dash` (the interpreter) | `script2.sh` | `/bin/sh ./script2.sh` |
| Running binary renamed with `mv` | follows the rename: `/tmp/…/renamed` | | |
| Running binary deleted with `rm` | `/tmp/…/renamed (deleted)` | | |
| Non-root reading `/proc/1/exe` | permission denied (empty) | `systemd` readable | |

Takeaways:

- `exe` is canonical: absolute, with symlinks resolved. `cmdline` and `comm` reflect how the program was invoked.
- For any interpreted program, `exe` is the interpreter.
- A game updated while it runs shows ` (deleted)`.

---

## 2. Windows programs under Wine

### What each file shows (Medium-High, Wine source)

- **`exe` is a Wine binary, not the game.** `loader_exec()` re-executes through `"<wineloader>-preloader"` on x86 and arm ([dlls/ntdll/unix/loader.c `preloader_exec`](https://gitlab.winehq.org/wine/wine/-/blob/master/dlls/ntdll/unix/loader.c)). In current Wine, `wineloader` is `<ntdll_dir>/wine`, giving for example `…/lib/wine/x86_64-unix/wine-preloader`. Older builds, including Proton's `files/bin/`, use `wine-preloader` / `wine64-preloader` in `bin/`. Every Windows program in every prefix that uses the same Wine build therefore has the same `exe`. **Matching on `exe` alone would match all Wine programs.**
- **`cmdline` is rewritten.** The preloader shifts argv to drop its own name. Wine's `rebuild_argv()` / `prepend_argv()` then drop the loader name ([dlls/ntdll/unix/env.c](https://gitlab.winehq.org/wine/wine/-/blob/master/dlls/ntdll/unix/env.c); [loader/preloader.c `set_process_name`](https://gitlab.winehq.org/wine/wine/-/blob/master/loader/preloader.c)). What remains depends on how the process was started:
  - **Started from a Unix command** (`wine /path/game.exe`, `wine C:\\...`): argv[0] is exactly what the caller passed. That can be a Unix path, a Windows path, or a relative name.
  - **Started by another Windows process** (`CreateProcess`): Wine builds argv by splitting the **Windows command line** ([dlls/ntdll/unix/process.c `build_argv`](https://gitlab.winehq.org/wine/wine/-/blob/master/dlls/ntdll/unix/process.c)). argv[0] is whatever the parent wrote: often `C:\...\game.exe` or `Z:\home\...\game.exe` with backslashes, sometimes a bare file name or a quoted relative path. Launcher stubs, Proton's `steam.exe` and store clients all start games this way.
- **`comm` is the Windows file name, but it can change.** `set_process_name()` calls `prctl(PR_SET_NAME)` with the base name after stripping `\` and `/`, so `comm` reads e.g. `game.exe`, truncated to 15 bytes (`GameName-Win64-` for `GameName-Win64-Shipping.exe`). Wine also copies Windows thread descriptions (`SetThreadDescription`) into `/proc/<pid>/task/<tid>/comm` ([dlls/ntdll/unix/thread.c `set_native_thread_name`](https://gitlab.winehq.org/wine/wine/-/blob/master/dlls/ntdll/unix/thread.c)). If a game names its main thread, `/proc/<pid>/comm` changes to that name. *Inference, Medium:* `comm` is not a dependable label for Wine games.
- **`maps` names the real `.exe` file with its Unix path, `dev` and `inode`.** `map_image_into_view()` / `map_pe_header()` `mmap()` the PE header straight from the file descriptor of the image ([dlls/ntdll/unix/virtual.c](https://gitlab.winehq.org/wine/wine/-/blob/master/dlls/ntdll/unix/virtual.c)). They fall back to `pread()` into anonymous memory only for "removable" images, or when `mmap` fails with `EPERM`/`EACCES` (noexec filesystems) or `ENODEV`/`ENOEXEC` (filesystems without mmap). In the normal case the game's `.exe` therefore appears in `/proc/<pid>/maps` as a canonical Unix path (High: kernel `d_path`), alongside DLLs and Wine's builtin `.dll`/`.exe` files from the Wine install. *Not observed on a live system here (Medium).*
- **Helper processes in the same prefix.** Each prefix also runs `wineserver` (a native binary) and Windows-side system programs. Lutris's own process tracking lists `services.exe`, `winedevice.exe`, `plugplay.exe`, `explorer.exe`, `rpcss.exe`, `rundll32.exe` and `wineboot.exe` as "core processes" ([lutris/runners/wine.py](https://github.com/lutris/lutris/blob/master/lutris/runners/wine.py)). Typical others are `svchost.exe`, `conhost.exe` and `start.exe`, plus Proton's `steam.exe` below.

### Windows path → Unix path (Medium, Wine source)

Wine converts Unix paths to DOS paths by walking up the path until it finds the **deepest directory that is a drive root** in the prefix's `dosdevices/` ([dlls/ntdll/unix/file.c `find_drive_nt_root`](https://gitlab.winehq.org/wine/wine/-/blob/master/dlls/ntdll/unix/file.c)). The same game can therefore appear as `Z:\home\u\Games\X\x.exe` in one prefix and as `D:\X\x.exe` or `S:\…` in another. Turning a Windows path from `cmdline` back into a Unix path needs that prefix's `dosdevices` symlinks. The prefix comes from `WINEPREFIX` in `/proc/<pid>/environ`, or a default. The `maps` entry needs no such translation.

### Proton (in or outside Steam)

- Proton runs `<proton>/files/bin/wine64 c:\windows\system32\steam.exe <game args>` ([proton `Session.run`](https://github.com/ValveSoftware/Proton/blob/proton_10.0/proton)). The `files/bin-wow64/wine` loader is used if `PROTON_USE_WOW64=1` or there is no 32-bit `wine`. Proton's `steam.exe` turns an absolute Unix game path into a DOS path with `wine_get_dos_file_name` and starts the game with `CreateProcessW` (or `ShellExecuteW` for non-executables) ([steam_helper/steam.c `run_process`](https://github.com/ValveSoftware/Proton/blob/proton_10.0/steam_helper/steam.c)) (Medium-High). As a result:
  - There is always a `steam.exe` Wine process (its `comm` is `steam.exe`) alongside the game, even outside Steam. It is a helper, not the game.
  - The game's `cmdline` holds the DOS form, e.g. `Z:\…\game.exe`. When Proton's optional `gamedrive` / `steamdrive` compat options are on, it can be `S:\…` or `T:\…` ([proton `setup_dir_drive`](https://github.com/ValveSoftware/Proton/blob/proton_10.0/proton)).
- **umu-launcher** (used by Heroic, Lutris and others to run Proton outside Steam) does the following ([umu/umu_run.py](https://github.com/Open-Wine-Components/umu-launcher/blob/main/umu/umu_run.py)):
  - It makes `EXE` absolute with `Path.absolute()`, so symlinks are not resolved.
  - It sets `STEAM_COMPAT_INSTALL_PATH` to the exe's parent directory, and sets `GAMEID`/`UMU_ID` (Heroic always sets `GAMEID=umu-0`), `STORE` and `WINEPREFIX`.
  - It runs Proton's `waitforexitandrun` verb inside the Steam Linux Runtime container (pressure-vessel).
  
  The process chain is roughly `umu-run` (python) → runtime entry point → `pressure-vessel-wrap` → `bwrap` → `pv-adverb` → `proton` (python) → `wine64 steam.exe` → `game.exe`, plus `wineserver` and helpers (Medium).
- **pressure-vessel's filesystem view** ([pressure-vessel wrap(1)](https://gitlab.steamos.cloud/steamrt/steam-runtime-tools/-/blob/main/pressure-vessel/wrap.1.md)): home is shared by default. `STEAM_COMPAT_INSTALL_PATH` and `STEAM_COMPAT_LIBRARY_PATHS` "are made available read/write in the container". Host `/usr` and `/etc` appear as `/run/host/usr` etc. (High from the docs.) *Inference, Medium:* shared paths keep their host path inside the container (the docs describe sharing "similar to flatpak-run"). So a game `.exe` under home or the install path shows its **host** path in `maps`, while runtime libraries show container paths.

### Launchers

- **Heroic** sets `HEROIC_APP_NAME`, `HEROIC_APP_RUNNER`, `HEROIC_APP_SOURCE`, `STORE` and `GAMEID=umu-0`, plus `STEAM_COMPAT_INSTALL_PATH` for Proton. It uses umu when supported and runs the Wine binary directly otherwise ([src/backend/launcher.ts](https://github.com/Heroic-Games-Launcher/HeroicGamesLauncher/blob/main/src/backend/launcher.ts)) (Medium-High).
- **Lutris** tags game processes with `LUTRIS_GAME_UUID` in the environment. Its own game-process detection intersects processes that have that UUID with processes whose `cmdline` contains the game folder after replacing `\` with `/` ([lutris/game.py](https://github.com/lutris/lutris/blob/master/lutris/game.py), [lutris/util/process.py](https://github.com/lutris/lutris/blob/master/lutris/util/process.py), [lutris/runners/wine.py `filter_game_pids`](https://github.com/lutris/lutris/blob/master/lutris/runners/wine.py)) (High). This shows that an established launcher also relies on `cmdline` plus environment, not `exe`, to find Wine game processes.

---

## 3. Flatpak-packaged launchers and games

- **Paths inside a Flatpak are sandbox paths** (High: kernel `d_path` plus Flatpak docs). Directories granted with `--filesystem` keep their host path. `--persist=DIR` bind-mounts host `~/.var/app/$FLATPAK_ID/DIR` over `~/DIR`, and `--persist=.` covers the whole home ([Flatpak sandbox permissions](https://docs.flatpak.org/en/latest/sandbox-permissions.html), [flatpak-run(1)](https://github.com/flatpak/flatpak/blob/main/doc/flatpak-run.xml)). `/app`, `/usr` and `/tmp` are private.
- **Flathub manifests** (finish-args, High; consequences are Medium inference):
  - **Steam** ([com.valvesoftware.Steam](https://github.com/flathub/com.valvesoftware.Steam/blob/master/com.valvesoftware.Steam.yml)) has `--persist=.` and `/mnt`, `/media`, `/run/media`, but no `home`. A library inside the default data dir is shown as `/home/u/.local/share/Steam/…` while the host file is `/home/u/.var/app/com.valvesoftware.Steam/.local/share/Steam/…`. This matters for **non-Steam shortcuts run by Flatpak Steam**, which go through the manual fallback.
  - **Heroic** ([com.heroicgameslauncher.hgl](https://github.com/flathub/com.heroicgameslauncher.hgl/blob/master/com.heroicgameslauncher.hgl.yml)) has `--persist=.` and `~/Games/Heroic`, `/mnt`, `/media`, `/run/media`. Games in the default `~/Games/Heroic` keep host paths. Anything installed elsewhere in home that is not explicitly shared lands under `~/.var/app/com.heroicgameslauncher.hgl/…` on the host but shows as `~/…`.
  - **Lutris** ([net.lutris.Lutris](https://github.com/flathub/net.lutris.Lutris/blob/master/net.lutris.Lutris.yml)) has `--filesystem=home`, so games under home keep host paths.
- **A Flatpak-packaged game** has `exe` = `/app/…`. Its host file lives under a deploy directory named by the OSTree commit, which changes with every update (Medium). Its app ID can be read from `/.flatpak-info` inside the sandbox, through `/proc/<pid>/root/.flatpak-info` ([flatpak-metadata(5)](https://github.com/flatpak/flatpak/blob/main/doc/flatpak-metadata.xml)) (High).
- **Turning a sandbox path into a real file** (High, from the docs): open `/proc/<pid>/root/<sandbox path>` (root daemon), or translate it with `/proc/<pid>/mountinfo` (field 4 is a bind mount's root within its filesystem, field 5 its mount point). `stat()` on `/proc/<pid>/exe` or on a `map_files` entry gives the real `st_dev`/`st_ino` in any namespace.
- **The Flatpak Prism app's file chooser** (High, portal docs): Prism's Flatpak finish-args grant only `--filesystem=/run/prism` (`forge.config.ts`). A file the user browses for through the FileChooser portal is therefore made available "which may involve adding it to the Documents portal" ([FileChooser](https://github.com/flatpak/xdg-desktop-portal/blob/main/data/org.freedesktop.portal.FileChooser.xml)). It then arrives as `/run/user/$UID/doc/$DOC_ID/<name>`, **not the host path**. The host path can be recovered with `org.freedesktop.portal.Documents.GetHostPaths` (interface version 5+) or the `user.document-portal.host-path` xattr ([Documents](https://github.com/flatpak/xdg-desktop-portal/blob/main/data/org.freedesktop.portal.Documents.xml)).

---

## 4. Which location is stable to match, and how a browsed file maps to it

Findings (the choice of rule is left to product):

1. **Native ELF programs:** `readlink /proc/<pid>/exe` is a canonical absolute path (High). A browsed file maps to it once symlinks are resolved (`realpath`). The path stays stable across updates that replace the file in place; only the inode changes. A process still running from before an update shows ` (deleted)`.
2. **Windows programs (Wine, Proton, umu, Heroic, Lutris):** `exe` is useless for identity. The game's `.exe` is identified by its **mapped-file path in `/proc/<pid>/maps`**, which is canonical, Unix-form and symlink-resolved, or by the `dev`/`inode` on that line (Medium: source-derived, not observed). `cmdline` argv[0] is a secondary signal. It is a Windows or relative path that needs per-prefix drive translation and is controlled by the parent process. A browsed `.exe` maps to the `maps` path after `realpath`.
3. **Sandboxed processes:** string paths from `exe`/`maps` are sandbox-relative. Comparing **device and inode** finds the same file regardless of namespace or path string: `stat` on `/proc/<pid>/exe`, `map_files/<range>`, or the `maps` `dev`/`inode` columns, against `stat` of the browsed file on the host (High for the mechanism). Device and inode do not survive an update that replaces the file, so a location stored as a path has to be re-resolved to device and inode at match time.
4. **The same file reachable through two paths** (bind mounts, a second mount of the same disk): `exe` reports the path used at `exec` time, not a unique name (High, `d_path`). Symlinks are resolved, bind mounts are not. Device and inode comparison handles this case.
5. **A browsed file in the Flatpak app** arrives as a document-portal path and must be converted to a host path, via `GetHostPaths`, before the daemon can use it (High).

*Uncertainty (Low-Medium):* on btrfs (common on desktop distros), `stat().st_dev` reports a per-subvolume anonymous device, while `/proc/<pid>/maps` may report the superblock device. Comparing the `maps` `dev` column against `stat()` could then fail. `stat()` through `map_files/` or `/proc/<pid>/exe` avoids the question. Not verified here.

---

## 5. False-match risk

- **Shared runtimes (High):** matching on `exe` treats every program run by the same interpreter as the same program. This covers Wine and Proton (`wine-preloader`), shell scripts (`/usr/bin/bash`, `dash`), Java (`java`), Python, Mono, LÖVE, and emulators (one binary, many ROMs).
- **Sandbox path collisions (Medium):** a sandbox path string can name an unrelated host file. Example: Flatpak Steam's `/home/u/.local/share/Steam/steamapps/common/X/x.exe` versus a native Steam install at the same host path. Comparing path strings across namespaces can match the wrong file, or miss the right one. Device and inode do not have this problem.
- **Wine helpers and builtins (Medium):** a Wine process also maps Wine's builtin `.exe`/`.dll` files and possibly other `.exe` files loaded as data. A rule that matches "any mapped `.exe`" rather than "the main image" could fire on helpers. Picking the main image reliably may need the `cmdline` argv[0] base name, or the PEB `ImagePathName` read through `process_vm_readv`, to agree with the mapped file (Low: not explored).
- **Bootstrap or stub executables (Medium, general knowledge, not source-verified here):** many games start through a small launcher `.exe` that spawns the real binary and then exits or idles. Unreal Engine's root `Game.exe` spawns `Binaries/Win64/<Project>-Win64-Shipping.exe`; publisher launchers do similar things. A **browsed** file is often the stub, so the game is missed (a false negative). Picking from **running** programs gets the real process.
- **Same file across prefixes or users:** identical location means identical game. The owning user (`Uid:` in `/proc/<pid>/status`) separates users, as the #5 resolution requires.

---

## 6. Presenting running programs for picking

Facts that bear on a picker. All of them are available to the root daemon; the Flatpak app sees none of them.

- **Display name:**
  - Native: the base name of `exe` (High).
  - Wine: the base name of the mapped main `.exe`, or argv[0] from `cmdline` with `\` normalized (Medium).
  - Do not rely on `comm`: it is truncated to 15 bytes and Wine can rename it per thread.
- **Location:** the canonical path, translated to the host path for sandboxed processes. This is what the user can check against where they installed the game.
- **Launcher context:** the parent chain from the `PPid:` line of `/proc/<pid>/status` shows the launcher (`heroic`, `lutris`, `umu-run`, `steam`, `pressure-vessel`, `bwrap`) (High). Environment tags identify the launcher and title: `HEROIC_APP_NAME`, `LUTRIS_GAME_UUID`, `GAMEID`/`UMU_ID`, `STORE`, `SteamGameId`, `WINEPREFIX` (High for each launcher's source as cited).
- **Helpers to filter or demote** (Medium): `wineserver`; Wine system programs (`services.exe`, `winedevice.exe`, `plugplay.exe`, `explorer.exe`, `rpcss.exe`, `rundll32.exe`, `wineboot.exe`, `svchost.exe`, `conhost.exe`, `start.exe`); Proton's `steam.exe`; `pressure-vessel-*`, `pv-adverb`, `bwrap`, `reaper`, `steam-runtime-launcher-*`; python launchers (`proton`, `umu-run`, `legendary`, `gogdl`); kernel threads (empty `cmdline`); the launcher clients themselves.
- **"Uses the GPU" signal:**
  - NVML `nvmlDeviceGetGraphicsRunningProcesses_v3` lists processes with a graphics context on an NVIDIA GPU ([NVML Device Queries](https://docs.nvidia.com/deploy/nvml-api/615/api/group__nvmlDeviceQueries.html)) (High that it exists; how its PIDs behave for sandboxed processes is not verified).
  - DRM drivers can expose per-client `drm-*` keys in `/proc/<pid>/fdinfo` ([drm-usage-stats](https://github.com/torvalds/linux/blob/master/Documentation/gpu/drm-usage-stats.rst)). Coverage depends on the driver (High for the spec).
  - Either signal separates a game from most launchers and helpers.
- **Ownership:** the `Uid:` line of `/proc/<pid>/status` (host UID even for user-namespaced sandboxes, as seen from the host).

---

## 7. What cannot be recognized reliably by location

- **Interpreted or VM-hosted programs:** scripts, Java (e.g. Minecraft), Python, system Mono, LÖVE. `exe` is the shared interpreter, and the program file appears only in `cmdline` (or `maps` for some runtimes) (High).
- **Emulated games:** the emulator is recognizable but the ROM is not. At best the ROM appears in `cmdline` (High).
- **AppImages:** type-2 AppImages run from a per-run SquashFS mount (`APPDIR`, e.g. `/tmp/.mount_…`), so `exe` changes on every launch. The stable `.AppImage` path is only in the `APPIMAGE` environment variable ([AppImage env vars](https://docs.appimage.org/packaging-guide/environment-variables.html)) (High for the variable; mount naming Medium).
- **Flatpak-packaged games:** `exe` is `/app/…`. The host path contains the OSTree commit and changes on update. Such a game is identifiable by Flatpak app ID, not by a stable location (Medium).
- **Windows programs whose image is not mmapped:** images on removable media, noexec mounts, or filesystems that cannot mmap fall back to `pread`, so they are not in `maps`. Only `cmdline` remains (Medium, source).
- **Programs that unpack themselves to a fresh temporary directory on each run, or are started through a stub that exits** (Medium).
- **Programs inside other containers or VMs** (distrobox/podman with overlay roots, VMs): paths are container-relative and the files can be overlay-backed. Device and inode still work for containers on the same kernel; VMs are invisible (Low-Medium).

---

## Open uncertainties

1. Not observed live: what the Wine, Proton, umu and Flatpak cases look like in `/proc` (main `.exe` in `maps`, argv[0] form, `comm` renaming). Checking on a machine with Steam, Proton and Heroic would settle it.
2. Telling the main image apart from other mapped `.exe` files in a Wine process. The reliable method is not established.
3. Whether the `dev` column in `maps` matches `stat().st_dev` on btrfs subvolumes.
4. Whether pressure-vessel always exposes shared paths at their host path (the docs imply it).
5. How NVML reports PIDs for processes in PID namespaces (Flatpak, pressure-vessel).
6. Snap-packaged launchers or games, and gamescope sessions, were not examined.
