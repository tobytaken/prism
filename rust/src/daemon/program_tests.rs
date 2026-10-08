//! Seam 1: manual programs through caller sessions, fixture processes and fake GPUs.
use super::harness::{tunable_gpu_with_id, FakeGpuBackend, FakeSession, Harness};
use serde_json::{json, Value};
use std::os::unix::fs::{symlink, MetadataExt};

fn ok(session: &FakeSession, request: Value) -> Value {
    let response = session.send_json(request);
    assert_ne!(response["type"], "error", "{response}");
    response["payload"].clone()
}

fn native(daemon: &Harness, pid: u32, uid: u32, program: &str) {
    daemon.run_process(pid, uid, 420, "", &format!("{program}\0"));
    symlink(
        daemon.host_path(program),
        daemon.host_path(&format!("/proc/{pid}/exe")),
    )
    .unwrap();
    daemon.write_host_file(&format!("/proc/{pid}/maps"), "");
}

#[test]
fn manual_native_program_switches_by_file_identity_and_re_resolves_updates() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu_with_id(
        "GPU-A",
    )]));
    let alice = daemon.user(1000, "alice");
    daemon.write_host_file("/games/game", "first version");
    let game = json!({"kind":"program", "path":"/games/game"});
    ok(
        &alice,
        json!({"type":"add_program", "payload":{"path":"/games/game"}}),
    );
    ok(
        &alice,
        json!({"type":"import_profiles", "payload":{"profiles":[{
            "id":"gaming", "name":"Gaming", "gpuId":"GPU-A",
            "settings":{"powerLimitWatts":{"kind":"value", "value":350}}
        }]}}),
    );
    ok(
        &alice,
        json!({"type":"set_game_profile", "payload":{"game":game, "gpuId":"GPU-A", "profileId":"gaming"}}),
    );
    ok(
        &alice,
        json!({"type":"set_automatic_switching", "payload":{"enabled":true}}),
    );
    // The executable link's path differs from the stored path, but the inode is identical.
    std::fs::hard_link(
        daemon.host_path("/games/game"),
        daemon.host_path("/games/alias"),
    )
    .unwrap();
    native(&daemon, 123, 1000, "/games/alias");
    daemon.scan();
    assert_eq!(
        daemon.gpus.gpu("GPU-A").power.current_limit_watts,
        Some(350)
    );
    assert_eq!(
        ok(&alice, json!({"type":"get_running_games"}))["games"][0]["game"],
        game
    );
    daemon.end_process(123);
    daemon.scan();
    // Replacing the file changes its inode; the association follows the location.
    daemon.write_host_file("/games/update", "new version");
    std::fs::rename(
        daemon.host_path("/games/update"),
        daemon.host_path("/games/game"),
    )
    .unwrap();
    native(&daemon, 124, 1000, "/games/game");
    daemon.scan();
    assert_eq!(
        daemon.gpus.gpu("GPU-A").power.current_limit_watts,
        Some(350)
    );
    std::fs::rename(
        daemon.host_path("/games/game"),
        daemon.host_path("/games/moved"),
    )
    .unwrap();
    let state = ok(&alice, json!({"type":"get_games"}));
    assert_eq!(state["games"][0]["installed"], false);
    assert_eq!(state["games"][0]["profiles"]["GPU-A"], "gaming");
}

fn map_image(daemon: &Harness, pid: u32, file: &str, view: &str) {
    let metadata = std::fs::metadata(daemon.host_path(file)).unwrap();
    daemon.write_host_file(
        &format!("/proc/{pid}/maps"),
        &format!(
            "00400000-00500000 r-xp 00000000 {:02x}:{:02x} {} {view}\n",
            libc::major(metadata.dev()),
            libc::minor(metadata.dev()),
            metadata.ino()
        ),
    );
}

