//! The daemon's cooling controller: the only owner of applied cooling configurations and the
//! only executor of fan curves. Every operation on a GPU's fans runs on that GPU's operation
//! queue; the controller's lock is held only between hardware calls.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use super::SessionPeer;
use super::backend::DaemonBackend;
use super::host::Clock;
use super::profiles::verified_gpu;
use super::tuning_store::{TuningStore, write_atomically};
use prism_gpu::cooling::{
    ApplyCoolingRequest, CoolingApplication, CoolingConfiguration, CoolingControl, CoolingRecovery,
    CurveSuspension, FanConfiguration, FanOutcome, FanRequest, FanRestoration, FanSetting,
    RecoveryStatus, RestorationStatus, SuspensionKind,
};
use prism_gpu::fan_curve::{self, FanCurvePoint};
use prism_gpu::{
    FanControlMode, GpuPerformanceDevice, SetGpuFanModeAutoRequest, SetGpuFanSpeedRequest,
    TuningOutcome, TuningOutcomeStatus,
};
use prism_protocol::{DaemonEvent, GpuCoolingControl};

/// How often the daemon follows each applied curve.
pub const EVALUATION_INTERVAL_MS: u64 = 500;
/// How long demand must stay below a curve's current speed before the speed drops.
pub const DECREASE_DELAY_MS: u64 = 5_000;

pub(super) const COOLING_RECORD_FILE_NAME: &str = "cooling-control.json";
const RECORD_SCHEMA_VERSION: u64 = 1;

const STALLED_REASON: &str =
    "Prism's fan curve control stopped responding, so the daemon stopped following this curve.";
const NOT_RESUMED_REASON: &str = "After the Prism daemon restarted, the curve couldn't resume.";

pub(super) struct CoolingController {
    tuning_store: Arc<TuningStore>,
    gpus: Mutex<HashMap<String, GpuCooling>>,
    record_path: PathBuf,
    boot_id: String,
    clock: Arc<dyn Clock>,
    /// When the curve loop last ran, so a supervisor outside it can tell it has stalled.
    loop_heartbeat_ms: AtomicU64,
    /// Once stopping, curves write nothing more.
    stopping: AtomicBool,
    /// Whether curve control has stopped making progress, so curves it leaves behind are faults
    /// that the next daemon must not resume.
    stalled: AtomicBool,
    /// When each GPU's current fan operation started, so a hung driver call counts as a stall
    /// wherever it happens.
    operations: Mutex<HashMap<String, u64>>,
}

/// All controller callers declare why they apply a configuration. Only an identified manual
/// application (live or profile) replaces that user's remembered choice. Automatic applications,
/// restorations and calls without an identified user never acquire ownership of that choice.
pub(super) enum CoolingApplicationSource {
    Manual(SessionPeer),
    /// Legacy manual callers update host manual state without claiming a user's remembered curve.
    UnidentifiedManual,
    NonManual,
}

/// What Prism applied to one GPU's fans, by fan index.
#[derive(Debug, Clone)]
struct GpuCooling {
    revision: u64,
    shared: bool,
    fans: BTreeMap<u32, FanState>,
    curve_temperature_c: Option<i64>,
    suspension: Option<CurveSuspension>,
    /// When a curve on this GPU was last followed, or started.
    last_followed_ms: u64,
    /// Whether the curve loop has queued an evaluation that hasn't finished.
    follow_queued: bool,
    /// Fans that may still be at a curve's speed although no curve runs on them: being written by
    /// an application, or released while their GPU's queue was stuck. They stay recorded for
    /// whoever recovers after the daemon stops.
    unreleased: BTreeSet<u32>,
    /// Curves an earlier daemon of this boot was following, healthy, when it stopped.
    interrupted: Option<InterruptedCurves>,
    recovery: Option<CoolingRecovery>,
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

impl FanState {
    fn new(setting: FanSetting, output: Option<CurveOutput>) -> Self {
        match setting {
            FanSetting::Automatic {} => Self::Automatic,
            FanSetting::Fixed { speed_percent } => Self::Fixed(speed_percent),
            FanSetting::Curve { points } => Self::Curve { points, output },
        }
    }
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
    fn new(shared: bool, now_ms: u64) -> Self {
        Self {
            revision: 0,
            shared,
            fans: BTreeMap::new(),
            curve_temperature_c: None,
            suspension: None,
            last_followed_ms: now_ms,
            follow_queued: false,
            unreleased: BTreeSet::new(),
            interrupted: None,
            recovery: None,
        }
    }

