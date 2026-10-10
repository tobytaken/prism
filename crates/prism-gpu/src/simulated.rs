//! Simulated GPUs, so Prism runs and is tested without NVIDIA hardware.
//!
//! Each named fixture is a fixed GPU inventory whose telemetry replays a scripted series, one
//! sample per second of an injected clock. The same fixture at the same time always reads the
//! same, so its readings are byte-stable across runs. Release builds leave this module out.

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use nvml_wrapper::error::NvmlError;

use crate::nvidia::{NvidiaProviderError, inventory_unavailable, performance_unavailable};
use crate::{
    ClockInfo, ClockLockControl, ClockOffsetInfo, ClockOffsetsInfo, ComputeApi, ComputeInfo,
    ControlSupport, CoolingInfo, FanControlInfo, FanControlMode, GpuControlInfo, GpuDevice,
    GpuInventoryProvider, GpuInventoryResponse, GpuPerformanceDevice, GpuPerformanceResponse,
    GpuSupportedFeatures, GpuTelemetryInfo, GpuTelemetryProvider, GpuVendor, MemoryInfo,
    OffsetControl, PcieInfo, PowerControlInfo, PowerInfo, PowerLimitControl,
};

/// How far the simulation has run, which picks the telemetry sample to replay.
pub trait SimulationClock: Send + Sync {
    fn elapsed(&self) -> Duration;
}

impl<F: Fn() -> Duration + Send + Sync> SimulationClock for F {
    fn elapsed(&self) -> Duration {
        self()
    }
}

/// A named host setup the simulated provider plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SimulatedFixture {
    /// One GPU whose power limit, clock offsets and two fans are all tunable.
    TunableGpu,
    /// Two GPUs of the same model, told apart only by their identity and readings.
    TwoIdenticalGpus,
    /// One GPU with tunable power and clocks but no fans Prism can control.
    NoFanControl,
    /// One GPU whose tuning values sit on the bounds of their ranges, and whose telemetry swings
    /// between the ends of each range.
    EdgeRanges,
    /// A host without NVML, so no GPU can be read.
    NvmlMissing,
    /// A host where NVML works but reports no GPUs.
    EmptyInventory,
    /// A host with NVML but without the NVIDIA driver loaded.
    DriverNotLoaded,
}

impl SimulatedFixture {
    pub const ALL: [Self; 7] = [
        Self::TunableGpu,
        Self::TwoIdenticalGpus,
        Self::NoFanControl,
        Self::EdgeRanges,
        Self::NvmlMissing,
        Self::EmptyInventory,
        Self::DriverNotLoaded,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::TunableGpu => "tunable",
            Self::TwoIdenticalGpus => "two-identical",
            Self::NoFanControl => "no-fan-control",
            Self::EdgeRanges => "edge-ranges",
            Self::NvmlMissing => "nvml-missing",
            Self::EmptyInventory => "empty-inventory",
            Self::DriverNotLoaded => "driver-not-loaded",
        }
    }

    fn gpus(self) -> &'static [SimulatedGpu] {
        match self {
            Self::TunableGpu => &[TUNABLE_GPU],
            Self::TwoIdenticalGpus => &IDENTICAL_GPUS,
            Self::NoFanControl => &[NO_FAN_CONTROL_GPU],
            Self::EdgeRanges => &[EDGE_RANGES_GPU],
            Self::NvmlMissing | Self::EmptyInventory | Self::DriverNotLoaded => &[],
        }
    }

    /// Why NVML can't be initialized on this host, as the NVML provider words it.
    fn nvml_failure(self) -> Option<String> {
        match self {
            Self::NvmlMissing => Some(
                "unable to initialize NVML: a libloading error occurred: libnvidia-ml.so: cannot \
                 open shared object file: No such file or directory"
                    .into(),
            ),
            Self::DriverNotLoaded => {
                Some(NvidiaProviderError::NvmlInit(NvmlError::DriverNotLoaded).to_string())
            }
            _ => None,
        }
    }
}

