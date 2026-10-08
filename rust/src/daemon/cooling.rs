//! The daemon's cooling controller: the only owner of applied cooling configurations and the
//! only executor of fan curves. Every operation on a GPU's fans runs on that GPU's operation
//! queue; the controller's lock is held only between hardware calls.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use super::backend::DaemonBackend;
use super::host::Clock;
use super::profiles::verified_gpu;
use super::tuning_store::write_atomically;
use crate::cooling::{
    ApplyCoolingRequest, CoolingApplication, CoolingConfiguration, CoolingControl, CurveSuspension,
    FanConfiguration, FanOutcome, FanRequest, FanRestoration, FanSetting, RestorationStatus,
};
use crate::fan_curve::{self, FanCurvePoint};
use crate::gpu::{
    FanControlMode, GpuPerformanceDevice, SetGpuFanModeAutoRequest, SetGpuFanSpeedRequest,
};
use crate::protocol::{TuningOutcome, TuningOutcomeStatus};

/// How often the daemon follows each applied curve.
pub const EVALUATION_INTERVAL_MS: u64 = 500;
/// How long demand must stay below a curve's current speed before the speed drops.
pub const DECREASE_DELAY_MS: u64 = 5_000;

pub(super) const COOLING_RECORD_FILE_NAME: &str = "cooling-control.json";
const RECORD_SCHEMA_VERSION: u64 = 1;

pub(super) const STOPPED_REASON: &str =
    "The Prism daemon stopped, so it stopped following this curve.";
pub(super) const STOPPED_UNEXPECTEDLY_REASON: &str =
    "The Prism daemon stopped unexpectedly while following this curve.";

pub(super) struct CoolingController {
    gpus: Mutex<HashMap<String, GpuCooling>>,
    record_path: PathBuf,
    boot_id: String,
    clock: Arc<dyn Clock>,
    /// When the curve loop last ran, so a supervisor outside it can tell it has stalled.
    loop_heartbeat_ms: AtomicU64,
    /// Once stopping, curves write nothing more.
    stopping: AtomicBool,
}

/// What Prism applied to one GPU's fans, by fan index.
#[derive(Debug, Clone)]
struct GpuCooling {
    shared: bool,
    fans: BTreeMap<u32, FanState>,
    curve_temperature_c: Option<i64>,
    suspension: Option<CurveSuspension>,
    /// When a curve on this GPU was last followed, or started.
    last_followed_ms: u64,
    evaluating: bool,
}

#[derive(Debug, Clone)]
enum FanState {
    Automatic,
    Fixed(i64),
    /// A curve; without an output, its control is suspended.
    Curve {
        points: Vec<FanCurvePoint>,
        output: Option<CurveOutput>,
    },
}

/// The speed a running curve requests, and since when demand has stayed below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CurveOutput {
    speed_percent: i64,
    lower_since_ms: Option<u64>,
}

impl CurveOutput {
    fn start(demand: i64) -> Self {
        Self {
            speed_percent: demand,
            lower_since_ms: None,
        }
    }

    /// Follows the demand: a rise applies at once, and a drop only once demand has stayed lower
    /// for the whole delay, at the demand of that moment. Returns the speed to request, if it
    /// changed.
    fn follow(&mut self, demand: i64, now_ms: u64) -> Option<i64> {
        if demand >= self.speed_percent {
            self.lower_since_ms = None;
            if demand == self.speed_percent {
                return None;
            }
        } else {
            let since = *self.lower_since_ms.get_or_insert(now_ms);
            if now_ms.saturating_sub(since) < DECREASE_DELAY_MS {
                return None;
            }
            self.lower_since_ms = None;
        }
        self.speed_percent = demand;
        Some(demand)
    }
}

impl GpuCooling {
    fn configuration(&self) -> Option<CoolingConfiguration> {
        let setting = |state: &FanState| match state {
            FanState::Automatic => FanSetting::Automatic {},
            FanState::Fixed(speed_percent) => FanSetting::Fixed {
                speed_percent: *speed_percent,
            },
            FanState::Curve { points, .. } => FanSetting::Curve {
                points: points.clone(),
            },
        };
        if self.shared {
            let first = self.fans.values().next()?;
            return Some(CoolingConfiguration::Shared {
                setting: setting(first),
            });
        }
        Some(CoolingConfiguration::PerFan {
            fans: self
                .fans
                .iter()
                .map(|(fan_index, state)| FanConfiguration {
                    fan_index: *fan_index,
                    setting: setting(state),
                })
                .collect(),
        })
    }

