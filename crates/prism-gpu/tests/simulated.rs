use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_gpu::{
    ControlSupport, FanControlMode, GpuInventoryProvider, GpuInventoryResponse,
    GpuPerformanceResponse, GpuProviders, GpuTelemetryProvider,
};

fn at(fixture: SimulatedFixture, seconds: u64) -> SimulatedGpus {
    SimulatedGpus::new(fixture, move || Duration::from_secs(seconds))
}

fn inventory(fixture: SimulatedFixture) -> GpuInventoryResponse {
    at(fixture, 0).inventory()
}

fn performance(fixture: SimulatedFixture) -> GpuPerformanceResponse {
    at(fixture, 0).performance()
}

#[test]
fn every_fixture_is_found_by_its_name() {
    let names: Vec<_> = SimulatedFixture::ALL
        .iter()
        .map(|fixture| fixture.name())
        .collect();

    assert_eq!(
        names,
        [
            "tunable",
            "two-identical",
            "no-fan-control",
            "edge-ranges",
            "nvml-missing",
            "empty-inventory",
            "driver-not-loaded",
        ]
    );
    for fixture in SimulatedFixture::ALL {
        assert_eq!(fixture.name().parse(), Ok(fixture));
        assert_eq!(fixture.to_string(), fixture.name());
    }
}

#[test]
fn an_unknown_fixture_name_lists_the_known_ones() {
    let error = "rtx-9090".parse::<SimulatedFixture>().unwrap_err();

    assert_eq!(
        error.to_string(),
        "unknown simulated GPU fixture \"rtx-9090\"; expected one of: tunable, two-identical, \
         no-fan-control, edge-ranges, nvml-missing, empty-inventory, driver-not-loaded"
    );
}

#[test]
fn tunable_fixture_has_one_gpu_with_two_controllable_fans() {
    let inventory = inventory(SimulatedFixture::TunableGpu);
    let performance = performance(SimulatedFixture::TunableGpu);

    assert_eq!(inventory.gpus.len(), 1);
    assert!(inventory.warnings.is_empty());
    let control = &inventory.gpus[0].control.supported_features;
    assert!(control.power_limit && control.graphics_offset && control.memory_offset);

    assert_eq!(performance.gpus.len(), 1);
    assert!(performance.warnings.is_empty());
    let gpu = &performance.gpus[0];
    assert_eq!(gpu.id, inventory.gpus[0].id);
    assert_eq!(gpu.stable_id.as_deref(), Some(gpu.id.as_str()));
    assert_eq!(gpu.power.support, ControlSupport::Supported);
    assert_eq!(gpu.clocks.graphics.support, ControlSupport::Supported);
    assert_eq!(gpu.clocks.memory.support, ControlSupport::Supported);
    assert!(gpu.cooling.is_supported);
    assert_eq!(gpu.cooling.fan_count, 2);
    assert_eq!(
        gpu.cooling
            .fans
            .iter()
            .map(|fan| (fan.index, fan.mode))
            .collect::<Vec<_>>(),
        [
            (0, Some(FanControlMode::Auto)),
            (1, Some(FanControlMode::Auto))
        ]
    );
    assert!(gpu.cooling.max_operating_temperature_c.is_some());
}

#[test]
fn two_identical_fixture_has_two_gpus_of_one_model() {
    let inventory = inventory(SimulatedFixture::TwoIdenticalGpus);
    let performance = performance(SimulatedFixture::TwoIdenticalGpus);

    let [first, second] = &inventory.gpus[..] else {
        panic!("expected two GPUs, got {:?}", inventory.gpus);
    };
    assert_eq!(first.name, second.name);
    assert_ne!(first.id, second.id);
    assert_ne!(first.bus_id, second.bus_id);
    assert_eq!(
        performance
            .gpus
            .iter()
            .map(|gpu| gpu.id.as_str())
            .collect::<Vec<_>>(),
        [first.id.as_str(), second.id.as_str()]
    );
}

