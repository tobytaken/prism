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

use super::{Clock, CoreDependencies, DaemonBackend, DaemonCore, HostShutdownSignal};
use crate::drivers::DriverService;
use crate::gpu::{
    ApplyGpuConfigRequest, ControlSupport, FanControlMode, GpuCommandError, GpuCommandErrorCode,
    GpuInventoryResponse, GpuPerformanceDevice, GpuPerformanceResponse, GpuTargetRequest,
    PersistedGpuState, SetGpuClockOffsetsRequest, SetGpuFanModeAutoRequest, SetGpuFanSpeedRequest,
    SetGpuPowerLimitRequest,
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
    ApplyConfig,
    ResetConfig,
}

/// A stateful fake of the host's GPUs. Setters change the values that performance reads report.
#[derive(Default)]
pub struct FakeGpuBackend {
    state: Mutex<FakeGpus>,
}

#[derive(Default)]
struct FakeGpus {
    gpus: Vec<GpuPerformanceDevice>,
    failures: HashMap<(String, Setter), String>,
    gates: HashMap<(String, Setter), Gate>,
    writes: Vec<(String, Setter)>,
    applied_configs: Vec<ApplyGpuConfigRequest>,
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

    /// Every setter call so far, in the order the calls started.
    pub fn writes(&self) -> Vec<(String, Setter)> {
        self.state.lock().unwrap().writes.clone()
    }

    pub fn applied_configs(&self) -> Vec<ApplyGpuConfigRequest> {
        self.state.lock().unwrap().applied_configs.clone()
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
    fn inventory(&self, _persisted_state: &PersistedGpuState) -> GpuInventoryResponse {
        GpuInventoryResponse {
            gpus: vec![],
            warnings: vec![],
        }
    }

    fn performance(&self) -> GpuPerformanceResponse {
        GpuPerformanceResponse {
            gpus: self.gpus(),
            warnings: vec![],
        }
    }

    fn apply(&self, request: &ApplyGpuConfigRequest) -> Result<(), GpuCommandError> {
        let id = request.gpu_id.clone().unwrap_or_default();
        self.write(&id, Setter::ApplyConfig, |gpu| {
            if let Some(limit) = request.power_limit_watts {
                gpu.power.current_limit_watts = Some(limit);
            }
            if let Some(offset) = request.graphics_offset_mhz {
                gpu.clocks.graphics.current_offset_mhz = Some(offset);
            }
            if let Some(offset) = request.memory_offset_mhz {
                gpu.clocks.memory.current_offset_mhz = Some(offset);
            }
        })
        .map_err(|message| GpuCommandError::new(GpuCommandErrorCode::Internal, message))?;
        self.state
            .lock()
            .unwrap()
            .applied_configs
            .push(request.clone());
        Ok(())
    }

    fn reset(&self, gpu_id: &str) -> Result<(), GpuCommandError> {
        self.write(gpu_id, Setter::ResetConfig, |gpu| {
            gpu.power.current_limit_watts = gpu.power.default_limit_watts;
            gpu.clocks.graphics.current_offset_mhz = Some(0);
            gpu.clocks.memory.current_offset_mhz = Some(0);
        })
        .map_err(|message| GpuCommandError::new(GpuCommandErrorCode::Internal, message))
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

pub struct Harness {
    pub gpus: Arc<FakeGpuBackend>,
    pub clock: Arc<FakeClock>,
    host_shutting_down: Arc<FakeHostShutdown>,
    persistent_dir: TempDir,
    boot_dir: TempDir,
    boot_id: String,
    core: Option<Arc<DaemonCore>>,
}

impl Harness {
    pub fn start(gpus: FakeGpuBackend) -> Self {
        Self::start_with_saved_state(gpus, None)
    }

    /// Starts the daemon on a persistent directory that already holds this saved state.
    pub fn start_with_saved_state(gpus: FakeGpuBackend, saved: Option<&[u8]>) -> Self {
        let persistent_dir = tempfile::tempdir().unwrap();
        if let Some(saved) = saved {
            std::fs::write(persistent_dir.path().join("state.json"), saved).unwrap();
        }
        let mut harness = Self {
            gpus: Arc::new(gpus),
            clock: Arc::default(),
            host_shutting_down: Arc::default(),
            persistent_dir,
            boot_dir: tempfile::tempdir().unwrap(),
            boot_id: "boot-1".into(),
            core: None,
        };
        harness.start_core();
        harness
    }

    /// Simulates a daemon restart: drops the core and starts a new one on the same directories.
    pub fn restart(&mut self) {
        self.core = None;
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
        })));
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
        std::fs::read(self.persistent_dir.path().join("state.json")).ok()
    }

    pub fn saved_state(&self) -> PersistedGpuState {
        serde_json::from_slice(&self.saved_state_bytes().expect("no saved state")).unwrap()
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
