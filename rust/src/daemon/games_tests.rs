//! Seam 1: the caller's installed Steam apps and game associations, through daemon session
//! requests, with Steam roots in a fake host root.

use serde_json::{json, Value};

use super::harness::{tunable_gpu, tunable_gpu_with_id, FakeGpuBackend, FakeSession, Harness};

const VALVE_ROOT: &str = "/home/alice/.local/share/Steam";
const DEBIAN_ROOT: &str = "/home/alice/.steam/debian-installation";
const FLATPAK_ROOT: &str = "/home/alice/.var/app/com.valvesoftware.Steam/.local/share/Steam";
const SNAP_ROOT: &str = "/home/alice/snap/steam/common/.local/share/Steam";

/// Records the library folders of a Steam root, as Steam writes them.
fn library_folders(daemon: &Harness, root: &str, libraries: &[&str]) {
    let folders: String = libraries
        .iter()
        .enumerate()
        .map(|(index, path)| {
            format!(
                "\t\"{index}\"\n\t{{\n\t\t\"path\"\t\t\"{path}\"\n\t\t\"label\"\t\t\"\"\n\t\t\"apps\"\n\t\t{{\n\t\t}}\n\t}}\n"
            )
        })
        .collect();
    daemon.write_host_file(
        &format!("{root}/steamapps/libraryfolders.vdf"),
        &format!("\"libraryfolders\"\n{{\n{folders}}}\n"),
    );
}

/// Writes an app manifest in the library, with its install folder.
fn app(daemon: &Harness, library: &str, app_id: u32, name: &str, state_flags: u32) {
    let install_dir = format!("{name} files");
    daemon.write_host_file(
        &format!("{library}/steamapps/appmanifest_{app_id}.acf"),
        &format!(
            "\"AppState\"\n{{\n\t\"appid\"\t\t\"{app_id}\"\n\t\"Universe\"\t\t\"1\"\n\t\"name\"\t\t\"{name}\"\n\t\"StateFlags\"\t\t\"{state_flags}\"\n\t\"installdir\"\t\t\"{install_dir}\"\n\t\"UserConfig\"\n\t{{\n\t\t\"language\"\t\t\"english\"\n\t}}\n}}\n"
        ),
    );
    daemon.write_host_file(
        &format!("{library}/steamapps/common/{install_dir}/readme.txt"),
        "",
    );
}

/// Writes a fully installed compatibility tool, whose install folder carries the marker.
fn compatibility_tool(daemon: &Harness, library: &str, app_id: u32, name: &str) {
    app(daemon, library, app_id, name, 4);
    daemon.write_host_file(
        &format!("{library}/steamapps/common/{name} files/toolmanifest.vdf"),
        "\"manifest\"\n{\n\t\"version\" \"2\"\n\t\"commandline\" \"/proton %verb%\"\n}\n",
    );
}

fn steam_apps(session: &FakeSession) -> Value {
    let response = session.send_json(json!({ "type": "list_steam_apps" }));
    assert_eq!(response["type"], "steam_apps", "{response}");
    response["payload"].clone()
}

fn games(session: &FakeSession) -> Value {
    let response = session.send_json(json!({ "type": "get_games" }));
    assert_eq!(response["type"], "games", "{response}");
    response["payload"].clone()
}

fn add_steam_game(session: &FakeSession, app_id: u32) -> Value {
    session.send_json(json!({ "type": "add_steam_game", "payload": { "appId": app_id } }))
}

fn link(session: &FakeSession, app_id: u32, gpu_id: &str, profile_id: Option<&str>) -> Value {
    session.send_json(json!({
        "type": "set_game_profile",
        "payload": {
            "game": { "kind": "steam", "appId": app_id },
            "gpuId": gpu_id,
            "profileId": profile_id
        }
    }))
}

fn import(session: &FakeSession, profiles: Value) {
    let response = session.send_json(json!({
        "type": "import_profiles", "payload": { "profiles": profiles }
    }));
    assert_eq!(response["type"], "profile_import", "{response}");
}

