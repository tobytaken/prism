//! Seam 1: drives the daemon core below the socket with a fake GPU backend, temporary
//! persistent and boot-scoped directories, an injected boot id and a controllable clock.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use tempfile::TempDir;

use super::migration::LEGACY_STATE_FILE_NAME;
use super::{
    Clock, CoreDependencies, DaemonBackend, DaemonCore, DaemonSession, HostShutdownSignal,
    SessionPeer, SessionPolicy, UserDirectory,
};
use crate::drivers::DriverService;
use crate::gpu::{
    ControlSupport, FanControlInfo, FanControlMode, GpuDaemonStatus, GpuInventoryResponse,
    GpuPerformanceDevice, GpuPerformanceResponse, GpuTargetRequest, SetGpuClockOffsetsRequest,
    SetGpuFanModeAutoRequest, SetGpuFanSpeedRequest, SetGpuPowerLimitRequest,
};
use crate::protocol::{DaemonRequest, DaemonResponse};

/// Longer than any request should take, so a stuck request fails its test instead of hanging.
const RESPONSE_DEADLINE: Duration = Duration::from_secs(5);

/// The GPU from the shared contract fixture: power reported but unreadable, offsets untunable.
pub fn observed_gpu() -> GpuPerformanceDevice {
    let performance: GpuPerformanceResponse = serde_json::from_str(include_str!(
        "../../../src/test/contracts/tuning-observations.json"
    ))
    .unwrap();
    performance.gpus.into_iter().next().unwrap()
}

/// GPU-123 with readable values and every profile setting supported.
pub fn tunable_gpu() -> GpuPerformanceDevice {
    let mut gpu = observed_gpu();
    gpu.power.current_limit_watts = Some(475);
    gpu.power.current_value_error = None;
    gpu.clocks.graphics.support = ControlSupport::Supported;
    gpu.clocks.graphics.current_offset_mhz = Some(120);
    gpu.clocks.graphics.min_offset_mhz = Some(-200);
    gpu.clocks.graphics.max_offset_mhz = Some(200);
    gpu.clocks.graphics.reason = None;
    gpu.clocks.memory = gpu.clocks.graphics.clone();
    gpu.clocks.memory.current_offset_mhz = Some(0);
    gpu
}

/// GPU-123 with two fans sharing a 20-100% range, a 90°C maximum operating temperature and a
/// 71°C reading.
pub fn cooled_gpu() -> GpuPerformanceDevice {
    let mut gpu = tunable_gpu();
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
    gpu.cooling.max_operating_temperature_reason = None;
    gpu.telemetry.temperature_c = Some(71);
    gpu
}