    fn running_curve_fans(&self) -> Vec<u32> {
        self.fans
            .iter()
            .filter(|(_, state)| {
                matches!(
                    state,
                    FanState::Curve {
                        output: Some(_),
                        ..
                    }
                )
            })
            .map(|(fan_index, _)| *fan_index)
            .collect()
    }

    fn control(&self) -> CoolingControl {
        let requested: Vec<_> = self
            .fans
            .iter()
            .filter_map(|(fan_index, state)| match state {
                FanState::Curve {
                    output: Some(output),
                    ..
                } => Some(FanRequest {
                    fan_index: *fan_index,
                    speed_percent: output.speed_percent,
                }),
                _ => None,
            })
            .collect();
        CoolingControl {
            applied: self.configuration(),
            curve_temperature_c: (!requested.is_empty())
                .then_some(self.curve_temperature_c)
                .flatten(),
            requested,
            suspension: self.suspension.clone(),
        }
    }
}

impl CoolingController {
    /// Opens the controller on the boot-scoped record of curves an earlier daemon followed.
    pub fn open(boot_dir: &Path, boot_id: &str, clock: Arc<dyn Clock>) -> Self {
        let record_path = boot_dir.join(COOLING_RECORD_FILE_NAME);
        let gpus = read_record(&record_path, boot_id)
            .into_iter()
            .map(|(uuid, recorded)| (uuid, recorded.restore()))
            .collect();
        Self {
            gpus: Mutex::new(gpus),
            record_path,
            boot_id: boot_id.into(),
            loop_heartbeat_ms: AtomicU64::new(clock.now_monotonic_ms()),
            clock,
            stopping: AtomicBool::new(false),
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, GpuCooling>> {
        self.gpus.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Prism's control of the GPU's fans, as reported in performance readings.
    pub fn control(&self, uuid: &str) -> CoolingControl {
        self.lock()
            .get(uuid)
            .map(GpuCooling::control)
            .unwrap_or_default()
    }

    /// Validates the configuration against fresh readings, then applies it. A curve starts at the
    /// speed it requests at the current temperature.
    pub fn apply(
        &self,
        backend: &dyn DaemonBackend,
        request: ApplyCoolingRequest,
    ) -> CoolingApplication {
        let performance = backend.performance();
        let Ok(gpu) = verified_gpu(&performance, &request.gpu_id) else {
            return unattempted(
                &[],
                "The GPU is absent or its stable identity could not be verified. Refresh the GPU readings and try again.",
            );
        };
        let shared = matches!(request.configuration, CoolingConfiguration::Shared { .. });
        let targets: Vec<(u32, FanSetting)> = match request.configuration {
            CoolingConfiguration::Shared { setting } => gpu
                .cooling
                .fans
                .iter()
                .map(|fan| (fan.index, setting.clone()))
                .collect(),
            CoolingConfiguration::PerFan { fans } => fans
                .into_iter()
                .map(|fan| (fan.fan_index, fan.setting))
                .collect(),
        };
        let indexes: Vec<u32> = targets.iter().map(|(index, _)| *index).collect();
        let problems = problems(gpu, shared, &targets);
        if !problems.is_empty() {
            return unattempted(&indexes, &problems.join("\n"));
        }
        let temperature = gpu.telemetry.temperature_c.map(i64::from);
        let now = self.clock.now_monotonic_ms();
        let mut failure = None;
        let mut fans = Vec::new();
        let mut states = Vec::new();
        for (fan_index, setting) in targets {
            if failure.is_some() {
                fans.push(fan_outcome(
                    fan_index,
                    TuningOutcomeStatus::Unattempted,
                    Some("An earlier fan failed. Retry or discard this change.".into()),
                ));
                continue;
            }
            let (state, result) = match setting {
                FanSetting::Automatic {} => (
                    FanState::Automatic,
                    backend.set_fan_mode_auto(SetGpuFanModeAutoRequest {
                        gpu_id: gpu.id.clone(),
                        fan_index,
                    }),
                ),
                FanSetting::Fixed { speed_percent } => (
                    FanState::Fixed(speed_percent),
                    set_speed(backend, gpu, fan_index, speed_percent),
                ),
                FanSetting::Curve { points } => {
                    let demand = fan_curve::requested_speed(&points, temperature.unwrap_or(0));
                    (
                        FanState::Curve {
                            points,
                            output: Some(CurveOutput::start(demand)),
                        },
                        set_speed(backend, gpu, fan_index, demand),
                    )
                }
            };
            match result {
                Ok(()) => fans.push(fan_outcome(fan_index, TuningOutcomeStatus::Succeeded, None)),
                Err(error) => {
                    failure = Some(format!("Fan {}: {error}", fan_index + 1));
                    fans.push(fan_outcome(
                        fan_index,
                        TuningOutcomeStatus::Failed,
                        Some(error),
                    ));
                }
            }
            states.push((fan_index, state));
        }
        let mut gpus = self.lock();
        let cooling = gpus
            .entry(request.gpu_id.clone())
            .or_insert_with(|| GpuCooling {
                shared,
                fans: BTreeMap::new(),
                curve_temperature_c: None,
                suspension: None,
                last_followed_ms: now,
                evaluating: false,
            });
        // A fan left unattempted keeps what it had, so a shared arrangement needs every fan.
        let every_fan = states.len() == fans.len();
        if shared && every_fan {
            cooling.fans.clear();
        }
        cooling.shared = shared && every_fan;
        cooling.fans.extend(states);
        cooling.curve_temperature_c = temperature;
        cooling.last_followed_ms = now;
        if !has_suspended_curve(cooling) {
            cooling.suspension = None;
        }
        drop(gpus);
        if let Some(reason) = failure {
            self.suspend(backend, &request.gpu_id, &reason);
        }
        self.save_record();
        CoolingApplication {
            outcome: TuningOutcome {
                status: if fans
                    .iter()
                    .all(|fan| fan.outcome.status == TuningOutcomeStatus::Succeeded)
                {
                    TuningOutcomeStatus::Succeeded
                } else {
                    TuningOutcomeStatus::Failed
                },
                error: None,
            },
            fans,
        }
    }

    /// Follows each running curve on the GPU from a fresh temperature reading.
    pub fn follow(&self, backend: &dyn DaemonBackend, uuid: &str) {
        let now = self.clock.now_monotonic_ms();
        let Some(mut cooling) = self.lock().get(uuid).cloned() else {
            return;
        };
        let running = cooling.running_curve_fans();
        if running.is_empty() || self.stopping.load(Ordering::SeqCst) {
            self.finish_following(uuid, None);
            return;
        }
        let performance = backend.performance();
        let Ok(gpu) = verified_gpu(&performance, uuid) else {
            self.finish_following(uuid, None);
            return self.suspend(
                backend,
                uuid,
                "The GPU is no longer reported, so Prism stopped following its curve.",
            );
        };
        let Some(temperature) = gpu.telemetry.temperature_c.map(i64::from) else {
            self.finish_following(uuid, None);
            return self.suspend(
                backend,
                uuid,
                "The GPU's temperature is unavailable, so Prism stopped following its curve.",
            );
        };
        let mut failure = None;
        for fan_index in running {
            let Some(FanState::Curve {
                points,
                output: Some(output),
            }) = cooling.fans.get_mut(&fan_index)
            else {
                continue;
            };
            let demand = fan_curve::requested_speed(points, temperature);
            let Some(speed) = output.follow(demand, now) else {
                continue;
            };
            if self.stopping.load(Ordering::SeqCst) {
                return;
            }
            if let Err(error) = set_speed(backend, gpu, fan_index, speed) {
                failure = Some(format!(
                    "Fan {} couldn't be set to {speed}%: {error}",
                    fan_index + 1
                ));
                break;
            }
        }
        cooling.curve_temperature_c = Some(temperature);
        cooling.last_followed_ms = now;
        self.finish_following(uuid, Some(cooling));
        if let Some(reason) = failure {
            self.suspend(backend, uuid, &reason);
        }
    }

    fn finish_following(&self, uuid: &str, followed: Option<GpuCooling>) {
        let mut gpus = self.lock();
        let Some(current) = gpus.get_mut(uuid) else {
            return;
        };
        if let Some(mut followed) = followed {
            followed.evaluating = false;
            *current = followed;
        } else {
            current.evaluating = false;
        }
    }

    /// Stops following the GPU's running curves, keeps them, and tries to return their fans to the
    /// GPU's default automatic cooling, reporting what happened to each fan.
    fn suspend(&self, backend: &dyn DaemonBackend, uuid: &str, reason: &str) {
        let fans = {
            let mut gpus = self.lock();
            let Some(cooling) = gpus.get_mut(uuid) else {
                return;
            };
            let fans = cooling.running_curve_fans();
            for fan_index in &fans {
                if let Some(FanState::Curve { output, .. }) = cooling.fans.get_mut(fan_index) {
                    *output = None;
                }
            }
            fans
        };
        if fans.is_empty() {
            return;
        }
        let restoration = restore_automatic(backend, uuid, &fans);
        if let Some(cooling) = self.lock().get_mut(uuid) {
            cooling.suspension = Some(CurveSuspension {
                reason: reason.into(),
                restoration,
            });
        }
        self.save_record();
    }

    /// The GPUs with a running curve that isn't already being followed, now marked as being
    /// followed. The loop that calls this proves it is alive.
    pub fn claim_due(&self) -> Vec<String> {
        self.loop_heartbeat_ms
            .store(self.clock.now_monotonic_ms(), Ordering::SeqCst);
        let mut gpus = self.lock();
        gpus.iter_mut()
            .filter(|(_, cooling)| !cooling.evaluating && !cooling.running_curve_fans().is_empty())
            .map(|(uuid, cooling)| {
                cooling.evaluating = true;
                uuid.clone()
            })
            .collect()
    }

    /// Whether the curve loop and every running curve have made progress within the window.
    pub fn is_live(&self, window_ms: u64) -> bool {
        let now = self.clock.now_monotonic_ms();
        let fresh = |at: u64| now.saturating_sub(at) <= window_ms;
        let gpus = self.lock();
        let running = gpus
            .values()
            .filter(|cooling| !cooling.running_curve_fans().is_empty());
        let mut running = running.peekable();
        running.peek().is_none()
            || (fresh(self.loop_heartbeat_ms.load(Ordering::SeqCst))
                && running.all(|cooling| fresh(cooling.last_followed_ms)))
    }

    /// The GPUs with a running curve.
    pub fn running_gpus(&self) -> Vec<String> {
        self.lock()
            .iter()
            .filter(|(_, cooling)| !cooling.running_curve_fans().is_empty())
            .map(|(uuid, _)| uuid.clone())
            .collect()
    }

    /// Stops all curve writes and returns each GPU's curve fans to automatic cooling.
    pub fn stop(&self, backend: &dyn DaemonBackend, uuid: &str) {
        self.stopping.store(true, Ordering::SeqCst);
        self.suspend(backend, uuid, STOPPED_REASON);
    }

    pub fn begin_stopping(&self) {
        self.stopping.store(true, Ordering::SeqCst);
    }

    fn save_record(&self) {
        if self.boot_id.is_empty() {
            return;
        }
        let gpus = self
            .lock()
            .iter()
            .filter_map(|(uuid, cooling)| {
                Some((
                    uuid.clone(),
                    RecordedGpu {
                        configuration: cooling.configuration()?,
                        fans: cooling.fans.keys().copied().collect(),
                        curve_fans: cooling.running_curve_fans(),
                        suspension: cooling.suspension.clone(),
                    },
                ))
            })
            .collect();
        let record = CoolingRecord {
            schema_version: RECORD_SCHEMA_VERSION,
            boot_id: self.boot_id.clone(),
            gpus,
        };
        if let Err(error) = write_atomically(
            &self.record_path,
            &serde_json::to_vec_pretty(&record).unwrap(),
        ) {
            eprintln!("failed to record Prism's fan curve control: {error}");
        }
    }
}

fn has_suspended_curve(cooling: &GpuCooling) -> bool {
    cooling
        .fans
        .values()
        .any(|state| matches!(state, FanState::Curve { output: None, .. }))
}

/// Why the configuration can't be applied to the GPU now; none means it can.
fn problems(
    gpu: &GpuPerformanceDevice,
    shared: bool,
    targets: &[(u32, FanSetting)],
) -> Vec<String> {
    let cooling = &gpu.cooling;
    if !cooling.is_supported || cooling.fans.is_empty() {
        return vec![format!(
            "Fan control is unavailable. {}",
            cooling
                .unsupported_reason
                .as_deref()
                .unwrap_or("This GPU reports no controllable fans.")
        )];
    }
    if targets.is_empty() {
        return vec!["Choose at least one fan to configure.".into()];
    }
    let mut problems = Vec::new();
    let mut seen = Vec::new();
    for (fan_index, _) in targets {
        if seen.contains(fan_index) {
            problems.push(format!(
                "Fan {} is configured more than once.",
                fan_index + 1
            ));
        } else if !cooling.fans.iter().any(|fan| fan.index == *fan_index) {
            problems.push(format!("Fan {} isn't reported by this GPU.", fan_index + 1));
        }
        seen.push(*fan_index);
    }
    if !problems.is_empty() {
        return problems;
    }
    let curve_problems = |indexes: &[u32], points: &[FanCurvePoint]| {
        fan_curve::validate_for(cooling, indexes, points)
            .into_iter()
            .map(|problem| problem.message)
    };
    let all: Vec<u32> = targets.iter().map(|(index, _)| *index).collect();
    if let (true, Some((_, FanSetting::Curve { points }))) = (shared, targets.first()) {
        problems.extend(curve_problems(&all, points));
    }
    for (fan_index, setting) in targets {
        match setting {
            FanSetting::Automatic {} => {}
            FanSetting::Fixed { speed_percent } => {
                let fan = cooling
                    .fans
                    .iter()
                    .find(|fan| fan.index == *fan_index)
                    .unwrap();
                let min = fan.min_speed_percent.map(i64::from).unwrap_or(0);
                let max = fan.max_speed_percent.map(i64::from).unwrap_or(100);
                if !(min..=max).contains(speed_percent) {
                    problems.push(format!(
                        "Fan {}: speed must be between {min}% and {max}%.",
                        fan_index + 1
                    ));
                }
            }
            FanSetting::Curve { .. } if shared => {}
            FanSetting::Curve { points } => problems.extend(
                curve_problems(&[*fan_index], points)
                    .map(|message| format!("Fan {}: {message}", fan_index + 1)),
            ),
        }
    }
    let has_curve = targets
        .iter()
        .any(|(_, setting)| matches!(setting, FanSetting::Curve { .. }));
    if has_curve && gpu.telemetry.temperature_c.is_none() {
        problems.push(
            "The GPU's temperature is unavailable, so a curve can't be followed. Refresh the GPU readings and try again."
                .into(),
        );
    }
    problems
}

fn set_speed(
    backend: &dyn DaemonBackend,
    gpu: &GpuPerformanceDevice,
    fan_index: u32,
    speed_percent: i64,
) -> Result<(), String> {
    backend.set_fan_speed(SetGpuFanSpeedRequest {
        gpu_id: gpu.id.clone(),
        fan_index,
        speed_percent: speed_percent as u32,
    })
}

/// Asks the driver to return each fan to the GPU's default automatic cooling, then reads back
/// which fans report automatic control. A request the driver accepted is not proof on its own.
pub(super) fn restore_automatic(
    backend: &dyn DaemonBackend,
    uuid: &str,
    fans: &[u32],
) -> Vec<FanRestoration> {
    let gpu_id = verified_gpu(&backend.performance(), uuid)
        .map(|gpu| gpu.id.clone())
        .ok();
    let results: Vec<_> = fans
        .iter()
        .map(|&fan_index| match &gpu_id {
            Some(gpu_id) => backend.set_fan_mode_auto(SetGpuFanModeAutoRequest {
                gpu_id: gpu_id.clone(),
                fan_index,
            }),
            None => Err("The GPU is no longer reported.".into()),
        })
        .collect();
    let performance = backend.performance();
    let readback = verified_gpu(&performance, uuid).ok();
    fans.iter()
        .zip(results)
        .map(|(&fan_index, result)| {
            let (status, detail) = match result {
                Err(error) => (RestorationStatus::Failed, Some(error)),
                Ok(()) => match readback
                    .and_then(|gpu| gpu.cooling.fans.iter().find(|fan| fan.index == fan_index))
                    .and_then(|fan| fan.mode)
                {
                    Some(FanControlMode::Auto) => (RestorationStatus::Restored, None),
                    Some(FanControlMode::Manual) => (
                        RestorationStatus::Unverified,
                        Some("The driver accepted the request but still reports manual control.".into()),
                    ),
                    None => (
                        RestorationStatus::Unverified,
                        Some("The driver accepted the request but doesn't report the fan's control mode.".into()),
                    ),
                },
            };
            FanRestoration {
                fan_index,
                status,
                detail,
            }
        })
        .collect()
}

fn fan_outcome(fan_index: u32, status: TuningOutcomeStatus, error: Option<String>) -> FanOutcome {
    FanOutcome {
        fan_index,
        outcome: TuningOutcome { status, error },
    }
}

fn unattempted(fans: &[u32], message: &str) -> CoolingApplication {
    CoolingApplication {
        outcome: TuningOutcome {
            status: TuningOutcomeStatus::Unattempted,
            error: Some(message.into()),
        },
        fans: fans
            .iter()
            .map(|&fan_index| {
                fan_outcome(
                    fan_index,
                    TuningOutcomeStatus::Unattempted,
                    Some(message.into()),
                )
            })
            .collect(),
    }
}

/// The boot-scoped record of the curves Prism follows, so whoever runs after an unexpected stop
/// knows which fans to return to automatic cooling, and which suspensions to keep reporting.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CoolingRecord {
    schema_version: u64,
    boot_id: String,
    gpus: BTreeMap<String, RecordedGpu>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecordedGpu {
    configuration: CoolingConfiguration,
    /// Every fan the configuration covers.
    fans: Vec<u32>,
    /// Fans whose curve was being followed: no one has returned them to automatic cooling yet.
    curve_fans: Vec<u32>,
    suspension: Option<CurveSuspension>,
}

impl RecordedGpu {
    /// The applied configuration, with every curve suspended: a curve never resumes by itself.
    fn restore(self) -> GpuCooling {
        let state = |setting: FanSetting| match setting {
            FanSetting::Automatic {} => FanState::Automatic,
            FanSetting::Fixed { speed_percent } => FanState::Fixed(speed_percent),
            FanSetting::Curve { points } => FanState::Curve {
                points,
                output: None,
            },
        };
        let (shared, fans) = match self.configuration {
            CoolingConfiguration::Shared { setting } => (
                true,
                self.fans
                    .iter()
                    .map(|&fan_index| (fan_index, state(setting.clone())))
                    .collect(),
            ),
            CoolingConfiguration::PerFan { fans } => (
                false,
                fans.into_iter()
                    .map(|fan| (fan.fan_index, state(fan.setting)))
                    .collect(),
            ),
        };
        GpuCooling {
            shared,
            fans,
            curve_temperature_c: None,
            suspension: self.suspension,
            last_followed_ms: 0,
            evaluating: false,
        }
    }
}

fn read_record(path: &Path, boot_id: &str) -> BTreeMap<String, RecordedGpu> {
    let Ok(bytes) = fs::read(path) else {
        return BTreeMap::new();
    };
    match serde_json::from_slice::<CoolingRecord>(&bytes) {
        Ok(record)
            if record.schema_version == RECORD_SCHEMA_VERSION
                && record.boot_id == boot_id
                && !boot_id.is_empty() =>
        {
            record.gpus
        }
        Ok(_) => BTreeMap::new(),
        Err(error) => {
            eprintln!("ignoring an unreadable record of Prism's fan curve control: {error}");
            BTreeMap::new()
        }
    }
}

/// Returns fans whose curve an earlier daemon was following to automatic cooling, and records
/// the outcome as that curve's suspension. Runs at daemon start and from the service manager after
/// the daemon stops, without needing the daemon or its curve loop.
pub fn recover_abandoned_curves(backend: &dyn DaemonBackend, boot_dir: &Path, boot_id: &str) {
    let path = boot_dir.join(COOLING_RECORD_FILE_NAME);
    let mut gpus = read_record(&path, boot_id);
    if gpus.values().all(|gpu| gpu.curve_fans.is_empty()) {
        return;
    }
    for (uuid, gpu) in &mut gpus {
        if gpu.curve_fans.is_empty() {
            continue;
        }
        let fans = std::mem::take(&mut gpu.curve_fans);
        let restoration = restore_automatic(backend, uuid, &fans);
        gpu.suspension = Some(CurveSuspension {
            reason: STOPPED_UNEXPECTEDLY_REASON.into(),
            restoration,
        });
    }
    let record = CoolingRecord {
        schema_version: RECORD_SCHEMA_VERSION,
        boot_id: boot_id.into(),
        gpus,
    };
    if let Err(error) = write_atomically(&path, &serde_json::to_vec_pretty(&record).unwrap()) {
        eprintln!("failed to record the recovery of Prism's fan curves: {error}");
    }
}
