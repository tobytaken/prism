# Research: listing installed Steam games

Ticket: [#24](https://github.com/tobytaken/prism-electron/issues/24), a child of the map [#2](https://github.com/tobytaken/prism-electron/issues/2). Background: [#5 resolution](https://github.com/tobytaken/prism-electron/issues/5#issuecomment-5969896382), `docs/adr/0003-daemon-driven-game-switching.md`.

Question: how can Prism list a user's installed Steam **games** for the game association picker, and where must that listing happen? This document reports facts and confidence only. It does not decide product behaviour.

Researched 2026-10-03. The research host has no Steam install, so nothing here was checked against live files.

## Confidence levels

- **High**: stated by the owner of the thing (Valve, Valve's Steam Runtime docs, the Flathub or Snap packaging source, Flatpak docs, this repo), or follows directly from that source code.
- **Medium**: no first-party spec. Based on file formats that many maintained third-party tools rely on, or on inference from first-party code.
- **Low**: a single third-party report, or not verified at all.

Valve does **not** publicly document `libraryfolders.vdf`, `appmanifest_*.acf`, `appinfo.vdf`, or the artwork cache. Anything about their contents is Medium at best.

## 1. Where Steam keeps its data, by install type

Steam is installed separately for each Linux user, in that user's home. Every install type uses the same layout under the **Steam root**: `steamapps/`, `config/`, `appcache/`, `userdata/`, and `compatibilitytools.d/`.

| Install type | Steam root on the host | Confidence | Source |
|---|---|---|---|
| Valve's `steam-launcher` package (Ubuntu/Arch/Fedora repos repackage it) | `$XDG_DATA_HOME/Steam`, which defaults to `~/.local/share/Steam`. Very old installs use `~/Steam`. `~/.steam/steam` and `~/.steam/root` are symlinks to the root. | High | Valve `bin_steam.sh` in `steam_1.0.0.87.tar.gz` (`DEFAULTSTEAMDIR="$STEAM_DATA_HOME/Steam"`, `CLASSICSTEAMDIR="$HOME/Steam"`, `ln -fns "$1" "$STEAMDATALINK"`) |
| Debian `steam-installer` package | New installs: `~/.steam/debian-installation`. Older ones: `~/.steam` itself, or whatever `~/.steam/steam` or `~/.steam/root` points to. | High | Debian [`debian/scripts/steam.in`](https://salsa.debian.org/games-team/steam-installer/-/blob/master/debian/scripts/steam.in) and `README.Debian`: "usually either ~/.steam/debian-installation or ~/.local/share/Steam" |
| Flathub `com.valvesoftware.Steam` | `~/.var/app/com.valvesoftware.Steam/.local/share/Steam`. Its `.steam` symlink directory is at `~/.var/app/com.valvesoftware.Steam/.steam`, not at the host's `~/.steam`. | High (current); Medium (older layouts) | [Flathub manifest](https://github.com/flathub/com.valvesoftware.Steam/blob/master/com.valvesoftware.Steam.yml) sets `--persist=.` and `FLATPAK_STEAM_XDG_DIRS_PREFIX=~/.var/app/com.valvesoftware.Steam`. `steam_wrapper.py` sets `XDG_DATA_HOME` to that prefix plus `.local/share`, then runs Valve's `bin_steam.sh`. Lutris also checks `~/.var/app/com.valvesoftware.Steam/data/Steam` for older Flatpak builds. |
| Snap `steam` (Canonical, strict confinement) | `~/snap/steam/common/.local/share/Steam`, with symlinks in `~/snap/steam/common/.steam/`. | High | [steam-snap `snapcraft.yaml`](https://github.com/canonical/steam-snap/blob/main/snap/snapcraft.yaml) sets `HOME: $SNAP_USER_COMMON` for the `steam` app. Snap docs (`docs/reference/external-libraries.md`, troubleshooting) use `~/snap/steam/common/.local/share/Steam`. |
| Steam Deck / SteamOS / gamescope sessions | Same as Valve's package: `~/.local/share/Steam`. A gamescope session doesn't change where files are stored. | Medium | Inferred: SteamOS ships Valve's client and launcher. Not checked on a device. |

The way to find a Steam root that Valve's own tools use is to resolve the `~/.steam/root` symlink. If that fails, they resolve `~/.steam/steam`, then `~/.steam/bin32/..`, then fall back to `~/.local/share/Steam` (steam-runtime-tools [`steam.c`](https://gitlab.steamos.cloud/steamrt/steam-runtime-tools/-/blob/main/steam-runtime-tools/steam.c), High). For Flatpak and Snap, `~/.steam` means the sandboxed home shown above, not the host's `~/.steam`. A single user can have several installs at once, such as native and Flatpak. Each has its own libraries and manifests, and the same app ID can be installed in more than one. (High, follows from the separate roots.)

### Library folders

- A user can add Steam library folders anywhere, such as other drives or `/mnt/...` ([Steam Support FAQ](https://help.steampowered.com/en/faqs/view/4bd4-4528-6b2e-8327)). High.
- The list of libraries is a text KeyValues/VDF file (format: [Valve Developer Wiki, KeyValues](https://developer.valvesoftware.com/wiki/KeyValues)). It is at `<root>/steamapps/libraryfolders.vdf`, and current clients also write a copy at `<root>/config/libraryfolders.vdf`. Lutris reads the `config/` copy and other tools read the `steamapps/` copy. Medium.
- Structure: top-level `"libraryfolders"`, then numeric keys `"0"`, `"1"`, and so on. Each holds `"path"` (an absolute path, no `steamapps` suffix), `"label"`, `"contentid"`, `"totalsize"`, and an `"apps"` block mapping app ID to size. Key `"0"` is normally the Steam root. Older clients wrote `"LibraryFolders"` with plain `"1" "/path"` values. Proton's `steam_helper/steam.c` (Valve) writes a minimal version, `"LibraryFolders" { "N" { "path" "..." } }`, inside the Wine prefix. That confirms the `path` key but not the other fields. Medium.
- Older configs also listed libraries as `BaseInstallFolder_N` in `config/config.vdf` (Lutris still reads them). Low to Medium.
- Older libraries may be spelled `SteamApps` instead of `steamapps`. Lutris does a case-insensitive path fix for this. Medium.
- Limits on where each sandboxed Steam can put libraries:
  - Flatpak Steam can only see `/mnt`, `/media`, and `/run/media`, plus whatever the user grants with `flatpak override`. The wiki tells users to add other drives that way (manifest finish-args and [wiki](https://github.com/flathub/com.valvesoftware.Steam/wiki)). High.
  - Snap Steam is "restricted to libraries located in `~`, `/media`, `/run/media`, or `/mnt`" (snap docs). High.

**Paths recorded inside sandboxed Steam (Medium, unverified):**
- **Flatpak.** For current Flatpak installs, the root path inside the sandbox is the same as on the host, because `~/.var/app/<id>` is mounted at its real path. Installs that began before the wrapper used the `~/.var/app/...` prefix may record their root as `/home/<user>/.local/share/Steam`. Inside the sandbox that path is the persisted `~/.var/app/com.valvesoftware.Steam/.local/share/Steam`. On the host it is the native install's path, or nothing. So the `"0"` path in a Flatpak install's `libraryfolders.vdf` can't safely be resolved on the host without translating it.
- **Snap.** Snap does not remap `/home`, so the paths it records should be valid on the host.

### Installed apps

- Each installed app has `<library>/steamapps/appmanifest_<appid>.acf`, a text KeyValues file with top-level `"AppState"`. Medium (no Valve spec; every maintained third-party tool depends on it).
- Useful keys:
  - `appid`
  - `name`: the display name
  - `installdir`: the folder name under `<library>/steamapps/common/`
  - `StateFlags`: a bitmask, where `4` means fully installed and other bits mark update, download, uninstall and similar states (reverse-engineered, Medium to Low)
  - `LastOwner`: the SteamID64 that installed it
  - `SizeOnDisk`, `buildid`, `LastUpdated`, `InstalledDepots`
  - `UserConfig` / `MountedConfig`: for example language and beta branch
- Treat the `.acf` files as the source of truth. The `"apps"` block in `libraryfolders.vdf` is a summary and may lag behind them (Medium).
- DLC has no `.acf` of its own; it shows up as depots inside the base game's manifest (Medium).
- Installed games are shared by every Steam account that uses that install. `LastOwner` only records who installed the app. Medium.

## 2. Excluding Proton, Steam Linux Runtime, and redistributables

Installed tools get ordinary `appmanifest_*.acf` files just like games, and nothing in the `.acf` says what type the app is (Medium). The options are:

1. **`toolmanifest.vdf` in the install directory.** Valve's Steam Runtime docs ([steam-compat-tool-interface.md](https://gitlab.steamos.cloud/steamrt/steam-runtime-tools/-/blob/main/docs/steam-compat-tool-interface.md)) say "Each compatibility tool has a tool manifest, `toolmanifest.vdf`". For Valve's built-in tools, the declaration "is downloaded out-of-band and stored in `appinfo.vdf`, so the depot downloaded into the Steam library will not contain [`compatibilitytool.vdf`]", but the depot does include `toolmanifest.vdf`. Proton's build installs `toolmanifest.vdf` into the depot root (`Makefile.in`, `toolmanifest_x86_64.vdf`).
   - Confidence: **High** for Proton (all versions, including Experimental and Hotfix), Steam Linux Runtime 1.0/2.0/3.0/4.0 (app IDs 1070560, 1391110, 1628350, 4183110, plus 4185400 for arm64), and the Legacy Steam Runtime (4690330) (IDs from steam-runtime-tools [`container-runtime.md`](https://gitlab.steamos.cloud/steamrt/steam-runtime-tools/-/blob/main/docs/container-runtime.md) and `legacy-steam-runtime.md`).
   - It does **not** catch Steamworks Common Redistributables (228980), and it probably misses the Proton EasyAntiCheat and BattlEye runtimes, which aren't compatibility tools (Low, unverified).
2. **Fixed list of app IDs to exclude.** Lutris excludes only `221410` (Steam for Linux), `228980` (Steamworks Common Redistributables) and `1070560` (Steam Linux Runtime 1.0) ([`lutris/services/steam.py`](https://github.com/lutris/lutris/blob/master/lutris/services/steam.py)). Valve keeps adding runtime app IDs: 4183110, 4185400 and 4690330 are all recent, and each Proton release gets a new ID. A fixed list therefore needs maintenance. High that it goes stale; app IDs themselves Medium unless cited above.
3. **App type in `<root>/appcache/appinfo.vdf`.** Steam caches each app's metadata here, including `common/type` (`Game`, `Application`, `Tool`, `Demo`, `DLC`, `Music`, `Config`, ...). Valve's compat-tool doc confirms that built-in tool metadata lives in this file. The file is binary and undocumented. Its format is described only by third parties ([SteamDatabase/SteamAppInfo](https://github.com/SteamDatabase/SteamAppInfo)): v40 (`0x07564428`, Dec 2022) added a hash, and v41 (`0x07564429`, June 2024) added a string table, breaking parsers both times. This is the most complete local filter. Medium for the content, High that the format changes without notice.
4. **Network.** Steam Web API `IStoreService/GetAppList` can filter games, DLC and software, but it needs a Web API key ([Steamworks docs](https://partner.steamgames.com/doc/webapi/IStoreService)). High. Not a local source.

Already left out without filtering (High):
- Third-party compatibility tools such as GE-Proton live in `compatibilitytools.d/` and have no `appmanifest` (Valve compat-tool doc).
- Non-Steam shortcuts live in `userdata/<id>/config/shortcuts.vdf`, not in library manifests (Medium).

**Not resolved by research:**
- Apps typed `Application`, such as Wallpaper Engine or Blender, and soundtracks (`Music`) also have manifests. Whether the picker shows them is a product question.
- SteamVR (250820) and similar `Tool`-typed apps also have manifests and need the same decision.

## 3. Names and artwork available locally

- **Name:** `name` in the `.acf` (Medium). `installdir` is a fallback. `appinfo.vdf` also holds names, including localized ones (Medium).
- **Artwork:** Steam caches library art in `<root>/appcache/librarycache/`. This is undocumented and has changed shape:
  - Older clients wrote flat files such as `<appid>_library_600x900.jpg`, `_header.jpg`, `_library_hero.jpg` and `_logo.png`.
  - Current clients use one directory per app, `librarycache/<appid>/`. It holds `library_600x900.jpg`, `header.jpg`, `library_hero.jpg` and `logo.png`, sometimes inside a 40-hex content-hash subdirectory, plus hash-named files (icons).
  - Sources: third-party reports from September 2026 ([sunshine-apps-ui #21](https://github.com/4o66/sunshine-apps-ui/issues/21), [OptiScaler-GUI #34](https://github.com/King4s/OptiScaler-GUI/pull/34)). Low to Medium.
  - Not every app has every asset. A file only exists after the client has fetched it.
- **Asset meanings and sizes** are documented by Valve ([Steamworks library assets](https://partner.steamgames.com/doc/store/assets/libraryassets)), High:
  - capsule: 600×900
  - header: 920×430
  - hero: 3840×1240
  - logo: 1280 wide and/or 720 tall
- **Without the local cache:** the same assets are on Steam's public CDN under the app ID. That needs network access and the URL scheme is undocumented (Low).

## 4. Can the Flatpak Prism app read these files?

**Not under its current permissions** (High). `forge.config.ts` gives the Flatpak build `--share=ipc`, `--socket=wayland`, `--socket=fallback-x11`, `--device=dri` and `--filesystem=/run/prism` only. By default a Flatpak app sees "no host files except the runtime, the app, `~/.var/app/$FLATPAK_ID`, and `$XDG_RUNTIME_DIR/app/$FLATPAK_ID`" ([Flatpak sandbox permissions](https://docs.flatpak.org/en/latest/sandbox-permissions.html)).

To read the files itself, the app would need read-only access to:
- `xdg-data/Steam` (Valve native)
- `~/.steam` (symlinks, and Debian's `debian-installation`)
- `~/.var/app/com.valvesoftware.Steam` (Flatpak Steam). `--filesystem=home` explicitly excludes `~/.var/app`, but naming another app's directory works: Flathub's Lutris ships `--filesystem=~/.var/app/com.valvesoftware.Steam:ro` and Heroic ships it read-write. High.
- `~/snap/steam/common` (Snap)
- every external library location: `/mnt`, `/media`, `/run/media`, or anything else the user chose

Library folders can be on any path, and symlinks only resolve if their targets are also exposed. So no fixed permission set covers every library (High, by construction). Read-only access is still possible on paths outside Flatpak's reserved list (`/usr`, `/etc`, `/tmp`, `/run/host`, and so on).

**The daemon can read them** (High for this repo's setup):
- `contrib/systemd/prismd.service` runs `prismd` as root with no `ProtectHome=` or other filesystem sandboxing, so it can read every user's home and mounted library drives.
- `rust/src/daemon.rs` already gets the caller's uid via `SO_PEERCRED`, which tells the daemon whose Steam installs to look at.
- ADR-0003 already makes the daemon responsible for recognizing running games for each user without the app.

Known limits for a root reader (Medium):
- home directories on NFS with `root_squash`
- FUSE mounts not mounted with `allow_other`
- locked `systemd-homed` homes
- any MAC policy a distribution might add

None of these apply to the repo's unit as written.

## Open uncertainties

- Whether the `.acf` `name` is localized or always the default name. Unverified.
- Whether legacy Flatpak Steam installs really record `/home/<user>/.local/share/Steam` as library `"0"`. Inferred from `steam_wrapper.py` and needs a real install to confirm.
- Whether Proton's EasyAntiCheat and BattlEye runtime depots, and 228980, lack `toolmanifest.vdf`. Unverified. If so, `toolmanifest.vdf` alone isn't a complete filter.
- Exact `appinfo.vdf` v41 field paths and `common/type` values for each tool or redistributable app. Only third-party sources exist.
- The exact current `librarycache` layout and how to pick an icon from the hash-named files. Only third-party sources exist.
- Snap Steam and gamescope sessions were not checked on real installs.