/// A tunable GPU with its own identity.
pub fn tunable_gpu_with_id(id: &str) -> GpuPerformanceDevice {
    let mut gpu = tunable_gpu();
    gpu.id = id.into();
    gpu.stable_id = Some(id.into());
    gpu
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Setter {
    PowerLimit,
    ResetPowerLimit,
    FanSpeed,
    FanModeAuto,
    GraphicsOffset,
    MemoryOffset,
}

/// A stateful fake of the host's GPUs. Setters change the values that performance reads report.
#[derive(Default)]
pub struct FakeGpuBackend {
    state: Mutex<FakeGpus>,
    performance_gate: Mutex<Option<Gate>>,
}

#[derive(Default)]
struct FakeGpus {
    gpus: Vec<GpuPerformanceDevice>,
    failures: HashMap<(String, Setter), String>,
    gates: HashMap<(String, Setter), Gate>,
    writes: Vec<(String, Setter)>,
}

impl FakeGpuBackend {
    pub fn with_gpus(gpus: Vec<GpuPerformanceDevice>) -> Self {
        let backend = Self::default();
        backend.state.lock().unwrap().gpus = gpus;
        backend
    }

    pub fn gpu(&self, id: &str) -> GpuPerformanceDevice {
        self.state
            .lock()
            .unwrap()
            .gpus
            .iter()
            .find(|gpu| gpu.id == id)
            .cloned()
            .unwrap_or_else(|| panic!("no fake GPU {id}"))
    }

    pub fn gpus(&self) -> Vec<GpuPerformanceDevice> {
        self.state.lock().unwrap().gpus.clone()
    }

    /// Changes what the GPU reports, such as its capabilities, ranges or current values.
    pub fn update(&self, id: &str, change: impl FnOnce(&mut GpuPerformanceDevice)) {
        let mut state = self.state.lock().unwrap();
        let gpu = state
            .gpus
            .iter_mut()
            .find(|gpu| gpu.id == id)
            .unwrap_or_else(|| panic!("no fake GPU {id}"));
        change(gpu);
    }

    pub fn arrive(&self, gpu: GpuPerformanceDevice) {
        self.state.lock().unwrap().gpus.push(gpu);
    }

    pub fn depart(&self, id: &str) {
        self.state.lock().unwrap().gpus.retain(|gpu| gpu.id != id);
    }

    /// Makes every later call of the setter on the GPU fail with the message.
    pub fn fail(&self, id: &str, setter: Setter, message: &str) {
        self.state
            .lock()
            .unwrap()
            .failures
            .insert((id.into(), setter), message.into());
    }

    pub fn stop_failing(&self, id: &str, setter: Setter) {
        self.state
            .lock()
            .unwrap()
            .failures
            .remove(&(id.into(), setter));
    }

    /// Makes the next call of the setter on the GPU wait until the returned gate is released.
    pub fn block(&self, id: &str, setter: Setter) -> Gate {
        let gate = Gate::default();
        self.state
            .lock()
            .unwrap()
            .gates
            .insert((id.into(), setter), gate.clone());
        gate
    }

    /// Pauses the next hardware reading to exercise arrival/deadline races deterministically.
    pub fn block_performance(&self) -> Gate {
        let gate = Gate::default();
        *self.performance_gate.lock().unwrap() = Some(gate.clone());
        gate
    }

    /// Every setter call so far, in the order the calls started.
    pub fn writes(&self) -> Vec<(String, Setter)> {
        self.state.lock().unwrap().writes.clone()
    }

    fn write(
        &self,
        id: &str,
        setter: Setter,
        change: impl FnOnce(&mut GpuPerformanceDevice),
    ) -> Result<(), String> {
        let gate = {
            let mut state = self.state.lock().unwrap();
            state.writes.push((id.into(), setter));
            state.gates.remove(&(id.into(), setter))
        };
        if let Some(gate) = gate {
            gate.pass();
        }
        let mut state = self.state.lock().unwrap();
        if let Some(message) = state.failures.get(&(id.into(), setter)) {
            return Err(message.clone());
        }
        let gpu = state
            .gpus
            .iter_mut()
            .find(|gpu| gpu.id == id)
            .ok_or_else(|| "GPU unavailable".to_string())?;
        change(gpu);
        Ok(())
    }
}

impl DaemonBackend for FakeGpuBackend {
    fn inventory(&self) -> GpuInventoryResponse {
        GpuInventoryResponse {
            gpus: vec![],
            warnings: vec![],
        }
    }

    fn performance(&self) -> GpuPerformanceResponse {
        let gate = self.performance_gate.lock().unwrap().take();
        if let Some(gate) = gate {
            gate.pass();
        }
        GpuPerformanceResponse {
            gpus: self.gpus(),
            warnings: vec![],
        }
    }

    fn set_power_limit(&self, request: SetGpuPowerLimitRequest) -> Result<(), String> {
        self.write(&request.gpu_id, Setter::PowerLimit, |gpu| {
            gpu.power.current_limit_watts = Some(request.limit_watts)
        })
    }

    fn reset_power_limit(&self, request: GpuTargetRequest) -> Result<(), String> {
        self.write(&request.gpu_id, Setter::ResetPowerLimit, |gpu| {
            gpu.power.current_limit_watts = gpu.power.default_limit_watts
        })
    }

    fn set_fan_speed(&self, request: SetGpuFanSpeedRequest) -> Result<(), String> {
        self.write(&request.gpu_id, Setter::FanSpeed, |gpu| {
            if let Some(fan) = gpu.cooling.fans.get_mut(request.fan_index as usize) {
                fan.current_speed_percent = Some(request.speed_percent);
                fan.mode = Some(FanControlMode::Manual);
            }
        })
    }

    fn set_fan_mode_auto(&self, request: SetGpuFanModeAutoRequest) -> Result<(), String> {
        self.write(&request.gpu_id, Setter::FanModeAuto, |gpu| {
            if let Some(fan) = gpu.cooling.fans.get_mut(request.fan_index as usize) {
                fan.mode = Some(FanControlMode::Auto);
            }
        })
    }

    fn set_clock_offsets(&self, request: SetGpuClockOffsetsRequest) -> Result<(), String> {
        if let Some(offset) = request.graphics_offset_mhz {
            self.write(&request.gpu_id, Setter::GraphicsOffset, |gpu| {
                gpu.clocks.graphics.current_offset_mhz = Some(offset)
            })?;
        }
        if let Some(offset) = request.memory_offset_mhz {
            self.write(&request.gpu_id, Setter::MemoryOffset, |gpu| {
                gpu.clocks.memory.current_offset_mhz = Some(offset)
            })?;
        }
        Ok(())
    }
}

/// Holds one setter call until released, and lets a test wait until the call has arrived.
#[derive(Clone, Default)]
pub struct Gate(Arc<(Mutex<GateState>, Condvar)>);

#[derive(Default)]
struct GateState {
    entered: bool,
    released: bool,
}

impl Gate {
    pub fn wait_until_entered(&self) {
        let (state, changed) = &*self.0;
        let (_entered, timeout) = changed
            .wait_timeout_while(state.lock().unwrap(), RESPONSE_DEADLINE, |state| {
                !state.entered
            })
            .unwrap();
        assert!(!timeout.timed_out(), "the blocked setter was never called");
    }

    pub fn release(&self) {
        let (state, changed) = &*self.0;
        state.lock().unwrap().released = true;
        changed.notify_all();
    }

    fn pass(&self) {
        let (state, changed) = &*self.0;
        let mut guard = state.lock().unwrap();
        guard.entered = true;
        changed.notify_all();
        let _released = changed.wait_while(guard, |state| !state.released).unwrap();
    }
}

#[derive(Default)]
pub struct FakeClock(AtomicU64);

impl FakeClock {
    pub fn set(&self, unix_ms: u64) {
        self.0.store(unix_ms, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now_unix_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Default)]
pub struct FakeHostShutdown(AtomicBool);

impl HostShutdownSignal for FakeHostShutdown {
    fn host_shutting_down(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// The host's accounts: each uid has the username its latest session opened with, so opening a
/// session under a new username renames the account.
#[derive(Default)]
struct FakeUsers(Mutex<HashMap<u32, String>>);

impl UserDirectory for FakeUsers {
    fn username(&self, uid: u32) -> Option<String> {
        self.0.lock().unwrap().get(&uid).cloned()
    }
}

pub struct Harness {
    pub gpus: Arc<FakeGpuBackend>,
    pub clock: Arc<FakeClock>,
    host_shutting_down: Arc<FakeHostShutdown>,
    persistent_dir: TempDir,
    boot_dir: TempDir,
    host_root: TempDir,
    users: Arc<FakeUsers>,
    boot_id: String,
    boots: u32,
    core: Option<Arc<DaemonCore>>,
}

impl Harness {
    pub fn start(gpus: FakeGpuBackend) -> Self {
        Self::start_with_saved_state(gpus, None)
    }

    /// Starts the daemon on a persistent directory that already holds this saved state.
    pub fn start_with_saved_state(gpus: FakeGpuBackend, saved: Option<&[u8]>) -> Self {
        match saved {
            Some(saved) => {
                Self::start_with_persistent_files(gpus, &[(LEGACY_STATE_FILE_NAME, saved)])
            }
            None => Self::start_with_persistent_files(gpus, &[]),
        }
    }

    /// Starts the daemon on a persistent directory that already holds these files.
    pub fn start_with_persistent_files(gpus: FakeGpuBackend, files: &[(&str, &[u8])]) -> Self {
        let persistent_dir = tempfile::tempdir().unwrap();
        for (name, contents) in files {
            std::fs::write(persistent_dir.path().join(name), contents).unwrap();
        }
        let mut harness = Self {
            gpus: Arc::new(gpus),
            clock: Arc::default(),
            host_shutting_down: Arc::default(),
            persistent_dir,
            boot_dir: tempfile::tempdir().unwrap(),
            host_root: tempfile::tempdir().unwrap(),
            users: Arc::default(),
            boot_id: "boot-1".into(),
            boots: 1,
            core: None,
        };
        harness.start_core();
        harness
    }

    /// Simulates a daemon restart mid-boot: stops the core while the host keeps running and
    /// starts a new one on the same directories.
    pub fn restart(&mut self) {
        self.core().stop();
        self.core = None;
        self.start_core();
    }

    /// Simulates a clean host reboot: the daemon is stopped while the host shuts down, the
    /// persistent directory survives, and the boot-scoped directory starts empty under a new
    /// boot id.
    pub fn reboot(&mut self) {
        self.host_shutting_down.0.store(true, Ordering::SeqCst);
        self.core().stop();
        self.host_shutting_down.0.store(false, Ordering::SeqCst);
        self.crash_and_reboot();
    }

    /// Simulates a host crash: the host restarts without stopping the daemon first.
    pub fn crash_and_reboot(&mut self) {
        self.core = None;
        self.boots += 1;
        self.boot_dir = tempfile::tempdir().unwrap();
        self.boot_id = format!("boot-{}", self.boots);
        self.start_core();
    }

    /// Simulates a new kernel boot id even when the runtime directory wasn't cleared.
    pub fn reboot_with_retained_boot_state(&mut self) {
        self.core = None;
        self.boots += 1;
        self.boot_id = format!("boot-{}", self.boots);
        self.start_core();
    }

    fn start_core(&mut self) {
        let persistent_dir = self.persistent_dir.path().to_path_buf();
        self.core = Some(Arc::new(DaemonCore::start(CoreDependencies {
            backend: Arc::clone(&self.gpus) as Arc<dyn DaemonBackend>,
            drivers: DriverService::new(persistent_dir.join("driver-jobs.json")),
            persistent_dir,
            boot_dir: self.boot_dir.path().to_path_buf(),
            boot_id: self.boot_id.clone(),
            clock: Arc::clone(&self.clock) as Arc<dyn Clock>,
            host_shutting_down: Arc::clone(&self.host_shutting_down) as Arc<dyn HostShutdownSignal>,
            socket_path: PathBuf::from("/tmp/prismd-test.sock"),
            host_root: self.host_root.path().to_path_buf(),
            users: Arc::clone(&self.users) as Arc<dyn UserDirectory>,
        })));
    }

    /// Opens a session as this user, who is admitted as an administrator. The uid's account now
    /// has this username.
    pub fn user(&self, uid: u32, username: &str) -> FakeSession {
        self.users.0.lock().unwrap().insert(uid, username.into());
        let policy = SessionPolicy {
            admin_group: "admins".into(),
            admin_user: None,
        };
        let peer = SessionPeer {
            uid,
            username: username.into(),
            groups: vec!["admins".into()],
            primary_group: "users".into(),
            home: format!("/home/{username}").into(),
        };
        let core = self.core();
        let session = core.open_session(peer, &policy).unwrap();
        FakeSession { core, session }
    }

    pub fn core(&self) -> Arc<DaemonCore> {
        Arc::clone(self.core.as_ref().expect("the daemon is running"))
    }

    pub fn send(&self, request: DaemonRequest) -> DaemonResponse {
        self.send_in_background(request).response()
    }

    /// Sends a request built from protocol JSON and returns the response as protocol JSON.
    pub fn send_json(&self, request: serde_json::Value) -> serde_json::Value {
        let request = serde_json::from_value(request).unwrap();
        serde_json::to_value(self.send(request)).unwrap()
    }

    pub fn performance(&self) -> GpuPerformanceResponse {
        let DaemonResponse::Performance(performance) = self.send(DaemonRequest::GetPerformance)
        else {
            panic!("expected performance")
        };
        performance
    }

    pub fn status(&self) -> GpuDaemonStatus {
        let DaemonResponse::DaemonStatus(status) = self.send(DaemonRequest::GetDaemonStatus) else {
            panic!("expected daemon status")
        };
        status
    }

    /// Sends the request from its own connection without waiting for the response.
    pub fn send_in_background(&self, request: DaemonRequest) -> InFlightRequest {
        let core = self.core();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(core.handle(request));
        });
        InFlightRequest {
            receiver,
            arrived: None,
        }
    }

    /// The saved state file's bytes, if the daemon has written one.
    pub fn saved_state_bytes(&self) -> Option<Vec<u8>> {
        std::fs::read(self.persistent_dir.path().join(LEGACY_STATE_FILE_NAME)).ok()
    }

    /// A file's bytes in the persistent directory, if it exists.
    pub fn persistent_file(&self, name: &str) -> Option<Vec<u8>> {
        std::fs::read(self.persistent_dir.path().join(name)).ok()
    }

    pub fn boot_path(&self, name: &str) -> PathBuf {
        self.boot_dir.path().join(name)
    }

    pub fn persistent_path(&self, name: &str) -> PathBuf {
        self.persistent_dir.path().join(name)
    }

    /// Where the absolute host path lies in the fake host root.
    pub fn host_path(&self, path: &str) -> PathBuf {
        self.host_root
            .path()
            .join(path.strip_prefix('/').expect("an absolute host path"))
    }

    /// Writes a file at the absolute host path, creating its directories.
    pub fn write_host_file(&self, path: &str, contents: &str) {
        let path = self.host_path(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    /// Installs a Steam app in the user's own Steam library.
    pub fn install_steam_app(&self, username: &str, app_id: u32, name: &str) {
        let root = format!("/home/{username}/.local/share/Steam");
        self.write_host_file(
            &format!("{root}/steamapps/libraryfolders.vdf"),
            &format!(
                "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{root}\"\n\t}}\n}}\n"
            ),
        );
        self.write_host_file(
            &format!("{root}/steamapps/appmanifest_{app_id}.acf"),
            &format!(
                "\"AppState\"\n{{\n\t\"appid\"\t\t\"{app_id}\"\n\t\"name\"\t\t\"{name}\"\n\t\"StateFlags\"\t\t\"4\"\n\t\"installdir\"\t\t\"{name}\"\n}}\n"
            ),
        );
        self.write_host_file(&format!("{root}/steamapps/common/{name}/game"), "");
    }

    /// Starts a fixture host process owned by the uid, with its start time in clock ticks.
    pub fn run_process(&self, pid: u32, uid: u32, start: u64, env: &str, cmd: &str) {
        self.write_host_file(&format!("/proc/{pid}/environ"), env);
        self.write_host_file(&format!("/proc/{pid}/cmdline"), cmd);
        self.write_host_file(
            &format!("/proc/{pid}/status"),
            &format!("Name:\tgame\nUid:\t{uid}\t0\t0\t0\n"),
        );
        // Field 22 is starttime, in clock ticks since boot. comm can contain spaces and ')'.
        self.write_host_file(
            &format!("/proc/{pid}/stat"),
            &format!("{pid} (game ) name) S 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 {start} 0\n"),
        );
    }

    /// Starts a native Steam game's process.
    pub fn run_steam_game(&self, pid: u32, uid: u32, start: u64, app_id: u32) {
        self.run_process(pid, uid, start, &format!("SteamAppId={app_id}\0"), "game\0");
    }

    /// Ends the fixture process, as an exit or a crash would.
    pub fn end_process(&self, pid: u32) {
        std::fs::remove_dir_all(self.host_path(&format!("/proc/{pid}"))).unwrap();
    }

    /// Runs one complete process scan and waits for the automatic operations it queued.
    pub fn scan(&self) {
        self.core().scan_running_games();
        self.settle();
    }

    /// Waits until every GPU's operation queue has run what was queued so far.
    pub fn settle(&self) {
        let core = self.core();
        for gpu in self.gpus.gpus() {
            for id in std::iter::once(gpu.id).chain(gpu.stable_id) {
                core.queues.submit(&id, || ()).wait();
            }
        }
    }
}

/// A user's session connection to the daemon.
pub struct FakeSession {
    core: Arc<DaemonCore>,
    session: DaemonSession,
}

impl FakeSession {
    pub fn send(&self, request: DaemonRequest) -> DaemonResponse {
        self.core.handle_session(&self.session, request)
    }

    /// Sends a request built from protocol JSON and returns the response as protocol JSON.
    pub fn send_json(&self, request: serde_json::Value) -> serde_json::Value {
        let request = serde_json::from_value(request).unwrap();
        serde_json::to_value(self.send(request)).unwrap()
    }

    /// The events pushed to this session so far, as protocol JSON, oldest first.
    pub fn pushed_events(&self) -> Vec<serde_json::Value> {
        let mut events = vec![];
        while let Ok(event) = self.session.recv_timeout(Duration::from_millis(50)) {
            events.push(serde_json::to_value(event).unwrap());
        }
        events
    }
}

pub struct InFlightRequest {
    receiver: Receiver<DaemonResponse>,
    arrived: Option<DaemonResponse>,
}

impl InFlightRequest {
    pub fn response(self) -> DaemonResponse {
        self.arrived.unwrap_or_else(|| {
            self.receiver
                .recv_timeout(RESPONSE_DEADLINE)
                .expect("the daemon did not respond in time")
        })
    }

    /// Whether the request is still unanswered after waiting briefly.
    pub fn is_pending(&mut self) -> bool {
        if self.arrived.is_none() {
            match self.receiver.recv_timeout(Duration::from_millis(200)) {
                Ok(response) => self.arrived = Some(response),
                Err(RecvTimeoutError::Timeout) => return true,
                Err(RecvTimeoutError::Disconnected) => panic!("the request was dropped"),
            }
        }
        false
    }
}