#[test]
fn wine_mapped_image_switches_without_matching_the_shared_preloader_or_another_users_process() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu_with_id(
        "GPU-A",
    )]));
    let alice = daemon.user(1000, "alice");
    for file in [
        "/games/game.exe",
        "/games/other.exe",
        "/usr/bin/wine-preloader",
    ] {
        daemon.write_host_file(file, "binary");
    }
    ok(
        &alice,
        json!({"type":"add_program", "payload":{"path":"/games/game.exe"}}),
    );
    ok(
        &alice,
        json!({"type":"import_profiles", "payload":{"profiles":[{
            "id":"gaming", "name":"Gaming", "gpuId":"GPU-A",
            "settings":{"powerLimitWatts":{"kind":"value", "value":350}}
        }]}}),
    );
    ok(
        &alice,
        json!({"type":"set_game_profile", "payload":{"game":{"kind":"program","path":"/games/game.exe"}, "gpuId":"GPU-A", "profileId":"gaming"}}),
    );
    ok(
        &alice,
        json!({"type":"set_automatic_switching", "payload":{"enabled":true}}),
    );
    native(&daemon, 123, 1000, "/usr/bin/wine-preloader");
    // Misleading sandbox path: identity, not the map's pathname, decides the match.
    map_image(&daemon, 123, "/games/other.exe", "/games/game.exe");
    native(&daemon, 124, 1001, "/usr/bin/wine-preloader");
    map_image(&daemon, 124, "/games/game.exe", "/sandbox/game.exe");
    daemon.scan();
    assert_eq!(
        ok(&alice, json!({"type":"get_running_games"}))["games"],
        json!([])
    );
    map_image(&daemon, 123, "/games/game.exe", "/sandbox/game.exe");
    daemon.scan();
    assert_eq!(
        daemon.gpus.gpu("GPU-A").power.current_limit_watts,
        Some(350)
    );
    assert_eq!(
        ok(&alice, json!({"type":"get_running_games"}))["games"][0]["pid"],
        123
    );
    // A failed read must not pretend the running game ended.
    std::fs::remove_file(daemon.host_path("/proc/123/maps")).unwrap();
    daemon.scan();
    assert_eq!(
        ok(&alice, json!({"type":"get_running_games"}))["status"],
        "unavailable"
    );
    assert_eq!(
        daemon.gpus.gpu("GPU-A").power.current_limit_watts,
        Some(350)
    );
}

#[test]
fn running_program_picker_scopes_ownership_labels_wine_and_ancestry_and_demotes_helpers() {
    let daemon = Harness::start(FakeGpuBackend::default());
    let alice = daemon.user(1000, "alice");
    let paths = [
        "/usr/bin/lutris",
        "/usr/bin/wine-preloader",
        "/games/My Game.exe",
        "/usr/bin/wineserver",
        "/usr/bin/python3",
        "/games/Game.AppImage",
        "/usr/bin/qemu-system-x86_64",
        "/usr/bin/retroarch",
        "/app/bin/game",
        "/wine/drive_c/windows/system32/services.exe",
        "/proton/steam.exe",
        "/usr/bin/pressure-vessel-wrap",
        "/usr/bin/bwrap",
        "/steam/reaper",
    ];
    for (index, path) in paths.iter().enumerate() {
        daemon.write_host_file(path, "binary");
        native(&daemon, 10 + index as u32, 1000, path);
    }
    native(&daemon, 99, 1001, "/games/My Game.exe");
    // Wine child -> intermediate preloader -> Lutris, independent of argv's claimed launcher.
    daemon.run_process(
        11,
        1000,
        420,
        "WINEPREFIX=/home/alice/prefix\0SECRET_TOKEN=private\0",
        "wine\0Game.exe\0",
    );
    let stat = std::fs::read_to_string(daemon.host_path("/proc/11/stat")).unwrap();
    daemon.write_host_file("/proc/11/stat", &stat.replace(") S 1 ", ") S 10 "));
    map_image(&daemon, 11, "/games/My Game.exe", "/games/My Game.exe");
    daemon.write_host_file("/proc/18/environ", "FLATPAK_ID=org.example.Game\0");
    let listed = ok(&alice, json!({"type":"list_running_programs"}));
    let entries = listed.as_array().unwrap();
    assert_eq!(entries.len(), paths.len());
    assert!(entries.iter().all(|entry| entry["pid"] != 99));
    let wine = entries.iter().find(|entry| entry["pid"] == 11).unwrap();
    assert_eq!(wine["name"], "My Game.exe");
    assert_eq!(wine["path"], "/games/My Game.exe");
    assert_eq!(wine["launcher"], "Lutris");
    assert_eq!(
        wine["environment"],
        json!(["WINEPREFIX=/home/alice/prefix"])
    );
    for pid in [13, 14, 19, 20, 21, 22, 23] {
        assert_eq!(
            entries.iter().find(|entry| entry["pid"] == pid).unwrap()["helper"],
            true,
            "pid {pid}"
        );
    }
    for pid in [14, 15, 16, 17, 18] {
        assert!(
            entries.iter().find(|entry| entry["pid"] == pid).unwrap()["recognitionWarning"]
                .is_string(),
            "pid {pid}"
        );
    }
    let first_helper = entries
        .iter()
        .position(|entry| entry["helper"] == true)
        .unwrap();
    assert!(entries[first_helper..]
        .iter()
        .all(|entry| entry["helper"] == true));
}