fn profile(id: &str, name: &str, gpu_id: &str) -> Value {
    json!({ "id": id, "name": name, "gpuId": gpu_id,
        "settings": { "powerLimitWatts": { "kind": "value", "value": 300 } } })
}

fn steam_game(app_id: u32, name: &str, installed: bool, profiles: Value) -> Value {
    json!({
        "game": { "kind": "steam", "appId": app_id },
        "name": name,
        "installed": installed,
        "profiles": profiles
    })
}

#[test]
fn installed_steam_apps_are_listed_from_every_library_of_the_callers_package_flatpak_and_snap_steam(
) {
    let daemon = Harness::start(FakeGpuBackend::default());
    library_folders(
        &daemon,
        VALVE_ROOT,
        &[VALVE_ROOT, "/mnt/games/SteamLibrary"],
    );
    app(&daemon, VALVE_ROOT, 570, "Dota 2", 4);
    app(&daemon, "/mnt/games/SteamLibrary", 620, "Portal 2", 4);
    // Being updated: installed, with an update pending.
    app(
        &daemon,
        "/mnt/games/SteamLibrary",
        1086940,
        "Baldur's Gate 3",
        6,
    );
    // Still downloading, so not fully installed.
    app(
        &daemon,
        "/mnt/games/SteamLibrary",
        730,
        "Counter-Strike 2",
        1026,
    );
    compatibility_tool(&daemon, VALVE_ROOT, 1493710, "Proton Experimental");
    compatibility_tool(
        &daemon,
        VALVE_ROOT,
        1628350,
        "Steam Linux Runtime 3.0 (sniper)",
    );
    app(
        &daemon,
        VALVE_ROOT,
        228980,
        "Steamworks Common Redistributables",
        4,
    );
    // The Debian layout, whose library list can also live under config/.
    app(&daemon, DEBIAN_ROOT, 440, "Team Fortress 2", 4);
    let debian_list = format!("{DEBIAN_ROOT}/steamapps/libraryfolders.vdf");
    library_folders(&daemon, DEBIAN_ROOT, &[DEBIAN_ROOT, "/srv/steam"]);
    let config_list = daemon.host_path(&format!("{DEBIAN_ROOT}/config/libraryfolders.vdf"));
    std::fs::create_dir_all(config_list.parent().unwrap()).unwrap();
    std::fs::rename(daemon.host_path(&debian_list), config_list).unwrap();
    app(&daemon, "/srv/steam", 292030, "The Witcher 3", 4);
    // Flatpak Steam records paths as it sees them inside its sandbox, whose home is its
    // persisted data directory.
    library_folders(
        &daemon,
        FLATPAK_ROOT,
        &[
            "/home/alice/.local/share/Steam",
            "/home/alice/Games/SteamLibrary",
            "/run/media/alice/External/SteamLibrary",
        ],
    );
    app(&daemon, FLATPAK_ROOT, 570, "Dota 2", 4);
    app(
        &daemon,
        "/home/alice/.var/app/com.valvesoftware.Steam/Games/SteamLibrary",
        367520,
        "Hollow Knight",
        4,
    );
    app(
        &daemon,
        "/run/media/alice/External/SteamLibrary",
        1145360,
        "Hades",
        4,
    );
    app(&daemon, SNAP_ROOT, 413150, "Stardew Valley", 4);
    app(
        &daemon,
        "/home/bob/.local/share/Steam",
        105600,
        "Terraria",
        4,
    );

    assert_eq!(
        steam_apps(&daemon.user(1000, "alice")),
        json!([
            { "appId": 1086940, "name": "Baldur's Gate 3" },
            { "appId": 570, "name": "Dota 2" },
            { "appId": 1145360, "name": "Hades" },
            { "appId": 367520, "name": "Hollow Knight" },
            { "appId": 620, "name": "Portal 2" },
            { "appId": 413150, "name": "Stardew Valley" },
            { "appId": 440, "name": "Team Fortress 2" },
            { "appId": 292030, "name": "The Witcher 3" }
        ])
    );
    assert_eq!(
        steam_apps(&daemon.user(1001, "bob")),
        json!([{ "appId": 105600, "name": "Terraria" }])
    );
    assert_eq!(steam_apps(&daemon.user(1002, "carol")), json!([]));
}

