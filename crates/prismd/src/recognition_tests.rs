//! Seam 1: recognition through session responses and events, using only fixture processes.
use super::harness::{FakeGpuBackend, FakeSession, Harness};
use serde_json::{json, Value};

fn process(daemon: &Harness, pid: u32, uid: u32, start: u64, env: &str, cmd: &str) {
    daemon.write_host_file(&format!("/proc/{pid}/environ"), env);
    daemon.write_host_file(&format!("/proc/{pid}/cmdline"), cmd);
    daemon.write_host_file(
        &format!("/proc/{pid}/status"),
        &format!("Name:\tgame\nUid:\t{uid}\t0\t0\t0\n"),
    );
    // Field 22 is starttime, in clock ticks since boot. comm can contain spaces and ')'.
    daemon.write_host_file(
        &format!("/proc/{pid}/stat"),
        &format!("{pid} (game ) name) S 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 {start} 0\n"),
    );
}

fn running(session: &FakeSession) -> Value {
    let response = session.send_json(json!({"type": "get_running_games"}));
    assert_eq!(response["type"], "running_games", "{response}");
    response["payload"].clone()
}

#[test]
fn native_steam_process_is_recognized_for_its_real_uid_without_an_association() {
    let daemon = Harness::start(FakeGpuBackend::default());
    let alice = daemon.user(1000, "alice");
    process(
        &daemon,
        123,
        1000,
        420,
        "SteamAppId=570\0USER=alice\0",
        "/games/dota2\0",
    );
    daemon.core().scan_running_games();
    assert_eq!(
        running(&alice),
        json!({"status":"available", "games":[{
            "game":{"kind":"steam", "appId":570}, "pid":123, "startTimeTicks":420
        }]})
    );
    assert_eq!(
        running(&daemon.user(0, "root")),
        json!({"status":"available", "games":[]})
    );
}

#[test]
fn proton_flatpak_and_reaper_signals_group_by_game_and_keep_the_earliest_process() {
    let daemon = Harness::start(FakeGpuBackend::default());
    let alice = daemon.user(1000, "alice");
    // Proton helpers and game processes count; the lowest pid is not necessarily earliest.
    process(
        &daemon,
        10,
        1000,
        900,
        "SteamAppId=620\0",
        "wine64\0game.exe\0",
    );
    process(&daemon, 20, 1000, 500, "SteamAppId=620\0", "wineserver\0");
    // Flatpak portal ancestry is unrelated to the reaper. Only the environment is needed.
    process(&daemon, 30, 1000, 600, "SteamAppId=570\0", "/app/game\0");
    process(
        &daemon,
        40,
        1000,
        700,
        "",
        "/steam/ubuntu12_32/reaper\0SteamLaunch\0AppId=730\0--\0game\0",
    );
    daemon.core().scan_running_games();
    assert_eq!(
        running(&alice),
        json!({"status":"available", "games":[
            {"game":{"kind":"steam", "appId":570}, "pid":30, "startTimeTicks":600},
            {"game":{"kind":"steam", "appId":620}, "pid":20, "startTimeTicks":500},
            {"game":{"kind":"steam", "appId":730}, "pid":40, "startTimeTicks":700}
        ]})
    );
}

#[test]
fn client_install_scripts_and_arbitrary_appid_arguments_are_not_games() {
    let daemon = Harness::start(FakeGpuBackend::default());
    let alice = daemon.user(1000, "alice");
    process(&daemon, 1, 1000, 10, "SteamAppId=0\0", "steam\0");
    process(
        &daemon,
        2,
        1000,
        20,
        "STEAM_COMPAT_APP_ID=570\0",
        "proton\0runinprefix\0install.exe\0",
    );
    process(
        &daemon,
        3,
        1000,
        30,
        "",
        "reaper\0SteamLaunch\0AppId=570\0Install=1\0--\0install.exe\0",
    );
    process(&daemon, 4, 1000, 40, "", "editor\0AppId=620\0");
    process(
        &daemon,
        5,
        1000,
        50,
        "",
        "reaper\0SteamLaunch\0--\0program\0AppId=730\0",
    );
    daemon.core().scan_running_games();
    assert_eq!(running(&alice), json!({"status":"available", "games":[]}));
}

#[test]
fn changes_are_pushed_only_to_the_owner_and_partial_scans_are_unavailable_until_recovery() {
    let daemon = Harness::start(FakeGpuBackend::default());
    std::fs::create_dir_all(daemon.host_path("/proc")).unwrap();
    daemon.core().scan_running_games();
    let alice = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");
    alice.pushed_events();
    bob.pushed_events();
    process(&daemon, 123, 1000, 420, "SteamAppId=570\0", "dota2\0");
    daemon.core().scan_running_games();
    assert_eq!(
        alice.pushed_events(),
        vec![json!({"type":"runningGamesChanged", "payload":running(&alice)})]
    );
    assert!(bob.pushed_events().is_empty());
    daemon.core().scan_running_games();
    assert!(alice.pushed_events().is_empty());

    // A candidate's missing start time is not evidence that its game ended.
    std::fs::remove_file(daemon.host_path("/proc/123/stat")).unwrap();
    daemon.core().scan_running_games();
    assert_eq!(running(&alice)["status"], "unavailable");
    assert!(running(&alice).get("games").is_none());
    assert_eq!(alice.pushed_events()[0]["payload"]["status"], "unavailable");
    assert_eq!(bob.pushed_events()[0]["payload"]["status"], "unavailable");
    process(&daemon, 123, 1000, 420, "SteamAppId=570\0", "dota2\0");
    daemon.core().scan_running_games();
    assert_eq!(alice.pushed_events()[0]["payload"]["status"], "available");
    std::fs::remove_dir_all(daemon.host_path("/proc/123")).unwrap();
    daemon.core().scan_running_games();
    assert_eq!(
        alice.pushed_events()[0]["payload"],
        json!({"status":"available", "games":[]})
    );

    // A missing proc root, or an unreadable process environment, invalidates the whole scan.
    std::fs::remove_dir(daemon.host_path("/proc")).unwrap();
    daemon.core().scan_running_games();
    assert_eq!(running(&alice)["status"], "unavailable");
    process(&daemon, 456, 1000, 900, "SteamAppId=620\0", "portal2\0");
    std::fs::remove_file(daemon.host_path("/proc/456/environ")).unwrap();
    daemon.core().scan_running_games();
    assert_eq!(running(&alice)["status"], "unavailable");
}