    fn configuration(&self) -> Option<CoolingConfiguration> {
        if self.fans.is_empty() {
            return None;
        }
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

    fn has_running_curve(&self) -> bool {
        !self.running_curve_fans().is_empty()
    }

    /// Fans no one has returned from a curve's speed yet.
    fn owned_curve_fans(&self) -> Vec<u32> {
        let mut fans: BTreeSet<u32> = self.running_curve_fans().into_iter().collect();
        fans.extend(&self.unreleased);
        fans.into_iter().collect()
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
            remembered_curve: None,
            revision: self.revision,
            applied: self.configuration(),
            curve_temperature_c: (!requested.is_empty())
                .then_some(self.curve_temperature_c)
                .flatten(),
            requested,
            suspension: self.suspension.clone(),
            recovery: self.recovery.clone(),
        }
    }

    /// Stops following every running curve, returning the fans that were following one.
    fn release_running(&mut self) -> Vec<u32> {
        let fans = self.running_curve_fans();
        for fan_index in &fans {
            if let Some(FanState::Curve { output, .. }) = self.fans.get_mut(fan_index) {
                *output = None;
            }
        }
        fans
    }
}

impl CoolingController {
    /// Opens the controller on the boot-scoped record of curves an earlier daemon followed.
    pub fn open(
        boot_dir: &Path,
        boot_id: &str,
        clock: Arc<dyn Clock>,
        tuning_store: Arc<TuningStore>,
    ) -> Self {
        let record_path = boot_dir.join(COOLING_RECORD_FILE_NAME);
        let gpus = read_record(&record_path, boot_id)
            .gpus
            .into_iter()
            .map(|(uuid, recorded)| (uuid, recorded.into_suspended()))
            .collect();
        Self {
            tuning_store,
            gpus: Mutex::new(gpus),
            record_path,
            boot_id: boot_id.into(),
            loop_heartbeat_ms: AtomicU64::new(clock.now_monotonic_ms()),
            clock,
            stopping: AtomicBool::new(false),
            stalled: AtomicBool::new(false),
            operations: Mutex::default(),
        }
    }