#[test]
fn associations_and_the_switching_opt_in_belong_to_the_caller_and_survive_a_restart() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![
        tunable_gpu(),
        tunable_gpu_with_id("GPU-456"),
    ]));
    app(&daemon, VALVE_ROOT, 570, "Dota 2", 4);
    app(&daemon, VALVE_ROOT, 620, "Portal 2", 4);
    let alice = daemon.user(1000, "alice");
    import(
        &alice,
        json!([
            profile("quiet", "Quiet", "GPU-123"),
            profile("loud", "Loud", "GPU-456")
        ]),
    );
    assert_eq!(
        games(&alice),
        json!({ "automaticSwitching": false, "games": [] })
    );

    let added = add_steam_game(&alice, 570);
    assert_eq!(added["type"], "games", "{added}");
    assert_eq!(
        added["payload"]["games"],
        json!([steam_game(570, "Dota 2", true, json!({}))])
    );
    add_steam_game(&alice, 620);
    // Associations can be configured while automatic switching is off.
    link(&alice, 570, "GPU-123", Some("quiet"));
    link(&alice, 570, "GPU-456", Some("loud"));
    // Many games can link to one profile.
    link(&alice, 620, "GPU-123", Some("quiet"));
    link(&alice, 620, "GPU-456", Some("loud"));
    // Leave unchanged clears the GPU's link.
    let left = link(&alice, 620, "GPU-456", None);
    assert_eq!(
        left["payload"],
        json!({
            "automaticSwitching": false,
            "games": [
                steam_game(570, "Dota 2", true, json!({ "GPU-123": "quiet", "GPU-456": "loud" })),
                steam_game(620, "Portal 2", true, json!({ "GPU-123": "quiet" }))
            ]
        })
    );
    let enabled = alice.send_json(json!({
        "type": "set_automatic_switching", "payload": { "enabled": true }
    }));
    assert_eq!(enabled["payload"]["automaticSwitching"], true, "{enabled}");
    let removed = alice.send_json(json!({
        "type": "remove_game", "payload": { "game": { "kind": "steam", "appId": 570 } }
    }));
    assert_eq!(
        removed["payload"],
        json!({
            "automaticSwitching": true,
            "games": [steam_game(620, "Portal 2", true, json!({ "GPU-123": "quiet" }))]
        })
    );

    // Another user, and a reused uid under another username, see none of it.
    let empty = json!({ "automaticSwitching": false, "games": [] });
    assert_eq!(games(&daemon.user(1001, "bob")), empty);
    assert_eq!(games(&daemon.user(1000, "carol")), empty);
    let theirs = daemon.user(1001, "bob");
    app(&daemon, "/home/bob/.local/share/Steam", 620, "Portal 2", 4);
    add_steam_game(&theirs, 620);
    let foreign = link(&theirs, 620, "GPU-123", Some("quiet"));
    assert_eq!(foreign["payload"]["code"], "notFound", "{foreign}");

    daemon.restart();
    let alice = daemon.user(1000, "alice");
    assert_eq!(games(&alice), removed["payload"]);
    assert_eq!(
        games(&daemon.user(1001, "bob")),
        json!({
            "automaticSwitching": false,
            "games": [steam_game(620, "Portal 2", true, json!({}))]
        })
    );
}

