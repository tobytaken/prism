# NVIDIA live game recognition validation

The 9 October 2026 live pass for [issue #49](https://github.com/tobytaken/prism-electron/issues/49) **fails the release gate for automatic switching**. Both distribution-package Steam forms were exercised: native Oxygen Not Included and Balatro through Proton. Both games reached their main menus, but Prism reported recognition unavailable throughout. Profile management and startup behavior are not cleared or blocked by this result; their hardware behavior was outside this pass.

Tested revision: `57f4037ccc68815c852003c256ff880c2c79bfe8`. No production implementation was changed during validation. [Selected machine-readable evidence](2026-10-09-nvidia-game-recognition.json) preserves the observations below. Full local logs and screenshots are in the ignored `artifacts/issue-49/` directory.

## Host and execution

- CachyOS, kernel `7.2.9-2-cachyos`, KDE Plasma Wayland, btrfs root and home.
- NVIDIA GeForce RTX 5090, driver `615.78.08`; native Steam package `1.0.0.87-3`.
- Balatro Steam app `2379780`, using the installed `proton-cachyos-slr` tool and Steam Linux Runtime 4. Oxygen Not Included Steam app `457140`, verified native ELF executable. Hollow Knight was not launched because its installed build was Windows.
- Release Rust daemon and packaged Electron app built from the tested checkout. The app displayed the RTX 5090 and its 540 W power limit, the Games page, saved Balatro association, recognition failure, and running-program picker.
- The root daemon used `/run/prism-issue49/` for its socket and state. Its initial tuning store was a copy of the existing store, with startup behavior and per-user automatic switching disabled. Test associations and opt-in changes affected only this copy.
- Game launches and closure were approved by the user. The games were observed at their main menus; this was recognition validation, not a gameplay or stress benchmark.

## Required forms

| Form | Result | Evidence |
| --- | --- | --- |
| Distribution-package Steam, native | **Fail** | Oxygen Not Included exposed `SteamAppId=457140`; the picker recovered the real host executable. All 80 recognition samples over 39.77 seconds returned `unavailable`. |
| Distribution-package Steam, Proton | **Fail** | Balatro exposed `SteamAppId=2379780` through the pressure-vessel/Proton tree. All 80 recognition samples over 39.77 seconds returned `unavailable`. |
| Flatpak Steam, native | **Not tested** | Flatpak Steam was not installed. |
| Flatpak Steam, Proton | **Not tested** | Flatpak Steam was not installed. |

The native and Proton failures already block release under #49. The untested Flatpak forms must still pass after the failures are repaired.

## Checklist against the research issues

| Checklist item | Result and scope |
| --- | --- |
| Flatpak Steam sub-sandbox ancestry and `SteamAppId` | **Not tested.** Native Steam pressure-vessel observations do not establish Flatpak portal ancestry. See [#23](https://github.com/tobytaken/prism-electron/issues/23). |
| Overlay environment | **Pass for the observed native Steam overlay exclusion signal.** Opening Balatro's overlay with Shift+Tab created `gameoverlayui` as a child of the Steam client, with neither `SteamAppId` nor `STEAM_COMPAT_APP_ID`. Overall recognition remained blocked. See #23. |
| `fossilize_replay` parentage and launch duration | **Partial.** An already-running shader replay master was a direct child of Steam; its workers were children of that master. None carried either app-ID environment variable. This background replay concerned another app, not the approved launches. Launch-phase duration and the transition into a running game were not established. See #23. |
| Wine and Proton mapped-image matching | **Fail for the installed Proton on btrfs.** Balatro's PE image was mapped and named correctly, but the picker returned no host path. Plain Wine was not tested. See [#25](https://github.com/tobytaken/prism-electron/issues/25). |
| umu, Heroic and Lutris mapped-image matching | **Not tested.** No games were launched through these paths. Native Steam Proton evidence does not validate them. See #25. |
| pressure-vessel shared host paths | **Pass for Balatro's observed shared game path.** Its mapped path retained the host path under `~/.local/share/Steam/steamapps/common/Balatro/`; stat through the process root and `map_files` identified the same file as the host. This does not establish all sandbox paths. See #25. |
| btrfs device numbers in mapped images | **Fail for the implementation's equality assumption.** Maps reported `00:1e`, while host stat and stat through `map_files` both reported `00:35`, with inode `17251934` in all cases. See #25. |
| Document-portal host-path recovery | **Not tested.** This pass ran the native packaged Prism app, not a Flatpak Prism file chooser. See #25. |
| Snap Steam | **Not tested; unclaimed.** Snap was not available. See #23 and [#24](https://github.com/tobytaken/prism-electron/issues/24). |
| gamescope | **Not tested; unclaimed.** The executable was installed, but no approved game was launched through it. See #23 and #24. |
| Native Steam library listing | **Pass for the local library sample.** The daemon listed 59 installed entries, including both tested games, and excluded installed Steam Linux Runtime and Proton tools from the result. Existing Balatro links were preserved. This is not exhaustive validation of app-type filtering or alternate libraries. See #24. |
| Manual program caveat | **Pass.** The Games page and running-program dialog retain the “depends on how it runs” caveat. |

## Confirmed implementation failures

### The process scan aborts on a live kernel thread

`get_running_games` returned:

```json
{"status":"unavailable","reason":"A complete host process scan could not be read. Recognition will retry automatically."}
```

A root probe reproduced `ESRCH` reading `/proc/2/environ` while `/proc/2/stat` continued to identify the live `kthreadd` kernel thread. Three scans each encountered more than 350 environment-read failures, predominantly `ESRCH`. This is not merely a game exiting between samples.

In `rust/src/daemon/recognition.rs`, `scan()` reads every numeric process directory's environment with `?`, so this ordinary host condition aborts the whole scan. The failure was present while idle, during both games, and after closure. It prevents any game lifecycle or automatic application/restoration from being validated.

Repair must distinguish processes with no userspace memory and confirmed process exits from genuinely incomplete observations, retaining the existing protection against false game-end events and PID reuse. Merely converting every read error to “no game” would lose that protection.

### Mapped-image device identity is wrong for this btrfs host

Balatro's mapping was:

```text
140000000-140001000 r--p 00000000 00:1e 17251934 /home/toby/.local/share/Steam/steamapps/common/Balatro/Balatro.exe
```

The host file, `/proc/257589/map_files/140000000-140001000`, and the same path through `/proc/257589/root` all returned device `00:35`, inode `17251934`. The daemon identified the entry as `Balatro.exe`, with launcher Steam and the expected app ID, but returned `path: null` and a host-location recovery warning.

`rust/src/daemon/programs.rs` constructs mapped identities directly from the textual maps device/inode. Both host-location recovery and manual recognition compare that identity to stat-derived identities. This live counterexample invalidates that equality assumption. The overall manual recognition request was also blocked by the process-scan failure, so a separate end-to-end matching result is not claimed.

### Several launcher helpers are presented as ordinary programs

The Balatro picker response marked `services.exe`, `explorer.exe`, `winedevice.exe`, `rpcss.exe`, `srt-bwrap`, and `pv-adverb` as `helper: false`. `steam.exe` and `wineserver` were correctly marked as helpers. The missing Wine host paths remove directory context used by helper classification, while the observed wrapper basenames are not all covered. This contradicts the dialog's blanket “Helpers appear last” wording.

## Support claims audit

The manual recognition caveat is present. However, the README describes native/Flatpak/Snap library paths and Wine/Proton matching without a live-validation qualification, and the automatic-switching UI describes successful application and restoration without a release-gate notice. These statements must not be treated as proof of support. Snap and gamescope remain unclaimed by this pass; Flatpak, umu, Heroic, Lutris and document-portal behavior remain unvalidated. The README/UI acceptance criterion in #49 remains open alongside the implementation failures.

## Automated checks and packaging

- With installed Node `24.3.0` on PATH, `bun run check` passed lint, TypeScript, 244 renderer tests, 65 Electron tests, 338 Rust unit tests, 12 sidecar integration tests, and the three custom cooling-supervision checks.
- `bun run package` completed with Node 24 and produced `out/prism-electron-linux-x64/prism-electron`, which was launched successfully against the test daemon. The release daemon build passed. No DEB, RPM or Flatpak installation was performed.
- Under the host default Node `26.10.0`, the initial renderer run failed two startup-settings tests because `window.localStorage` was undefined, plus one fan-polling assertion that passed on a targeted retry. Packaging exited without an artifact at “Finalizing package”. The Node 24 run resolved these validation-environment failures; it did not resolve the live recognition failures.
- The task environment inherited `ELECTRON_RUN_AS_NODE=1`; launching Electron required unsetting it. The packaged app was run with the production sidecar and real daemon, without a fake backend.

## Hardware outcome and remaining validation

Temporarily enabling automatic switching in the isolated store returned `recognitionUnavailable`, with no per-GPU operation. It was disabled again. Before/after readings remained 540 W power limit, +240 MHz graphics offset, +450 MHz memory offset, and automatic cooling on both fans. No profile application, restoration, cooling recovery, startup/reboot behavior, or actual tuning write is certified by this pass.

Both games, the packaged app, and the test daemon were closed and their processes verified absent. The original `/var/lib/prism/tuning-state.json` retained its pre-test modification time. The isolated root-owned copy remains under `/run/prism-issue49/` until cleanup or reboot; no system service was installed or enabled.

After repairs, repeat idle/start/running/exit recognition on both native and Proton games, then validate actual automatic application and restoration with a conservative profile. Validate both required Flatpak Steam forms and the remaining checklist separately before expanding support claims. Keep #49 open and automatic-switching release blocked until those gates pass.