#[test]
fn no_fan_control_fixture_has_a_gpu_without_controllable_fans() {
    let performance = performance(SimulatedFixture::NoFanControl);

    let [gpu] = &performance.gpus[..] else {
        panic!("expected one GPU, got {:?}", performance.gpus);
    };
    assert!(!gpu.cooling.is_supported);
    assert!(gpu.cooling.unsupported_reason.is_some());
    assert_eq!(gpu.cooling.fan_count, 0);
    assert!(gpu.cooling.fans.is_empty());
    assert_eq!(gpu.telemetry.fan_speed_percent, None);
    assert_eq!(gpu.power.support, ControlSupport::Supported);
}

#[test]
fn edge_ranges_fixture_sets_tuning_values_on_their_bounds() {
    let inventory = inventory(SimulatedFixture::EdgeRanges);
    let performance = performance(SimulatedFixture::EdgeRanges);

    let [device] = &inventory.gpus[..] else {
        panic!("expected one GPU, got {:?}", inventory.gpus);
    };
    let power_limit = &device.control.power_limit;
    assert_eq!(power_limit.current_watts, power_limit.max_watts);
    let offsets = &device.control.offsets;
    assert_eq!(offsets.graphics_mhz, offsets.min_graphics_mhz);
    assert_eq!(offsets.memory_mhz, offsets.max_memory_mhz);

    let [gpu] = &performance.gpus[..] else {
        panic!("expected one GPU, got {:?}", performance.gpus);
    };
    assert_eq!(gpu.power.current_limit_watts, gpu.power.max_limit_watts);
    assert_eq!(
        gpu.clocks.graphics.current_offset_mhz,
        gpu.clocks.graphics.min_offset_mhz
    );
    assert_eq!(
        gpu.clocks.memory.current_offset_mhz,
        gpu.clocks.memory.max_offset_mhz
    );
    for fan in &gpu.cooling.fans {
        assert_eq!(fan.min_speed_percent, Some(0));
        assert_eq!(fan.max_speed_percent, Some(100));
    }
}

#[test]
fn edge_ranges_fixture_replays_telemetry_at_both_ends_of_each_range() {
    let gpus: Vec<_> = (0..2)
        .map(|second| {
            at(SimulatedFixture::EdgeRanges, second)
                .performance()
                .gpus
                .remove(0)
        })
        .collect();
    let [idle, peak] = &gpus[..] else {
        unreachable!()
    };

    assert_eq!(idle.telemetry.gpu_utilization_percent, Some(0));
    assert_eq!(peak.telemetry.gpu_utilization_percent, Some(100));
    assert_eq!(idle.telemetry.power_draw_watts, Some(0));
    assert_eq!(peak.telemetry.power_draw_watts, peak.power.max_limit_watts);
    assert_eq!(
        peak.telemetry.temperature_c,
        peak.cooling.max_operating_temperature_c
    );
    assert_eq!(
        peak.telemetry.memory_used_mib,
        peak.telemetry.memory_total_mib
    );
    for (idle_fan, peak_fan) in idle.cooling.fans.iter().zip(&peak.cooling.fans) {
        assert_eq!(idle_fan.current_speed_percent, idle_fan.min_speed_percent);
        assert_eq!(peak_fan.current_speed_percent, peak_fan.max_speed_percent);
    }
}

#[test]
fn nvml_missing_fixture_reports_no_gpus_and_why() {
    let inventory = inventory(SimulatedFixture::NvmlMissing);
    let performance = performance(SimulatedFixture::NvmlMissing);

    assert!(inventory.gpus.is_empty());
    assert_eq!(
        inventory.warnings,
        [
            "NVIDIA inventory is unavailable on this system: unable to initialize NVML: a \
             libloading error occurred: libnvidia-ml.so: cannot open shared object file: No \
             such file or directory"
        ]
    );
    assert!(performance.gpus.is_empty());
    assert_eq!(
        performance.warnings,
        [
            "NVIDIA performance controls are unavailable on this system: unable to initialize \
             NVML: a libloading error occurred: libnvidia-ml.so: cannot open shared object \
             file: No such file or directory"
        ]
    );
}