#[test]
fn links_are_checked_against_the_callers_profiles_games_and_installed_apps() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    app(&daemon, VALVE_ROOT, 570, "Dota 2", 4);
    compatibility_tool(&daemon, VALVE_ROOT, 1493710, "Proton Experimental");
    let alice = daemon.user(1000, "alice");
    import(
        &alice,
        json!([
            profile("quiet", "Quiet", "GPU-123"),
            profile("other", "Other", "GPU-456")
        ]),
    );
    for (response, code, message) in [
        (
            add_steam_game(&alice, 620),
            "notFound",
            "Steam app 620 isn't installed in your Steam libraries.",
        ),
        (
            add_steam_game(&alice, 1493710),
            "notFound",
            "Steam app 1493710 isn't installed in your Steam libraries.",
        ),
        (
            link(&alice, 570, "GPU-123", Some("quiet")),
            "notFound",
            "This game is no longer on your Games page.",
        ),
    ] {
        assert_eq!(response["payload"]["code"], code, "{response}");
        assert_eq!(response["payload"]["message"], message, "{response}");
    }
    add_steam_game(&alice, 570);
    let wrong_gpu = link(&alice, 570, "GPU-123", Some("other"));
    assert_eq!(
        wrong_gpu["payload"]["code"], "invalidArgument",
        "{wrong_gpu}"
    );
    assert_eq!(
        wrong_gpu["payload"]["message"],
        "The profile \"Other\" belongs to another GPU."
    );
    let missing = link(&alice, 570, "GPU-123", Some("deleted"));
    assert_eq!(
        missing["payload"]["message"],
        "The GPU tuning profile no longer exists. No GPU settings were changed."
    );
    // Adding a game again keeps its links.
    link(&alice, 570, "GPU-123", Some("quiet"));
    assert_eq!(
        add_steam_game(&alice, 570)["payload"]["games"],
        json!([steam_game(
            570,
            "Dota 2",
            true,
            json!({ "GPU-123": "quiet" })
        )])
    );
}

#[test]
fn an_uninstalled_games_association_is_kept_and_reported_not_installed_until_it_returns() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    library_folders(
        &daemon,
        VALVE_ROOT,
        &[VALVE_ROOT, "/mnt/games/SteamLibrary"],
    );
    app(&daemon, "/mnt/games/SteamLibrary", 570, "Dota 2", 4);
    let alice = daemon.user(1000, "alice");
    import(&alice, json!([profile("quiet", "Quiet", "GPU-123")]));
    add_steam_game(&alice, 570);
    link(&alice, 570, "GPU-123", Some("quiet"));

    std::fs::remove_file(daemon.host_path("/mnt/games/SteamLibrary/steamapps/appmanifest_570.acf"))
        .unwrap();
    let uninstalled = json!({
        "automaticSwitching": false,
        "games": [steam_game(570, "Dota 2", false, json!({ "GPU-123": "quiet" }))]
    });
    assert_eq!(games(&alice), uninstalled);
    daemon.restart();
    let alice = daemon.user(1000, "alice");
    assert_eq!(games(&alice), uninstalled);
    // The association can still be changed and removed while the game is gone.
    assert_eq!(
        link(&alice, 570, "GPU-123", None)["payload"]["games"][0]["profiles"],
        json!({})
    );

    app(&daemon, "/mnt/games/SteamLibrary", 570, "Dota 2", 4);
    assert_eq!(games(&alice)["games"][0]["installed"], true);
}

#[test]
fn deleting_a_profile_removes_its_associations_and_keeps_other_links() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![
        tunable_gpu(),
        tunable_gpu_with_id("GPU-456"),
    ]));
    app(&daemon, VALVE_ROOT, 570, "Dota 2", 4);
    app(&daemon, VALVE_ROOT, 620, "Portal 2", 4);
    let alice = daemon.user(1000, "alice");
    import(
        &alice,
        json!([
            profile("quiet", "Quiet", "GPU-123"),
            profile("loud", "Loud", "GPU-456")
        ]),
    );
    for app_id in [570, 620] {
        add_steam_game(&alice, app_id);
        link(&alice, app_id, "GPU-123", Some("quiet"));
    }
    link(&alice, 570, "GPU-456", Some("loud"));

    let deleted =
        alice.send_json(json!({ "type": "delete_profile", "payload": { "profileId": "quiet" } }));
    assert_eq!(deleted["payload"]["success"], true, "{deleted}");
    assert_eq!(
        games(&alice)["games"],
        json!([
            steam_game(570, "Dota 2", true, json!({ "GPU-456": "loud" })),
            steam_game(620, "Portal 2", true, json!({}))
        ])
    );
}