#[test]
fn environment_takes_precedence_and_helpers_keep_a_game_running() {
    let daemon = Harness::start(FakeGpuBackend::default());
    let alice = daemon.user(1000, "alice");
    process(
        &daemon,
        1,
        1000,
        100,
        "SteamAppId=570\0",
        "reaper\0SteamLaunch\0AppId=620\0--\0game\0",
    );
    process(&daemon, 2, 1000, 100, "SteamAppId=570\0", "helper\0");
    process(&daemon, 3, 1001, 90, "SteamAppId=570\0", "game\0");
    daemon.core().scan_running_games();
    assert_eq!(running(&alice)["games"].as_array().unwrap().len(), 1);
    assert_eq!(running(&alice)["games"][0]["pid"], 1);
    assert_eq!(running(&daemon.user(1001, "bob"))["games"][0]["pid"], 3);
    std::fs::remove_dir_all(daemon.host_path("/proc/1")).unwrap();
    daemon.core().scan_running_games();
    assert_eq!(running(&alice)["games"][0]["pid"], 2);
}

#[test]
fn malformed_candidate_details_cannot_be_reported_as_a_running_game() {
    let daemon = Harness::start(FakeGpuBackend::default());
    let alice = daemon.user(1000, "alice");
    process(&daemon, 123, 1000, 420, "SteamAppId=570\0", "dota2\0");
    daemon.write_host_file("/proc/123/status", "Uid:\tunknown\t1000\t1000\t1000\n");
    daemon.core().scan_running_games();
    assert_eq!(running(&alice)["status"], "unavailable");
    process(&daemon, 123, 1000, 420, "SteamAppId=570\0", "dota2\0");
    let stat = std::fs::read_to_string(daemon.host_path("/proc/123/stat")).unwrap();
    daemon.write_host_file("/proc/123/stat", &stat.replacen("123", "999", 1));
    daemon.core().scan_running_games();
    assert_eq!(running(&alice)["status"], "unavailable");
}

#[test]
fn kernel_threads_and_exited_tasks_without_environments_do_not_abort_recognition() {
    let daemon = Harness::start(FakeGpuBackend::default());
    let alice = daemon.user(1000, "alice");
    process(&daemon, 123, 1000, 420, "SteamAppId=570\0", "dota2\0");
    for (pid, state, flags) in [(2, "S", 0x0020_0000_u32), (3, "Z", 0), (4, "X", 0)] {
        process(&daemon, pid, 0, 10, "", "");
        daemon.write_host_file(
            &format!("/proc/{pid}/stat"),
            &format!("{pid} (no userspace) {state} 1 0 0 0 0 {flags} 0 0 0 0 0 0 0 0 0 0 0 10 0\n"),
        );
        // As on the live host, these still have a stat entry but no readable environment.
        std::fs::remove_file(daemon.host_path(&format!("/proc/{pid}/environ"))).unwrap();
        daemon.core().scan_running_games();
        assert_eq!(running(&alice)["status"], "available", "pid {pid}");
        assert_eq!(running(&alice)["games"][0]["pid"], 123);
    }
}

#[test]
fn exit_during_a_scan_is_verified_and_pid_reuse_remains_unknown() {
    use std::io::Write;
    use std::os::unix::ffi::OsStrExt;
    for reused in [false, true] {
        let daemon = Harness::start(FakeGpuBackend::default());
        let alice = daemon.user(1000, "alice");
        process(&daemon, 123, 1000, 420, "", "game\0");
        let environment = daemon.host_path("/proc/123/environ");
        std::fs::remove_file(&environment).unwrap();
        let name = std::ffi::CString::new(environment.as_os_str().as_bytes()).unwrap();
        // SAFETY: name is NUL-terminated and remains valid for the syscall.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        std::thread::scope(|scope| {
            let scan = scope.spawn(|| daemon.core().scan_running_games());
            // Opening the FIFO rendezvous with the real scan after its first stat read.
            let mut writer = std::fs::OpenOptions::new()
                .write(true)
                .open(&environment)
                .unwrap();
            if reused {
                let stat = std::fs::read_to_string(daemon.host_path("/proc/123/stat")).unwrap();
                daemon.write_host_file("/proc/123/stat", &stat.replace("420", "900"));
                writer.write_all(b"SteamAppId=570\0").unwrap();
            } else {
                std::fs::remove_dir_all(daemon.host_path("/proc/123")).unwrap();
            }
            drop(writer);
            scan.join().unwrap();
        });
        if reused {
            assert_eq!(running(&alice)["status"], "unavailable");
        } else {
            assert_eq!(running(&alice), json!({"status":"available", "games":[]}));
        }
    }
}