#[test]
fn empty_fixture_reports_no_gpus_and_no_warnings() {
    assert_eq!(
        inventory(SimulatedFixture::EmptyInventory),
        GpuInventoryResponse::default()
    );
    assert_eq!(
        performance(SimulatedFixture::EmptyInventory),
        GpuPerformanceResponse::default()
    );
}

#[test]
fn driver_not_loaded_fixture_reports_no_gpus_and_why() {
    let inventory = inventory(SimulatedFixture::DriverNotLoaded);
    let performance = performance(SimulatedFixture::DriverNotLoaded);

    assert!(inventory.gpus.is_empty());
    assert_eq!(
        inventory.warnings,
        [
            "NVIDIA inventory is unavailable on this system: unable to initialize NVML: NVIDIA \
             driver is not loaded"
        ]
    );
    assert!(performance.gpus.is_empty());
    assert_eq!(
        performance.warnings,
        [
            "NVIDIA performance controls are unavailable on this system: unable to initialize \
             NVML: NVIDIA driver is not loaded"
        ]
    );
}

/// The replay every fixture's readings must match, one JSON line per fixture and second.
const RECORDED_REPLAY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/simulated-replay.jsonl");

/// Rewrites the recorded replay instead of checking it, after an intended fixture change.
const RECORD_REPLAY_VAR: &str = "PRISM_RECORD_SIMULATED_REPLAY";

#[test]
fn telemetry_replayed_against_a_fixed_clock_matches_the_recorded_replay() {
    let mut replay = String::new();
    for fixture in SimulatedFixture::ALL {
        for second in [0, 1, 7, 3_600] {
            let gpus = at(fixture, second);
            let line = serde_json::json!({
                "fixture": fixture.name(),
                "second": second,
                "inventory": gpus.inventory(),
                "performance": gpus.performance(),
            });
            replay.push_str(&line.to_string());
            replay.push('\n');
        }
    }

    if std::env::var_os(RECORD_REPLAY_VAR).is_some() {
        std::fs::write(RECORDED_REPLAY, &replay).expect("record the replay");
    }
    let recorded = std::fs::read_to_string(RECORDED_REPLAY).expect("read the recorded replay");
    assert!(
        replay == recorded,
        "simulated readings differ from {RECORDED_REPLAY}; if the fixtures changed on purpose, \
         rerun with {RECORD_REPLAY_VAR}=1 and review the diff"
    );
}

#[test]
fn telemetry_follows_the_injected_clock() {
    let now = Arc::new(AtomicU64::new(0));
    let clock = Arc::clone(&now);
    let gpus = SimulatedGpus::new(SimulatedFixture::TunableGpu, move || {
        Duration::from_millis(clock.load(Ordering::SeqCst))
    });
    let reading = || gpus.performance().gpus.remove(0).telemetry;

    let first = reading();
    now.store(999, Ordering::SeqCst);
    assert_eq!(reading(), first, "a sample holds for its whole second");
    now.store(1_000, Ordering::SeqCst);
    assert_ne!(reading(), first, "the next second replays the next sample");
}

#[test]
fn telemetry_series_repeat() {
    let series = |start: u64| {
        (start..start + 120)
            .map(|second| at(SimulatedFixture::TunableGpu, second).performance())
            .collect::<Vec<_>>()
    };
    let first = series(0);
    let period = (1..first.len())
        .find(|&offset| series(offset as u64) == first)
        .expect("the series repeats within two minutes");

    assert!(period > 1, "the readings change over time");
}

#[test]
fn identical_gpus_report_different_readings() {
    let performance = at(SimulatedFixture::TwoIdenticalGpus, 3).performance();

    assert_ne!(performance.gpus[0].telemetry, performance.gpus[1].telemetry);
}

#[test]
fn a_fixture_serves_both_inventory_and_telemetry_through_injected_providers() {
    let gpus = Arc::new(at(SimulatedFixture::TwoIdenticalGpus, 0));
    let providers = GpuProviders::empty()
        .with_inventory(Arc::clone(&gpus))
        .with_telemetry(Arc::clone(&gpus));

    assert_eq!(providers.inventory(), gpus.inventory());
    assert_eq!(providers.performance(), gpus.performance());
}
