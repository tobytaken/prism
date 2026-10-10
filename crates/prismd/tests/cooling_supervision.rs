//! Process-level supervision of fan curves against fake hardware: the daemon runs as its own
//! process, and the test plays the service manager. Recovery after a graceful stop, an abnormal
//! termination and a stalled curve loop must not depend on that loop or on an app, and the next
//! daemon of the boot resumes only curves that were healthy.
//!
//! This binary re-runs itself as the daemon (`PRISM_TEST_ROLE=daemon`) or as the service
//! manager's stop command (`PRISM_TEST_ROLE=restore`), with GPUs kept in files.

use std::io::{Read, Write};
use std::os::unix::net::{UnixDatagram, UnixStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use prism_gpu::{
    FanControlInfo, FanControlMode, GpuInventoryResponse, GpuPerformanceDevice,
    GpuPerformanceResponse, GpuTargetRequest, SetGpuClockOffsetsRequest, SetGpuFanModeAutoRequest,
    SetGpuFanSpeedRequest, SetGpuPowerLimitRequest,
};
use prismd::DaemonBackend;
use serde_json::{json, Value};

const DEADLINE: Duration = Duration::from_secs(10);

fn main() {
    match std::env::var("PRISM_TEST_ROLE").as_deref() {
        Ok("daemon") => {
            let gpus = Arc::new(FileGpus(gpu_dir()));
            if let Err(error) = prismd::run_with(gpus) {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        Ok("restore") => prismd::restore_cooling_with(&FileGpus(gpu_dir())).unwrap(),
        _ => run_tests(),
    }
}

fn run_tests() {
    let tests: [(&str, fn()); 4] = [
        (
            "a_graceful_stop_restores_automatic_cooling_and_the_next_daemon_resumes_the_curve",
            a_graceful_stop_restores_automatic_cooling_and_the_next_daemon_resumes_the_curve,
        ),
        (
            "after_an_abnormal_termination_the_stop_command_restores_automatic_cooling",
            after_an_abnormal_termination_the_stop_command_restores_automatic_cooling,
        ),
        (
            "a_failed_restore_is_recorded_rather_than_reported_as_recovered",
            a_failed_restore_is_recorded_rather_than_reported_as_recovered,
        ),
        (
            "a_stalled_curve_starves_the_watchdog_and_the_stop_command_still_recovers",
            a_stalled_curve_starves_the_watchdog_and_the_stop_command_still_recovers,
        ),
    ];
    let mut failed = 0;
    for (name, test) in tests {
        match std::panic::catch_unwind(test) {
            Ok(()) => println!("test {name} ... ok"),
            Err(_) => {
                println!("test {name} ... FAILED");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        std::process::exit(1);
    }
}

fn gpu_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("PRISM_TEST_GPU_DIR").unwrap())
}

/// GPUs whose state lives in files the test can read: `gpus.json`, overlaid by a `temperature`
/// file. While a `hang` file exists, fan speed writes never finish; while `fail-auto-<fan>`
/// exists, the driver rejects returning that fan to automatic control.
struct FileGpus(PathBuf);

impl FileGpus {
    fn read(&self) -> GpuPerformanceResponse {
        let mut performance: GpuPerformanceResponse =
            serde_json::from_slice(&std::fs::read(self.0.join("gpus.json")).unwrap()).unwrap();
        if let Ok(temperature) = std::fs::read_to_string(self.0.join("temperature")) {
            performance.gpus[0].telemetry.temperature_c = temperature.trim().parse().ok();
        }
        performance
    }

    fn change_fan(&self, fan_index: u32, change: impl FnOnce(&mut FanControlInfo)) {
        let mut performance: GpuPerformanceResponse =
            serde_json::from_slice(&std::fs::read(self.0.join("gpus.json")).unwrap()).unwrap();
        change(&mut performance.gpus[0].cooling.fans[fan_index as usize]);
        let staged = self.0.join("gpus.json.staged");
        std::fs::write(&staged, serde_json::to_vec(&performance).unwrap()).unwrap();
        std::fs::rename(staged, self.0.join("gpus.json")).unwrap();
    }
}

impl DaemonBackend for FileGpus {
    fn inventory(&self) -> GpuInventoryResponse {
        GpuInventoryResponse::default()
    }

    fn performance(&self) -> GpuPerformanceResponse {
        self.read()
    }

    fn set_power_limit(&self, _: SetGpuPowerLimitRequest) -> Result<(), String> {
        Err("unsupported".into())
    }

    fn reset_power_limit(&self, _: GpuTargetRequest) -> Result<(), String> {
        Err("unsupported".into())
    }

    fn set_fan_speed(&self, request: SetGpuFanSpeedRequest) -> Result<(), String> {
        while self.0.join("hang").exists() {
            std::thread::sleep(Duration::from_millis(20));
        }
        self.change_fan(request.fan_index, |fan| {
            fan.mode = Some(FanControlMode::Manual);
            fan.current_speed_percent = Some(request.speed_percent);
        });
        Ok(())
    }

    fn set_fan_mode_auto(&self, request: SetGpuFanModeAutoRequest) -> Result<(), String> {
        if self
            .0
            .join(format!("fail-auto-{}", request.fan_index))
            .exists()
        {
            return Err("Insufficient permissions".into());
        }
        self.change_fan(request.fan_index, |fan| {
            fan.mode = Some(FanControlMode::Auto)
        });
        Ok(())
    }

    fn set_clock_offsets(&self, _: SetGpuClockOffsetsRequest) -> Result<(), String> {
        Err("unsupported".into())
    }
}

/// A host for one daemon process: its directories, configuration and GPUs, and the
/// notification socket the service manager listens on.
struct Host {
    dir: tempfile::TempDir,
    notifications: UnixDatagram,
}

impl Host {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        for sub in ["gpus", "state", "run"] {
            std::fs::create_dir(dir.path().join(sub)).unwrap();
        }
        std::fs::write(
            dir.path().join("prismd.toml"),
            format!(
                "adminGroup = \"{}\"\nadminUser = \"{}\"\n",
                users::get_current_groupname().unwrap().to_string_lossy(),
                users::get_current_username().unwrap().to_string_lossy(),
            ),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("gpus/gpus.json"),
            serde_json::to_vec(&GpuPerformanceResponse {
                gpus: vec![cooled_gpu()],
                warnings: vec![],
            })
            .unwrap(),
        )
        .unwrap();
        let notifications = UnixDatagram::bind(dir.path().join("notify.sock")).unwrap();
        notifications
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        Self { dir, notifications }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn command(&self, role: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .env("PRISM_TEST_ROLE", role)
            .env("PRISM_TEST_GPU_DIR", self.path("gpus"))
            .env("PRISM_SOCKET_PATH", self.path("run/prismd.sock"))
            .env("PRISM_STATE_DIR", self.path("state"))
            .env("PRISM_BOOT_STATE_DIR", self.path("run"))
            .env(
                "PRISM_DRIVER_JOBS_PATH",
                self.path("state/driver-jobs.json"),
            )
            .env("PRISM_CONFIG_PATH", self.path("prismd.toml"))
            .env("NOTIFY_SOCKET", self.path("notify.sock"))
            .env_remove("WATCHDOG_PID")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    /// Starts the daemon and waits until it reports that startup finished.
    fn start_daemon(&self, watchdog: Option<Duration>) -> Daemon {
        let mut command = self.command("daemon");
        match watchdog {
            Some(interval) => command.env("WATCHDOG_USEC", interval.as_micros().to_string()),
            None => command.env_remove("WATCHDOG_USEC"),
        };
        let daemon = Daemon(command.spawn().unwrap());
        let deadline = Instant::now() + DEADLINE;
        while !self
            .notifications_until(deadline)
            .contains(&"READY=1".to_string())
        {
            assert!(
                Instant::now() < deadline,
                "the daemon never reported readiness"
            );
        }
        daemon
    }

    /// Notifications received before the deadline or, once one arrives, shortly after it.
    fn notifications_until(&self, deadline: Instant) -> Vec<String> {
        let mut received = vec![];
        let mut buffer = [0; 64];
        while Instant::now() < deadline {
            match self.notifications.recv(&mut buffer) {
                Ok(size) => received.push(String::from_utf8_lossy(&buffer[..size]).into()),
                Err(_) if !received.is_empty() => break,
                Err(_) => {}
            }
        }
        received
    }

    /// Sends one request to the daemon, as a client of its socket would.
    fn send(&self, request: Value) -> Value {
        let mut stream = UnixStream::connect(self.path("run/prismd.sock")).unwrap();
        stream
            .write_all(&serde_json::to_vec(&request).unwrap())
            .unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();
        let mut response = vec![];
        stream.read_to_end(&mut response).unwrap();
        serde_json::from_slice(&response).unwrap()
    }

    fn apply_starter_curve(&self) {
        let response = self.send(json!({
            "type": "apply_cooling",
            "payload": {
                "gpuId": "GPU-123",
                "configuration": {
                    "arrangement": "shared",
                    "setting": { "mode": "curve", "points": [
                        { "temperatureC": 45, "speedPercent": 20 },
                        { "temperatureC": 65, "speedPercent": 60 },
                        { "temperatureC": 75, "speedPercent": 80 },
                        { "temperatureC": 85, "speedPercent": 100 },
                    ] },
                },
            },
        }));
        assert_eq!(
            response["payload"]["outcome"]["status"], "succeeded",
            "{response}"
        );
    }

    fn fan_modes(&self) -> Vec<Option<FanControlMode>> {
        FileGpus(self.path("gpus")).read().gpus[0]
            .cooling
            .fans
            .iter()
            .map(|fan| fan.mode)
            .collect()
    }

    fn wait_for_fan_modes(&self, expected: FanControlMode) {
        let deadline = Instant::now() + DEADLINE;
        while self.fan_modes() != vec![Some(expected); 2] {
            assert!(
                Instant::now() < deadline,
                "fans stayed {:?}",
                self.fan_modes()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// What the boot-scoped record holds about the GPU's cooling, for the next daemon.
    fn recorded(&self) -> Value {
        let record: Value =
            serde_json::from_slice(&std::fs::read(self.path("run/cooling-control.json")).unwrap())
                .unwrap();
        record["gpus"]["GPU-123"].clone()
    }

    /// Prism's control of the GPU's fans, as the running daemon reports it.
    fn control(&self) -> Value {
        self.send(json!({ "type": "get_performance" }))["payload"]["gpus"][0]["cooling"]["control"]
            .clone()
    }

    /// Runs the service manager's stop command, as ExecStopPost does after any stop.
    fn run_stop_command(&self) {
        let status = self.command("restore").status().unwrap();
        assert!(status.success());
    }
}

struct Daemon(Child);

impl Daemon {
    fn signal(&mut self, signal: libc::c_int) {
        // SAFETY: the pid belongs to this test's own child process.
        unsafe { libc::kill(self.0.id() as libc::pid_t, signal) };
    }

    fn wait_for_exit(&mut self) {
        let deadline = Instant::now() + DEADLINE;
        while self.0.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "the daemon didn't exit");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn cooled_gpu() -> GpuPerformanceDevice {
    let fixture: GpuPerformanceResponse = serde_json::from_str(include_str!(
        "../../prism-protocol/tests/contracts/tuning-observations.json"
    ))
    .unwrap();
    let mut gpu = fixture.gpus.into_iter().next().unwrap();
    gpu.cooling.is_supported = true;
    gpu.cooling.unsupported_reason = None;
    gpu.cooling.fan_count = 2;
    gpu.cooling.fans = (0..2)
        .map(|index| FanControlInfo {
            index,
            current_speed_percent: Some(40),
            min_speed_percent: Some(20),
            max_speed_percent: Some(100),
            mode: Some(FanControlMode::Auto),
        })
        .collect();
    gpu.cooling.max_operating_temperature_c = Some(90);
    gpu.telemetry.temperature_c = Some(71);
    gpu
}

fn a_graceful_stop_restores_automatic_cooling_and_the_next_daemon_resumes_the_curve() {
    let host = Host::new();
    let mut daemon = host.start_daemon(None);
    host.apply_starter_curve();
    assert_eq!(host.fan_modes(), vec![Some(FanControlMode::Manual); 2]);

    daemon.signal(libc::SIGTERM);
    daemon.wait_for_exit();
    host.run_stop_command();

    assert_eq!(host.fan_modes(), vec![Some(FanControlMode::Auto); 2]);
    assert_eq!(
        host.recorded()["interrupted"]["restoration"][1]["status"],
        "restored"
    );
    assert_eq!(host.recorded()["suspension"], Value::Null);

    std::fs::write(host.path("gpus/temperature"), "80").unwrap();
    let _daemon = host.start_daemon(None);
    assert_eq!(host.fan_modes(), vec![Some(FanControlMode::Manual); 2]);
    let control = host.control();
    assert_eq!(control["recovery"]["status"], "resumed");
    assert_eq!(control["requested"][1]["speedPercent"], 90);
}

fn after_an_abnormal_termination_the_stop_command_restores_automatic_cooling() {
    let host = Host::new();
    let mut daemon = host.start_daemon(None);
    host.apply_starter_curve();

    daemon.signal(libc::SIGKILL);
    daemon.wait_for_exit();
    assert_eq!(host.fan_modes(), vec![Some(FanControlMode::Manual); 2]);
    host.run_stop_command();

    assert_eq!(host.fan_modes(), vec![Some(FanControlMode::Auto); 2]);
    assert_eq!(
        host.recorded()["interrupted"]["restoration"][1]["status"],
        "restored"
    );

    // The service manager restarts the daemon, which resumes the healthy curve.
    let _daemon = host.start_daemon(None);
    assert_eq!(host.fan_modes(), vec![Some(FanControlMode::Manual); 2]);
    assert_eq!(host.control()["recovery"]["status"], "resumed");
}

fn a_stalled_curve_starves_the_watchdog_and_the_stop_command_still_recovers() {
    let host = Host::new();
    let mut daemon = host.start_daemon(Some(Duration::from_millis(1_500)));
    host.apply_starter_curve();
    let pings = |seconds| {
        host.notifications_until(Instant::now() + Duration::from_secs(seconds))
            .into_iter()
            .filter(|message| message == "WATCHDOG=1")
            .count()
    };
    let mut healthy = 0;
    for _ in 0..4 {
        healthy += pings(1);
    }
    assert!(
        healthy >= 2,
        "a healthy curve loop sent {healthy} watchdog pings"
    );

    // The next curve write never finishes.
    std::fs::write(host.path("gpus/hang"), "").unwrap();
    std::fs::write(host.path("gpus/temperature"), "80").unwrap();
    std::thread::sleep(Duration::from_secs(2));
    while host.notifications.recv(&mut [0; 64]).is_ok() {}
    let mut stalled = 0;
    for _ in 0..3 {
        stalled += pings(1);
    }
    assert_eq!(
        stalled, 0,
        "a stalled curve loop kept the watchdog satisfied"
    );

    // The service manager kills the daemon (systemd's watchdog sends SIGABRT, then SIGKILL),
    // then runs its stop command.
    daemon.signal(libc::SIGKILL);
    daemon.wait_for_exit();
    host.run_stop_command();
    host.wait_for_fan_modes(FanControlMode::Auto);
    assert_eq!(
        host.recorded()["suspension"]["reason"],
        "Prism's fan curve control stopped responding, so the daemon stopped following this curve."
    );

    // A stalled curve is a fault: the restarted daemon doesn't resume it.
    std::fs::remove_file(host.path("gpus/hang")).unwrap();
    let _daemon = host.start_daemon(None);
    assert_eq!(host.fan_modes(), vec![Some(FanControlMode::Auto); 2]);
    assert_eq!(host.control()["requested"], json!([]));
}

fn a_failed_restore_is_recorded_rather_than_reported_as_recovered() {
    let host = Host::new();
    let mut daemon = host.start_daemon(None);
    host.apply_starter_curve();
    std::fs::write(host.path("gpus/fail-auto-1"), "").unwrap();

    daemon.signal(libc::SIGKILL);
    daemon.wait_for_exit();
    host.run_stop_command();

    assert_eq!(
        host.fan_modes(),
        vec![Some(FanControlMode::Auto), Some(FanControlMode::Manual)]
    );
    let restoration = &host.recorded()["interrupted"]["restoration"];
    assert_eq!(restoration[0]["status"], "restored");
    assert_eq!(restoration[1]["status"], "failed");
    assert_eq!(restoration[1]["detail"], "Insufficient permissions");

    // The next daemon can't resume without a temperature, and reports that fan 2 still couldn't
    // return to automatic cooling rather than hiding it.
    std::fs::write(host.path("gpus/temperature"), "unavailable").unwrap();
    let _daemon = host.start_daemon(None);
    let control = host.control();
    assert_eq!(control["recovery"]["status"], "retryRequired");
    assert_eq!(control["suspension"]["restoration"][1]["status"], "failed");
}