impl fmt::Display for SimulatedFixture {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

impl FromStr for SimulatedFixture {
    type Err = UnknownFixture;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|fixture| fixture.name() == name)
            .ok_or_else(|| UnknownFixture(name.into()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownFixture(String);

impl fmt::Display for UnknownFixture {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<_> = SimulatedFixture::ALL
            .iter()
            .map(|fixture| fixture.name())
            .collect();
        write!(
            formatter,
            "unknown simulated GPU fixture {:?}; expected one of: {}",
            self.0,
            names.join(", ")
        )
    }
}

impl std::error::Error for UnknownFixture {}

/// A GPU inventory and telemetry provider that plays a fixture against a clock.
pub struct SimulatedGpus {
    fixture: SimulatedFixture,
    clock: Box<dyn SimulationClock>,
}

impl SimulatedGpus {
    pub fn new(fixture: SimulatedFixture, clock: impl SimulationClock + 'static) -> Self {
        Self {
            fixture,
            clock: Box::new(clock),
        }
    }

    fn second(&self) -> u64 {
        self.clock.elapsed().as_secs()
    }
}

impl GpuInventoryProvider for SimulatedGpus {
    fn inventory(&self) -> GpuInventoryResponse {
        if let Some(failure) = self.fixture.nvml_failure() {
            return inventory_unavailable(&failure);
        }
        let second = self.second();
        GpuInventoryResponse {
            gpus: self
                .fixture
                .gpus()
                .iter()
                .map(|gpu| gpu.device(second))
                .collect(),
            warnings: Vec::new(),
        }
    }
}

impl GpuTelemetryProvider for SimulatedGpus {
    fn performance(&self) -> GpuPerformanceResponse {
        if let Some(failure) = self.fixture.nvml_failure() {
            return performance_unavailable(&failure);
        }
        let second = self.second();
        GpuPerformanceResponse {
            gpus: self
                .fixture
                .gpus()
                .iter()
                .map(|gpu| gpu.performance(second))
                .collect(),
            warnings: Vec::new(),
        }
    }
}

const DRIVER_VERSION: &str = "595.58.03";
const CUDA_VERSION: &str = "13.2";

/// GPU utilization in percent, one sample per second: idle, a load spike, then cooling down.
const GAMING_SESSION: [u32; 16] = [2, 4, 3, 38, 87, 97, 99, 98, 100, 96, 99, 72, 41, 12, 5, 3];
/// GPU utilization alternating between the ends of its range every second.
const IDLE_AND_FULL: [u32; 2] = [0, 100];

/// A simulated GPU's fixed hardware and tuning values, and how its readings follow the load.
struct SimulatedGpu {
    id: &'static str,
    name: &'static str,
    architecture: &'static str,
    bus_id: &'static str,
    firmware_version: &'static str,
    compute_capability: &'static str,
    core_count: u32,
    memory_total_mib: u64,
    graphics_max_mhz: u32,
    memory_max_mhz: u32,
    power_limit: PowerLimit,
    graphics_offset: Offset,
    memory_offset: Offset,
    fans: Option<Fans>,
    max_operating_temperature_c: u32,
    /// Utilization in percent replayed one sample per second, starting `phase` samples in.
    load: &'static [u32],
    phase: usize,
    temperature_c: Span,
    power_draw_watts: Span,
    core_clock_mhz: Span,
    memory_clock_mhz: Span,
    memory_used_mib: Span,
    fan_speed_percent: Span,
}

struct PowerLimit {
    min_watts: u32,
    max_watts: u32,
    default_watts: u32,
    current_watts: u32,
}

struct Offset {
    min_mhz: i32,
    max_mhz: i32,
    current_mhz: i32,
}

struct Fans {
    count: u32,
    min_speed_percent: u32,
    max_speed_percent: u32,
}

/// A reading's value from an idle GPU to one at full load.
struct Span {
    idle: u32,
    full: u32,
}

impl Span {
    fn at(&self, load_percent: u32) -> u32 {
        let (idle, full) = (i64::from(self.idle), i64::from(self.full));
        (idle + (full - idle) * i64::from(load_percent) / 100) as u32
    }
}

impl SimulatedGpu {
    fn load_at(&self, second: u64) -> u32 {
        let samples = self.load.len() as u64;
        self.load[((second % samples + self.phase as u64) % samples) as usize]
    }

    fn device(&self, second: u64) -> GpuDevice {
        let load = self.load_at(second);
        GpuDevice {
            id: self.id.into(),
            vendor: GpuVendor::Nvidia,
            name: self.name.into(),
            architecture: Some(self.architecture.into()),
            driver_version: Some(DRIVER_VERSION.into()),
            firmware_version: Some(self.firmware_version.into()),
            bus_id: Some(self.bus_id.into()),
            memory: MemoryInfo {
                total_mib: Some(self.memory_total_mib),
            },
            compute: ComputeInfo {
                api: Some(ComputeApi::Cuda),
                api_version: Some(CUDA_VERSION.into()),
                capability: Some(self.compute_capability.into()),
                core_count: Some(self.core_count),
            },
            pcie: PcieInfo {
                gen_max: Some(4),
                width_max: Some(16),
            },
            power: PowerInfo {
                enforced_limit_watts: Some(self.power_limit.current_watts),
                default_limit_watts: Some(self.power_limit.default_watts),
            },
            clocks: ClockInfo {
                graphics_current_mhz: Some(self.core_clock_mhz.at(load)),
                graphics_max_mhz: Some(self.graphics_max_mhz),
                sm_max_mhz: Some(self.graphics_max_mhz),
                memory_max_mhz: Some(self.memory_max_mhz),
            },
            control: GpuControlInfo {
                supported_features: GpuSupportedFeatures {
                    power_limit: true,
                    gpu_clock_lock: true,
                    memory_clock_lock: true,
                    graphics_offset: true,
                    memory_offset: true,
                    reset: true,
                },
                power_limit: PowerLimitControl {
                    current_watts: Some(self.power_limit.current_watts),
                    default_watts: Some(self.power_limit.default_watts),
                    min_watts: Some(self.power_limit.min_watts),
                    max_watts: Some(self.power_limit.max_watts),
                },
                gpu_clock_lock: ClockLockControl {
                    current_min_mhz: None,
                    current_max_mhz: None,
                    min_allowed_mhz: Some(self.core_clock_mhz.idle),
                    max_allowed_mhz: Some(self.graphics_max_mhz),
                    supported: true,
                },
                memory_clock_lock: ClockLockControl {
                    current_min_mhz: None,
                    current_max_mhz: None,
                    min_allowed_mhz: Some(self.memory_clock_mhz.idle),
                    max_allowed_mhz: Some(self.memory_max_mhz),
                    supported: true,
                },
                offsets: OffsetControl {
                    graphics_mhz: Some(self.graphics_offset.current_mhz),
                    memory_mhz: Some(self.memory_offset.current_mhz),
                    min_graphics_mhz: Some(self.graphics_offset.min_mhz),
                    max_graphics_mhz: Some(self.graphics_offset.max_mhz),
                    min_memory_mhz: Some(self.memory_offset.min_mhz),
                    max_memory_mhz: Some(self.memory_offset.max_mhz),
                },
                warnings: Vec::new(),
            },
        }
    }

    fn performance(&self, second: u64) -> GpuPerformanceDevice {
        let load = self.load_at(second);
        let fan_speed_percent = self.fan_speed_percent.at(load);
        let fans: Vec<_> = self
            .fans
            .iter()
            .flat_map(|fans| {
                (0..fans.count).map(|index| FanControlInfo {
                    index,
                    current_speed_percent: Some(fan_speed_percent),
                    min_speed_percent: Some(fans.min_speed_percent),
                    max_speed_percent: Some(fans.max_speed_percent),
                    mode: Some(FanControlMode::Auto),
                })
            })
            .collect();
        let power_draw_watts = self.power_draw_watts.at(load);

        GpuPerformanceDevice {
            id: self.id.into(),
            stable_id: Some(self.id.into()),
            vendor: GpuVendor::Nvidia,
            name: self.name.into(),
            architecture: Some(self.architecture.into()),
            bus_id: Some(self.bus_id.into()),
            power: PowerControlInfo {
                support: ControlSupport::Supported,
                reason: None,
                current_value_error: None,
                current_draw_watts: Some(power_draw_watts),
                current_limit_watts: Some(self.power_limit.current_watts),
                default_limit_watts: Some(self.power_limit.default_watts),
                enforced_limit_watts: Some(self.power_limit.current_watts),
                min_limit_watts: Some(self.power_limit.min_watts),
                max_limit_watts: Some(self.power_limit.max_watts),
            },
            cooling: CoolingInfo {
                is_supported: self.fans.is_some(),
                unsupported_reason: self
                    .fans
                    .is_none()
                    .then(|| "Fan control is not exposed by this GPU or driver.".into()),
                fan_count: fans.len() as u32,
                fans,
                max_operating_temperature_c: Some(self.max_operating_temperature_c),
                max_operating_temperature_reason: None,
                curve_support: None,
                control: None,
            },
            clocks: ClockOffsetsInfo {
                graphics: self.graphics_offset.info(),
                memory: self.memory_offset.info(),
            },
            telemetry: GpuTelemetryInfo {
                power_draw_watts: Some(power_draw_watts),
                core_clock_mhz: Some(self.core_clock_mhz.at(load)),
                memory_clock_mhz: Some(self.memory_clock_mhz.at(load)),
                fan_speed_percent: self.fans.as_ref().map(|_| fan_speed_percent),
                voltage_millivolts: None,
                temperature_c: Some(self.temperature_c.at(load)),
                gpu_utilization_percent: Some(load),
                memory_used_mib: Some(u64::from(self.memory_used_mib.at(load))),
                memory_total_mib: Some(self.memory_total_mib),
                performance_state: Some(if load < 10 { "P8" } else { "P0" }.into()),
            },
        }
    }
}

impl Offset {
    fn info(&self) -> ClockOffsetInfo {
        ClockOffsetInfo {
            support: ControlSupport::Supported,
            reason: None,
            current_value_error: None,
            current_offset_mhz: Some(self.current_mhz),
            min_offset_mhz: Some(self.min_mhz),
            max_offset_mhz: Some(self.max_mhz),
        }
    }
}

const TUNABLE_GPU: SimulatedGpu = SimulatedGpu {
    id: "GPU-5a1b0000-0000-4000-8000-000000000001",
    name: "NVIDIA GeForce RTX 5090",
    architecture: "Blackwell",
    bus_id: "00000000:01:00.0",
    firmware_version: "98.02.2E.00.01",
    compute_capability: "12.0",
    core_count: 21_760,
    memory_total_mib: 32_607,
    graphics_max_mhz: 3_090,
    memory_max_mhz: 14_001,
    power_limit: PowerLimit {
        min_watts: 400,
        max_watts: 600,
        default_watts: 575,
        current_watts: 575,
    },
    graphics_offset: Offset {
        min_mhz: -1_000,
        max_mhz: 1_000,
        current_mhz: 0,
    },
    memory_offset: Offset {
        min_mhz: -2_000,
        max_mhz: 6_000,
        current_mhz: 0,
    },
    fans: Some(Fans {
        count: 2,
        min_speed_percent: 30,
        max_speed_percent: 100,
    }),
    max_operating_temperature_c: 90,
    load: &GAMING_SESSION,
    phase: 0,
    temperature_c: Span { idle: 34, full: 78 },
    power_draw_watts: Span {
        idle: 22,
        full: 560,
    },
    core_clock_mhz: Span {
        idle: 210,
        full: 2_790,
    },
    memory_clock_mhz: Span {
        idle: 405,
        full: 14_001,
    },
    memory_used_mib: Span {
        idle: 512,
        full: 28_672,
    },
    fan_speed_percent: Span { idle: 30, full: 72 },
};

const fn identical_gpu(id: &'static str, bus_id: &'static str, phase: usize) -> SimulatedGpu {
    SimulatedGpu {
        id,
        name: "NVIDIA GeForce RTX 4090",
        architecture: "Ada Lovelace",
        bus_id,
        firmware_version: "95.02.3C.00.01",
        compute_capability: "8.9",
        core_count: 16_384,
        memory_total_mib: 24_564,
        graphics_max_mhz: 3_120,
        memory_max_mhz: 10_501,
        power_limit: PowerLimit {
            min_watts: 150,
            max_watts: 600,
            default_watts: 450,
            current_watts: 450,
        },
        graphics_offset: Offset {
            min_mhz: -1_000,
            max_mhz: 1_000,
            current_mhz: 0,
        },
        memory_offset: Offset {
            min_mhz: -2_000,
            max_mhz: 6_000,
            current_mhz: 0,
        },
        fans: Some(Fans {
            count: 2,
            min_speed_percent: 30,
            max_speed_percent: 100,
        }),
        max_operating_temperature_c: 90,
        load: &GAMING_SESSION,
        phase,
        temperature_c: Span { idle: 32, full: 74 },
        power_draw_watts: Span {
            idle: 18,
            full: 440,
        },
        core_clock_mhz: Span {
            idle: 210,
            full: 2_745,
        },
        memory_clock_mhz: Span {
            idle: 405,
            full: 10_501,
        },
        memory_used_mib: Span {
            idle: 384,
            full: 20_480,
        },
        fan_speed_percent: Span { idle: 30, full: 68 },
    }
}

/// The second GPU runs five seconds ahead of the first, so their readings differ.
const IDENTICAL_GPUS: [SimulatedGpu; 2] = [
    identical_gpu(
        "GPU-5a1b0000-0000-4000-8000-000000000002",
        "00000000:01:00.0",
        0,
    ),
    identical_gpu(
        "GPU-5a1b0000-0000-4000-8000-000000000003",
        "00000000:02:00.0",
        5,
    ),
];

const NO_FAN_CONTROL_GPU: SimulatedGpu = SimulatedGpu {
    id: "GPU-5a1b0000-0000-4000-8000-000000000004",
    name: "NVIDIA GeForce RTX 4060 Laptop GPU",
    architecture: "Ada Lovelace",
    bus_id: "00000000:01:00.0",
    firmware_version: "95.07.36.00.0D",
    compute_capability: "8.9",
    core_count: 3_072,
    memory_total_mib: 8_188,
    graphics_max_mhz: 3_105,
    memory_max_mhz: 8_001,
    power_limit: PowerLimit {
        min_watts: 35,
        max_watts: 115,
        default_watts: 80,
        current_watts: 80,
    },
    graphics_offset: Offset {
        min_mhz: -1_000,
        max_mhz: 1_000,
        current_mhz: 0,
    },
    memory_offset: Offset {
        min_mhz: -2_000,
        max_mhz: 6_000,
        current_mhz: 0,
    },
    fans: None,
    max_operating_temperature_c: 87,
    load: &GAMING_SESSION,
    phase: 0,
    temperature_c: Span { idle: 41, full: 83 },
    power_draw_watts: Span { idle: 4, full: 78 },
    core_clock_mhz: Span {
        idle: 210,
        full: 2_370,
    },
    memory_clock_mhz: Span {
        idle: 405,
        full: 8_001,
    },
    memory_used_mib: Span {
        idle: 256,
        full: 6_144,
    },
    fan_speed_percent: Span { idle: 0, full: 0 },
};

const EDGE_RANGES_GPU: SimulatedGpu = SimulatedGpu {
    id: "GPU-5a1b0000-0000-4000-8000-000000000005",
    name: "NVIDIA GeForce RTX 3080",
    architecture: "Ampere",
    bus_id: "00000000:01:00.0",
    firmware_version: "94.02.42.00.A9",
    compute_capability: "8.6",
    core_count: 8_704,
    memory_total_mib: 10_240,
    graphics_max_mhz: 2_100,
    memory_max_mhz: 9_501,
    power_limit: PowerLimit {
        min_watts: 100,
        max_watts: 370,
        default_watts: 320,
        current_watts: 370,
    },
    graphics_offset: Offset {
        min_mhz: -1_000,
        max_mhz: 1_000,
        current_mhz: -1_000,
    },
    memory_offset: Offset {
        min_mhz: -2_000,
        max_mhz: 6_000,
        current_mhz: 6_000,
    },
    fans: Some(Fans {
        count: 2,
        min_speed_percent: 0,
        max_speed_percent: 100,
    }),
    // The highest maximum operating temperature Prism accepts as plausible.
    max_operating_temperature_c: 125,
    load: &IDLE_AND_FULL,
    phase: 0,
    temperature_c: Span {
        idle: 20,
        full: 125,
    },
    power_draw_watts: Span { idle: 0, full: 370 },
    core_clock_mhz: Span {
        idle: 210,
        full: 2_100,
    },
    memory_clock_mhz: Span {
        idle: 405,
        full: 9_501,
    },
    memory_used_mib: Span {
        idle: 0,
        full: 10_240,
    },
    fan_speed_percent: Span { idle: 0, full: 100 },
};