#[test]
fn picker_recovers_bind_mounted_host_locations_and_finds_a_python_launcher_in_the_parent_chain() {
    let daemon = Harness::start(FakeGpuBackend::default());
    let alice = daemon.user(1000, "alice");
    daemon.write_host_file("/usr/bin/python3", "runtime");
    daemon.write_host_file("/usr/bin/wine-preloader", "runtime");
    daemon.write_host_file("/home/alice/Games/My  Game.exe", "binary");
    native(&daemon, 10, 1000, "/usr/bin/python3");
    daemon.write_host_file("/proc/10/cmdline", "/usr/bin/python3\0/usr/bin/lutris\0");
    native(&daemon, 11, 1000, "/usr/bin/wine-preloader");
    let stat = std::fs::read_to_string(daemon.host_path("/proc/11/stat")).unwrap();
    daemon.write_host_file("/proc/11/stat", &stat.replace(") S 1 ", ") S 10 "));
    map_image(
        &daemon,
        11,
        "/home/alice/Games/My  Game.exe",
        "/sandbox/My  Game.exe",
    );
    let dev = std::fs::metadata(daemon.host_path("/home/alice/Games/My  Game.exe"))
        .unwrap()
        .dev();
    let device = format!("{}:{}", libc::major(dev), libc::minor(dev));
    daemon.write_host_file(
        "/proc/11/mountinfo",
        &format!("30 1 {device} /home/alice/Games /sandbox rw - ext4 /dev/test rw\n"),
    );
    daemon.write_host_file(
        "/proc/self/mountinfo",
        &format!("1 0 {device} / / rw - ext4 /dev/test rw\n"),
    );
    let listed = ok(&alice, json!({"type":"list_running_programs"}));
    let program = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["pid"] == 11)
        .unwrap();
    assert_eq!(program["name"], "My  Game.exe");
    assert_eq!(program["path"], "/home/alice/Games/My  Game.exe");
    assert_eq!(program["launcher"], "Lutris");
}

#[test]
fn manual_associations_survive_restart_and_upgrade_the_versioned_store_without_losing_steam_links()
{
    let saved = json!({"schemaVersion":9, "users":[{"uid":1000,"username":"alice", "profiles":[], "games":[{"game":{"kind":"steam","appId":570},"name":"Dota 2","profiles":{}}]}]});
    let mut daemon = Harness::start_with_persistent_files(
        FakeGpuBackend::default(),
        &[(
            super::tuning_store::TUNING_STORE_FILE_NAME,
            &serde_json::to_vec(&saved).unwrap(),
        )],
    );
    daemon.write_host_file("/games/game", "binary");
    let alice = daemon.user(1000, "alice");
    ok(
        &alice,
        json!({"type":"add_program", "payload":{"path":"/games/game"}}),
    );
    let document: Value = serde_json::from_slice(
        &daemon
            .persistent_file(super::tuning_store::TUNING_STORE_FILE_NAME)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(document["schemaVersion"], 10);
    drop(alice);
    daemon.restart();
    let alice = daemon.user(1000, "alice");
    let games = ok(&alice, json!({"type":"get_games"}));
    assert_eq!(games["games"].as_array().unwrap().len(), 2);
    assert_eq!(games["games"][0]["name"], "Dota 2");
    assert_eq!(
        games["games"][1]["game"],
        json!({"kind":"program", "path":"/games/game"})
    );
}

#[test]
fn a_reused_uid_does_not_recognize_the_previous_accounts_manual_programs() {
    let daemon = Harness::start(FakeGpuBackend::default());
    daemon.write_host_file("/games/private", "binary");
    let alice = daemon.user(1000, "alice");
    ok(
        &alice,
        json!({"type":"add_program", "payload":{"path":"/games/private"}}),
    );
    drop(alice);
    let replacement = daemon.user(1000, "replacement");
    native(&daemon, 123, 1000, "/games/private");
    daemon.scan();
    assert_eq!(
        ok(&replacement, json!({"type":"get_running_games"}))["games"],
        json!([])
    );
    assert_eq!(
        ok(&replacement, json!({"type":"get_games"}))["games"],
        json!([])
    );
}

#[test]
fn native_tools_mapping_a_program_file_do_not_count_as_running_that_program() {
    let daemon = Harness::start(FakeGpuBackend::default());
    let alice = daemon.user(1000, "alice");
    daemon.write_host_file("/games/game.exe", "binary");
    daemon.write_host_file("/usr/bin/inspector", "binary");
    ok(
        &alice,
        json!({"type":"add_program", "payload":{"path":"/games/game.exe"}}),
    );
    native(&daemon, 123, 1000, "/usr/bin/inspector");
    map_image(&daemon, 123, "/games/game.exe", "/games/game.exe");
    daemon.scan();
    assert_eq!(
        ok(&alice, json!({"type":"get_running_games"}))["games"],
        json!([])
    );
}