    /// Marks the GPU as inside a fan operation until the returned guard drops.
    fn operating(&self, uuid: &str) -> Operation<'_> {
        self.operations
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(uuid.into(), self.clock.now_monotonic_ms());
        Operation {
            controller: self,
            uuid: uuid.into(),
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

    /// The revision of Prism's control of the GPU's fans, which changes with it.
    pub fn revision(&self, uuid: &str) -> u64 {
        self.lock().get(uuid).map_or(0, |cooling| cooling.revision)
    }

    /// Pushes Prism's control of the GPU's fans to every session, each with its user's remembered
    /// curve for the GPU.
    pub fn publish(&self, uuid: &str) {
        let control = self.control(uuid);
        // Read before publishing, so no store lock is taken while the sessions' is held.
        let remembered: HashMap<(u32, String), CoolingConfiguration> = self
            .tuning_store
            .read(|document| {
                document
                    .users
                    .iter()
                    .filter_map(|user| {
                        let curve = user.remembered_curves.get(uuid)?.clone();
                        Some(((user.uid, user.username.clone()), curve))
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.tuning_store.publish_for_users(|peer| {
            let mut control = control.clone();
            control.remembered_curve = remembered.get(&(peer.uid, peer.username.clone())).cloned();
            Some(DaemonEvent::CoolingControlChanged(GpuCoolingControl::of(
                uuid, control,
            )))
        });
    }

    /// Validates the configuration against fresh readings, then applies it. A curve starts at the
    /// speed it requests at the current temperature.
    pub fn apply(
        &self,
        backend: &dyn DaemonBackend,
        request: ApplyCoolingRequest,
        source: CoolingApplicationSource,
    ) -> CoolingApplication {
        let uuid = request.gpu_id.clone();
        let custom_curve = request.configuration.contains_curve();
        let mut application = self.apply_configuration(backend, request);
        let control = self.control(&uuid);
        application.revision = control.revision;
        if custom_curve
            && application.outcome.status == TuningOutcomeStatus::Succeeded
            && let (CoolingApplicationSource::Manual(peer), Some(configuration)) =
                (source, control.applied)
            && let Err(error) = self.tuning_store.update(|document| {
                document
                    .user_mut(peer.uid, &peer.username)
                    .remembered_curves
                    .insert(uuid, configuration);
            })
        {
            // Hardware success stays success. Store health reports the persistence failure.
            eprintln!("Could not remember the manually applied fan curve: {error}");
        }
        application
    }

    fn apply_configuration(
        &self,
        backend: &dyn DaemonBackend,
        request: ApplyCoolingRequest,
    ) -> CoolingApplication {
        let _operation = self.operating(&request.gpu_id);
        let performance = backend.performance();
        let Ok(gpu) = verified_gpu(&performance, &request.gpu_id) else {
            return unattempted(
                &[],
                "The GPU is absent or its stable identity could not be verified. Refresh the GPU readings and try again.",
            );
        };
        if let Some(expected) = &request.fan_indexes {
            let actual: BTreeSet<_> = gpu.cooling.fans.iter().map(|fan| fan.index).collect();
            let reviewed: BTreeSet<_> = expected.iter().copied().collect();
            if actual != reviewed || reviewed.len() != expected.len() {
                return unattempted(
                    expected,
                    "The targeted fans changed. Review the current fan inventory before applying.",
                );
            }
        }
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
        let problems = application_problems(gpu, shared, &targets);
        if !problems.is_empty() {
            return unattempted(&indexes, &problems.join("\n"));
        }
        let temperature = gpu.telemetry.temperature_c.map(i64::from);
        let now = self.clock.now_monotonic_ms();
        // Curve fans are recorded before they are written, so a crash mid-application still
        // leaves them for recovery.
        let curve_fans: Vec<u32> = targets
            .iter()
            .filter(|(_, setting)| matches!(setting, FanSetting::Curve { .. }))
            .map(|(index, _)| *index)
            .collect();
        if !curve_fans.is_empty() {
            self.lock()
                .entry(request.gpu_id.clone())
                .or_insert_with(|| GpuCooling::new(shared, now))
                .unreleased
                .extend(&curve_fans);
            self.save_record();
        }
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
            let (output, result) = match &setting {
                FanSetting::Automatic {} => (
                    None,
                    backend.set_fan_mode_auto(SetGpuFanModeAutoRequest {
                        gpu_id: gpu.id.clone(),
                        fan_index,
                    }),
                ),
                FanSetting::Fixed { speed_percent } => {
                    (None, set_speed(backend, gpu, fan_index, *speed_percent))
                }
                FanSetting::Curve { points } => {
                    let demand = fan_curve::requested_speed(points, temperature.unwrap_or(0));
                    (
                        Some(CurveOutput::start(demand)),
                        set_speed(backend, gpu, fan_index, demand),
                    )
                }
            };
            let state = FanState::new(setting, output);
            match result {
                Ok(()) => {
                    fans.push(fan_outcome(fan_index, TuningOutcomeStatus::Succeeded, None));
                    states.push((fan_index, state));
                }
                Err(error) => {
                    // Keep a requested curve for explicit retry, visibly suspended below.
                    // A rejected automatic/fixed command must not replace the applied setting.
                    if matches!(state, FanState::Curve { .. }) {
                        states.push((fan_index, state));
                    }
                    failure = Some(format!("Fan {}: {error}", fan_index + 1));
                    fans.push(fan_outcome(
                        fan_index,
                        TuningOutcomeStatus::Failed,
                        Some(error),
                    ));
                }
            }
        }
        let mut gpus = self.lock();
        let cooling = gpus
            .entry(request.gpu_id.clone())
            .or_insert_with(|| GpuCooling::new(shared, now));
        for fan_index in &curve_fans {
            cooling.unreleased.remove(fan_index);
        }
        // A fan left unattempted keeps what it had, so a shared arrangement needs every fan.
        let every_fan = states.len() == fans.len();
        if shared && every_fan {
            cooling.fans.clear();
        }
        cooling.revision += 1;
        cooling.shared = shared && every_fan;
        cooling.fans.extend(states);
        cooling.curve_temperature_c = temperature;
        cooling.last_followed_ms = now;
        cooling.interrupted = None;
        cooling.recovery = None;
        if !has_suspended_curve(cooling) {
            cooling.suspension = None;
        }
        drop(gpus);
        if let Some(reason) = failure {
            // A partly applied fixed/automatic configuration needs the same fallback as curves.
            self.suspend_fans(backend, &request.gpu_id, &reason, &indexes);
        }
        self.save_record();
        CoolingApplication {
            revision: 0,
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
        let _operation = self.operating(uuid);
        let now = self.clock.now_monotonic_ms();
        let Some(mut cooling) = self.lock().get(uuid).cloned() else {
            return;
        };
        if cooling.interrupted.is_some() {
            self.resume_interrupted(backend, uuid);
            self.finish_following(uuid, None);
            return;
        }
        let running = cooling.running_curve_fans();
        if running.is_empty() || self.stopping.load(Ordering::SeqCst) {
            self.finish_following(uuid, None);
            return;
        }
        let performance = backend.performance();
        let Ok(gpu) = verified_gpu(&performance, uuid) else {
            self.finish_following(uuid, None);
            // A missing GPU isn't a curve fault. Keep the healthy configuration for the
            // same UUID's return, when resumption validates fresh fans, limits and temperature.
            return self.interrupt(backend, uuid);
        };
        if self.changed_outside_prism(gpu) {
            return;
        }
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

    /// Checks only observable controls, never physical speed or routine curve outputs.
    /// Called by automatic switching before writes as well as by the curve loop.
    pub fn changed_outside_prism(&self, gpu: &GpuPerformanceDevice) -> bool {
        let uuid = &gpu.id;
        let Some(cooling) = self.lock().get(uuid).cloned() else {
            return false;
        };
        if cooling
            .suspension
            .as_ref()
            .is_some_and(|s| s.kind == SuspensionKind::ExternalControl)
        {
            return true;
        }
        // The backend exposes policy, but no independent manual target readback. A speed
        // observation (including spin-up lag) or unknown policy cannot prove another writer.
        if cooling.fans.iter().any(|(index, state)| {
            // A fault's own fallback is controller activity, not a competing external edit.
            let restoration = cooling
                .suspension
                .as_ref()
                .and_then(|s| s.restoration.iter().find(|fan| fan.fan_index == *index));
            let expected = if let Some(restoration) = restoration {
                if restoration.status != RestorationStatus::Restored {
                    return false;
                }
                FanControlMode::Auto
            } else {
                match state {
                    FanState::Automatic => FanControlMode::Auto,
                    FanState::Fixed(_)
                    | FanState::Curve {
                        output: Some(_), ..
                    } => FanControlMode::Manual,
                    FanState::Curve { output: None, .. } => return false,
                }
            };
            gpu.cooling
                .fans
                .iter()
                .any(|fan| fan.index == *index && fan.mode.is_some_and(|mode| mode != expected))
        }) {
            self.yield_control(uuid);
            return true;
        }
        false
    }

    /// Relinquishes the whole GPU without issuing restoration writes. The boot record must no
    /// longer claim these fans, so shutdown and restart also leave the other controls alone.
    fn yield_control(&self, uuid: &str) {
        if let Some(cooling) = self.lock().get_mut(uuid) {
            for state in cooling.fans.values_mut() {
                if let FanState::Curve { output, .. } = state {
                    *output = None;
                }
            }
            cooling.unreleased.clear();
            cooling.follow_queued = false;
            cooling.curve_temperature_c = None;
            cooling.recovery = None;
            cooling.revision += 1;
            cooling.suspension = Some(CurveSuspension {
                kind: SuspensionKind::ExternalControl,
                reason: "The driver reports a fan control mode different from Prism's applied controls. Prism left those controls alone.".into(),
                restoration: vec![],
            });
        }
        self.save_record();
    }

    /// Keeps the speeds an evaluation requested, for fans whose curve still runs: a stop may have
    /// released them meanwhile.
    fn finish_following(&self, uuid: &str, followed: Option<GpuCooling>) {
        let mut gpus = self.lock();
        let Some(current) = gpus.get_mut(uuid) else {
            return;
        };
        current.follow_queued = false;
        let Some(followed) = followed else {
            return;
        };
        if self.stopping.load(Ordering::SeqCst) {
            return;
        }
        for (fan_index, state) in &mut current.fans {
            if let (
                FanState::Curve {
                    output: Some(output),
                    ..
                },
                Some(FanState::Curve {
                    output: Some(evaluated),
                    ..
                }),
            ) = (state, followed.fans.get(fan_index))
            {
                *output = *evaluated;
            }
        }
        current.curve_temperature_c = followed.curve_temperature_c;
        current.last_followed_ms = followed.last_followed_ms;
    }

    /// Stops following the GPU's running curves, keeps them, and tries to return their fans to the
    /// GPU's default automatic cooling, reporting what happened to each fan.
    fn suspend(&self, backend: &dyn DaemonBackend, uuid: &str, reason: &str) {
        self.suspend_fans(backend, uuid, reason, &[]);
    }

    /// Application faults include every targeted fan, even when no curve was running.
    fn suspend_fans(
        &self,
        backend: &dyn DaemonBackend,
        uuid: &str,
        reason: &str,
        affected: &[u32],
    ) {
        let _operation = self.operating(uuid);
        let mut fans: BTreeSet<_> = self
            .lock()
            .get_mut(uuid)
            .map(GpuCooling::release_running)
            .unwrap_or_default()
            .into_iter()
            .collect();
        fans.extend(affected);
        let fans: Vec<_> = fans.into_iter().collect();
        if fans.is_empty() {
            return;
        }
        let restoration = restore_automatic(backend, uuid, &fans);
        if let Some(cooling) = self.lock().get_mut(uuid) {
            cooling.revision += 1;
            cooling.recovery = None;
            cooling.suspension = Some(CurveSuspension {
                kind: SuspensionKind::Fault,
                reason: reason.into(),
                restoration,
            });
        }
        self.save_record();
    }

    /// Stops following the GPU's running curves because the daemon stops, and returns their fans
    /// to automatic cooling. The curves were healthy, so the next daemon of this boot may resume
    /// them.
    fn interrupt(&self, backend: &dyn DaemonBackend, uuid: &str) {
        let _operation = self.operating(uuid);
        let fans = self
            .lock()
            .get_mut(uuid)
            .map(GpuCooling::release_running)
            .unwrap_or_default();
        if fans.is_empty() {
            return;
        }
        let restoration = restore_automatic(backend, uuid, &fans);
        if let Some(cooling) = self.lock().get_mut(uuid) {
            cooling.revision += 1;
            cooling
                .interrupted
                .get_or_insert_with(InterruptedCurves::default)
                .add(restoration);
        }
        self.save_record();
    }

    /// The GPUs with curves an earlier daemon of this boot was following when it stopped.
    pub fn interrupted_gpus(&self) -> Vec<String> {
        self.lock()
            .iter()
            .filter(|(_, cooling)| cooling.interrupted.is_some())
            .map(|(uuid, _)| uuid.clone())
            .collect()
    }

    /// Resumes the curves an earlier daemon of this boot was following, healthy, when it stopped,
    /// once fresh readings show the same GPU and fans, limits the curve still fits, and the
    /// current temperature. Each curve starts at the speed it requests at that temperature. A
    /// curve that can't resume returns its fans to automatic cooling and waits for a retry.
    pub fn resume_interrupted(&self, backend: &dyn DaemonBackend, uuid: &str) {
        let _operation = self.operating(uuid);
        if self.stopping.load(Ordering::SeqCst) {
            return;
        }
        let Some(cooling) = self.lock().get(uuid).cloned() else {
            return;
        };
        let Some(interrupted) = cooling.interrupted.clone() else {
            return;
        };
        let fans = interrupted.fans();
        let performance = backend.performance();
        let Ok(gpu) = verified_gpu(&performance, uuid) else {
            // Neither a replacement in its slot nor temporary absence consumes recovery.
            return;
        };
        let targets = match resumption(gpu, &cooling, &interrupted) {
            Ok(targets) => targets,
            Err(NotResumable::Problems(problems)) => {
                return self.not_resumed(backend, uuid, &fans, &problems.join(" "));
            }
            Err(NotResumable::ExternalControl(reason)) => {
                return self.leave_to_external_control(uuid, &fans, &reason);
            }
        };
        // Validation rejected a missing temperature.
        let temperature = gpu.telemetry.temperature_c.map(i64::from).unwrap_or(0);
        // Recorded before they are written, so whoever runs after a crash mid-resumption returns
        // them to automatic cooling.
        if let Some(cooling) = self.lock().get_mut(uuid) {
            cooling.unreleased.extend(&fans);
        }
        self.save_record();
        let mut outcomes = Vec::new();
        let mut outputs = Vec::new();
        let mut failure = None;
        for (fan_index, points) in targets {
            if failure.is_some() {
                outcomes.push(fan_outcome(
                    fan_index,
                    TuningOutcomeStatus::Unattempted,
                    Some("An earlier fan failed. Retry the curve.".into()),
                ));
                continue;
            }
            let demand = fan_curve::requested_speed(&points, temperature);
            outputs.push((fan_index, CurveOutput::start(demand)));
            match set_speed(backend, gpu, fan_index, demand) {
                Ok(()) => {
                    outcomes.push(fan_outcome(fan_index, TuningOutcomeStatus::Succeeded, None))
                }
                Err(error) => {
                    failure = Some(format!("Fan {}: {error}", fan_index + 1));
                    outcomes.push(fan_outcome(
                        fan_index,
                        TuningOutcomeStatus::Failed,
                        Some(error),
                    ));
                }
            }
        }
        let now = self.clock.now_monotonic_ms();
        if let Some(cooling) = self.lock().get_mut(uuid) {
            for fan_index in &fans {
                cooling.unreleased.remove(fan_index);
            }
            for (fan_index, started) in outputs {
                if let Some(FanState::Curve { output, .. }) = cooling.fans.get_mut(&fan_index) {
                    *output = Some(started);
                }
            }
            if failure.is_none() {
                cooling.revision += 1;
                cooling.interrupted = None;
                cooling.curve_temperature_c = Some(temperature);
                cooling.last_followed_ms = now;
                cooling.recovery = Some(CoolingRecovery {
                    status: RecoveryStatus::Resumed,
                    fans: outcomes.clone(),
                });
                if !has_suspended_curve(cooling) {
                    cooling.suspension = None;
                }
            }
        }
        match failure {
            Some(reason) => self.fail_resumption(backend, uuid, &fans, &reason, outcomes),
            None => self.save_record(),
        }
    }

    /// Reports interrupted curves that fresh readings rule out, before any curve write.
    fn not_resumed(&self, backend: &dyn DaemonBackend, uuid: &str, fans: &[u32], reason: &str) {
        self.fail_resumption(backend, uuid, fans, reason, unattempted(fans, reason).fans);
    }

    /// Returns the interrupted curves' fans to automatic cooling and suspends the curves until an
    /// explicit retry, reporting each fan's resumption and restoration.
    fn fail_resumption(
        &self,
        backend: &dyn DaemonBackend,
        uuid: &str,
        fans: &[u32],
        reason: &str,
        outcomes: Vec<FanOutcome>,
    ) {
        if let Some(cooling) = self.lock().get_mut(uuid) {
            cooling.release_running();
        }
        let restoration = restore_automatic(backend, uuid, fans);
        if let Some(cooling) = self.lock().get_mut(uuid) {
            cooling.revision += 1;
            cooling.interrupted = None;
            cooling.curve_temperature_c = None;
            cooling.suspension = Some(CurveSuspension {
                kind: SuspensionKind::Fault,
                reason: format!("{NOT_RESUMED_REASON} {reason}"),
                restoration,
            });
            cooling.recovery = Some(CoolingRecovery {
                status: RecoveryStatus::RetryRequired,
                fans: outcomes,
            });
        }
        self.save_record();
    }

    /// Leaves fans another tool took over while no daemon ran to that tool, writing nothing.
    fn leave_to_external_control(&self, uuid: &str, fans: &[u32], reason: &str) {
        if let Some(cooling) = self.lock().get_mut(uuid) {
            cooling.revision += 1;
            cooling.interrupted = None;
            cooling.suspension = Some(CurveSuspension {
                kind: SuspensionKind::ExternalControl,
                reason: reason.into(),
                restoration: vec![],
            });
            cooling.recovery = Some(CoolingRecovery {
                status: RecoveryStatus::RetryRequired,
                fans: unattempted(fans, reason).fans,
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
            .filter(|(_, cooling)| {
                !cooling.follow_queued
                    && (cooling.has_running_curve() || cooling.interrupted.is_some())
            })
            .map(|(uuid, cooling)| {
                cooling.follow_queued = true;
                uuid.clone()
            })
            .collect()
    }

    /// Whether the curve loop, every running curve and every fan operation have made progress
    /// within the window.
    pub fn is_live(&self, window_ms: u64) -> bool {
        let now = self.clock.now_monotonic_ms();
        let fresh = |at: u64| now.saturating_sub(at) <= window_ms;
        let operations_fresh = self
            .operations
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .all(|started| fresh(*started));
        let gpus = self.lock();
        let mut running = gpus
            .values()
            .filter(|cooling| cooling.has_running_curve())
            .peekable();
        operations_fresh
            && (running.peek().is_none()
                || (fresh(self.loop_heartbeat_ms.load(Ordering::SeqCst))
                    && running.all(|cooling| fresh(cooling.last_followed_ms))))
    }

    /// The GPUs with a running curve, or one still resuming, that a stop must wait for.
    pub fn gpus_to_stop(&self) -> Vec<String> {
        self.lock()
            .iter()
            .filter(|(_, cooling)| cooling.has_running_curve() || cooling.interrupted.is_some())
            .map(|(uuid, _)| uuid.clone())
            .collect()
    }

    /// Stops following the GPU's curves and returns its curve fans to automatic cooling, for the
    /// next daemon of this boot to resume. Runs on the GPU's queue, so no curve write can follow
    /// it.
    pub fn stop_following(&self, backend: &dyn DaemonBackend, uuid: &str) {
        self.stopping.store(true, Ordering::SeqCst);
        self.interrupt(backend, uuid);
    }

    /// Like `stop_following`, for a GPU whose queue is stuck: a hung curve write may still land
    /// after this restoration, so the fans stay recorded to be returned to automatic cooling once
    /// the daemon has gone, as a stall the next daemon doesn't resume.
    pub fn stop_following_stuck(&self, backend: &dyn DaemonBackend, uuid: &str) {
        let fans = self
            .lock()
            .get(uuid)
            .map(GpuCooling::running_curve_fans)
            .unwrap_or_default();
        self.stalled.store(true, Ordering::SeqCst);
        self.stop_following(backend, uuid);
        if let Some(cooling) = self.lock().get_mut(uuid) {
            cooling.unreleased.extend(fans);
        }
        self.save_record();
    }

    pub fn begin_stopping(&self) {
        self.stopping.store(true, Ordering::SeqCst);
    }

    /// Records whether curve control has stalled, as judged by the supervisor outside the curve
    /// loop. While it has, curves the daemon leaves behind, however it stops, are faults that the
    /// next daemon doesn't resume.
    pub fn record_stall(&self, stalled: bool) {
        if self.stalled.swap(stalled, Ordering::SeqCst) != stalled {
            self.save_record();
        }
    }

    fn save_record(&self) {
        let gpus = self
            .lock()
            .iter()
            .map(|(uuid, cooling)| {
                (
                    uuid.clone(),
                    RecordedGpu {
                        revision: cooling.revision,
                        configuration: cooling.configuration(),
                        fans: cooling.fans.keys().copied().collect(),
                        curve_fans: cooling.owned_curve_fans(),
                        suspension: cooling.suspension.clone(),
                        interrupted: cooling.interrupted.clone(),
                        recovery: cooling.recovery.clone(),
                    },
                )
            })
            .collect();
        write_record(
            &self.record_path,
            &self.boot_id,
            gpus,
            self.stalled.load(Ordering::SeqCst),
        );
    }
}

/// Ends the GPU's fan operation for the stall supervisor.
struct Operation<'a> {
    controller: &'a CoolingController,
    uuid: String,
}

impl Drop for Operation<'_> {
    fn drop(&mut self) {
        self.controller
            .operations
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.uuid);
    }
}

fn has_suspended_curve(cooling: &GpuCooling) -> bool {
    cooling
        .fans
        .values()
        .any(|state| matches!(state, FanState::Curve { output: None, .. }))
}

enum NotResumable {
    Problems(Vec<String>),
    /// Another tool took manual control of a fan after the stopping daemon returned it to
    /// automatic cooling.
    ExternalControl(String),
}

/// Each interrupted fan's curve, once fresh readings show it can resume: the same fans, no other
/// tool controlling them, limits the curve still fits and a current temperature.
fn resumption(
    gpu: &GpuPerformanceDevice,
    cooling: &GpuCooling,
    interrupted: &InterruptedCurves,
) -> Result<Vec<(u32, Vec<FanCurvePoint>)>, NotResumable> {
    let reported: BTreeSet<u32> = gpu.cooling.fans.iter().map(|fan| fan.index).collect();
    let configured: BTreeSet<u32> = cooling.fans.keys().copied().collect();
    let fans_match = if cooling.shared {
        reported == configured
    } else {
        configured.is_subset(&reported)
    };
    if !fans_match {
        return Err(NotResumable::Problems(vec![
            "The GPU's fans changed since the curve was applied.".into(),
        ]));
    }
    let mut targets = Vec::new();
    for fan_index in interrupted.fans() {
        let Some(FanState::Curve { points, .. }) = cooling.fans.get(&fan_index) else {
            return Err(NotResumable::Problems(vec![format!(
                "Prism's record of fan {} has no curve to resume.",
                fan_index + 1
            )]));
        };
        targets.push((fan_index, points.clone()));
    }
    for restoration in &interrupted.restoration {
        let mode = gpu
            .cooling
            .fans
            .iter()
            .find(|fan| fan.index == restoration.fan_index)
            .and_then(|fan| fan.mode);
        if restoration.status == RestorationStatus::Restored && mode == Some(FanControlMode::Manual)
        {
            return Err(NotResumable::ExternalControl(format!(
                "While the Prism daemon was stopped, fan {} came under manual control from elsewhere. Prism left those controls alone.",
                restoration.fan_index + 1
            )));
        }
    }
    let settings: Vec<(u32, FanSetting)> = targets
        .iter()
        .map(|(fan_index, points)| {
            (
                *fan_index,
                FanSetting::Curve {
                    points: points.clone(),
                },
            )
        })
        .collect();
    let problems = application_problems(gpu, cooling.shared, &settings);
    if !problems.is_empty() {
        return Err(NotResumable::Problems(problems));
    }
    Ok(targets)
}

/// Why the configuration can't be applied to the GPU now; none means it can.
pub(super) fn application_problems(
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
                let (min, max) = fan.speed_range();
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
        revision: 0,
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

/// The boot-scoped record of the cooling Prism applied and the curves it follows, so whoever runs
/// after the daemon stops knows which fans to return to automatic cooling, which curves the next
/// daemon of this boot may resume, and which suspensions to keep reporting.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CoolingRecord {
    schema_version: u64,
    boot_id: String,
    gpus: BTreeMap<String, RecordedGpu>,
    /// Curve control had stalled: curves left behind are faults, not interruptions.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    stalled: bool,
}

impl CoolingRecord {
    fn empty(boot_id: &str) -> Self {
        Self {
            schema_version: RECORD_SCHEMA_VERSION,
            boot_id: boot_id.into(),
            gpus: BTreeMap::new(),
            stalled: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordedGpu {
    #[serde(default)]
    revision: u64,
    /// None until a first application has finished.
    configuration: Option<CoolingConfiguration>,
    /// Every fan the configuration covers.
    fans: Vec<u32>,
    /// Fans whose curve was being followed: no one has returned them to automatic cooling yet.
    curve_fans: Vec<u32>,
    suspension: Option<CurveSuspension>,
    // Omitted when empty, so an earlier daemon version can still read the record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    interrupted: Option<InterruptedCurves>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recovery: Option<CoolingRecovery>,
}

/// Curve fans that were following their curve, healthy, when the daemon stopped, and how each
/// returned to automatic cooling meanwhile.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InterruptedCurves {
    restoration: Vec<FanRestoration>,
}

impl InterruptedCurves {
    fn fans(&self) -> Vec<u32> {
        self.restoration.iter().map(|fan| fan.fan_index).collect()
    }

    /// Adds fans that have just been returned to automatic cooling, replacing older outcomes.
    fn add(&mut self, restoration: Vec<FanRestoration>) {
        for fan in restoration {
            self.restoration
                .retain(|known| known.fan_index != fan.fan_index);
            self.restoration.push(fan);
        }
        self.restoration.sort_by_key(|fan| fan.fan_index);
    }
}

impl RecordedGpu {
    /// The applied configuration, with every curve suspended: a curve never resumes by itself.
    fn into_suspended(self) -> GpuCooling {
        let state = |setting: FanSetting| FanState::new(setting, None);
        let (shared, fans) = match self.configuration {
            None => (false, BTreeMap::new()),
            Some(CoolingConfiguration::Shared { setting }) => (
                true,
                self.fans
                    .iter()
                    .map(|&fan_index| (fan_index, state(setting.clone())))
                    .collect(),
            ),
            Some(CoolingConfiguration::PerFan { fans }) => (
                false,
                fans.into_iter()
                    .map(|fan| (fan.fan_index, state(fan.setting)))
                    .collect(),
            ),
        };
        GpuCooling {
            revision: self.revision,
            fans,
            suspension: self.suspension,
            interrupted: self.interrupted,
            recovery: self.recovery,
            ..GpuCooling::new(shared, 0)
        }
    }
}

/// This boot's record. A previous boot's, or one that can't be read, holds nothing: it never
/// becomes a configuration to resume.
fn read_record(path: &Path, boot_id: &str) -> CoolingRecord {
    let Ok(bytes) = fs::read(path) else {
        return CoolingRecord::empty(boot_id);
    };
    match serde_json::from_slice::<CoolingRecord>(&bytes) {
        Ok(record)
            if record.schema_version == RECORD_SCHEMA_VERSION
                && record.boot_id == boot_id
                && !boot_id.is_empty() =>
        {
            record
        }
        Ok(_) => CoolingRecord::empty(boot_id),
        Err(error) => {
            eprintln!("ignoring an unreadable record of Prism's fan curve control: {error}");
            CoolingRecord::empty(boot_id)
        }
    }
}

/// Returns fans whose curve an earlier daemon was following to automatic cooling. A healthy curve
/// is recorded as interrupted, for the next daemon of this boot to resume; if curve control had
/// stalled, every curve the daemon left behind is suspended as a fault instead. Runs at daemon
/// start and from the service manager after the daemon stops, without needing the daemon or its
/// curve loop.
pub fn recover_abandoned_curves(backend: &dyn DaemonBackend, boot_dir: &Path, boot_id: &str) {
    let path = boot_dir.join(COOLING_RECORD_FILE_NAME);
    let mut record = read_record(&path, boot_id);
    if !record.stalled && record.gpus.values().all(|gpu| gpu.curve_fans.is_empty()) {
        return;
    }
    for (uuid, gpu) in &mut record.gpus {
        let fans = std::mem::take(&mut gpu.curve_fans);
        let restoration = if fans.is_empty() {
            vec![]
        } else {
            restore_automatic(backend, uuid, &fans)
        };
        if record.stalled {
            let mut stopped = gpu.interrupted.take().unwrap_or_default();
            stopped.add(restoration);
            if stopped.restoration.is_empty() {
                continue;
            }
            gpu.revision += 1;
            gpu.recovery = None;
            gpu.suspension = Some(CurveSuspension {
                kind: SuspensionKind::Fault,
                reason: STALLED_REASON.into(),
                restoration: stopped.restoration,
            });
        } else if !restoration.is_empty() {
            gpu.revision += 1;
            gpu.interrupted
                .get_or_insert_with(InterruptedCurves::default)
                .add(restoration);
        }
    }
    write_record(&path, boot_id, record.gpus, false);
}

/// Records the curves for this boot. Without a boot id, nothing could tell a later reader which
/// boot the record belongs to, so none is kept.
fn write_record(path: &Path, boot_id: &str, gpus: BTreeMap<String, RecordedGpu>, stalled: bool) {
    if boot_id.is_empty() {
        return;
    }
    let record = CoolingRecord {
        gpus,
        stalled,
        ..CoolingRecord::empty(boot_id)
    };
    if let Err(error) = write_atomically(path, &serde_json::to_vec_pretty(&record).unwrap()) {
        eprintln!("failed to record Prism's fan curve control: {error}");
    }
}
