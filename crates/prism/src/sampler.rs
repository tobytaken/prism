//! The in-process sampler, which reads the GPUs while there is no prismd connection.

use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use prism_gpu::{GpuInventoryResponse, GpuPerformanceResponse, GpuProviders};

/// What the sampler read.
pub(crate) enum SamplerReading {
    Inventory(GpuInventoryResponse),
    Telemetry(GpuPerformanceResponse),
}

/// Reads the GPU inventory, then every GPU's telemetry on each read, and the inventory again
/// when GPUs seem to have arrived or departed.
pub(crate) struct Sampler {
    gpus: GpuProviders,
    /// The GPUs, by id, in the inventory last handed on; none until it's read, and again once
    /// the sampler resumes.
    inventory_ids: Option<Vec<String>>,
}

impl Sampler {
    pub(crate) fn new(gpus: GpuProviders) -> Self {
        Self {
            gpus,
            inventory_ids: None,
        }
    }

    /// The next readings: the inventory when it's due or has changed, then a telemetry sample.
    pub(crate) fn read(&mut self) -> Vec<SamplerReading> {
        let mut readings = Vec::with_capacity(2);
        if self.inventory_ids.is_none() {
            let inventory = self.gpus.inventory();
            self.inventory_ids = Some(gpu_ids(&inventory.gpus, |gpu| &gpu.id));
            readings.push(SamplerReading::Inventory(inventory));
        }
        let performance = self.gpus.performance();
        // Readings of other GPUs hint that GPUs arrived or departed, or that discovery which
        // failed would now succeed. The inventory itself decides, so a reading that briefly
        // misses a GPU hands on nothing.
        if self.inventory_ids.as_ref() != Some(&gpu_ids(&performance.gpus, |gpu| &gpu.id)) {
            let inventory = self.gpus.inventory();
            let ids = gpu_ids(&inventory.gpus, |gpu| &gpu.id);
            if self.inventory_ids.as_ref() != Some(&ids) {
                self.inventory_ids = Some(ids);
                readings.push(SamplerReading::Inventory(inventory));
            }
        }
        readings.push(SamplerReading::Telemetry(performance));
        readings
    }

    /// Reads the inventory again on the next read, as GPUs may have come and gone while the
    /// sampler was off.
    pub(crate) fn resumed(&mut self) {
        self.inventory_ids = None;
    }

    /// Reads on a thread of its own once every `interval` while `switch` is on, handing each
    /// reading to `post` until it returns false.
    pub(crate) fn spawn(
        mut self,
        interval: Duration,
        switch: SamplerSwitch,
        mut post: impl FnMut(SamplerReading) -> bool + Send + 'static,
    ) -> std::io::Result<()> {
        thread::Builder::new()
            .name("prism-sampler".into())
            .spawn(move || {
                let mut next = Instant::now();
                loop {
                    if switch.wait_until_on() {
                        // Resumed after a pause: read now, not on the old schedule.
                        next = Instant::now();
                        self.resumed();
                    }
                    for reading in self.read() {
                        if !post(reading) {
                            return;
                        }
                    }
                    // Keep to the interval however long reading took, without catching up on
                    // ticks missed while the thread couldn't run.
                    next += interval;
                    let now = Instant::now();
                    match next.checked_duration_since(now) {
                        Some(wait) => thread::sleep(wait),
                        None => next = now,
                    }
                }
            })
            .map(|_| ())
    }
}

fn gpu_ids<T>(gpus: &[T], id: impl Fn(&T) -> &String) -> Vec<String> {
    gpus.iter().map(|gpu| id(gpu).clone()).collect()
}

/// Turns the sampler's reading on and off. Off, the sampler waits without reading, and keeps
/// what it loaded, such as NVML.
#[derive(Clone, Default)]
pub(crate) struct SamplerSwitch(Arc<(Mutex<bool>, Condvar)>);

impl SamplerSwitch {
    pub(crate) fn set(&self, on: bool) {
        let (state, changed) = &*self.0;
        *state.lock().unwrap() = on;
        changed.notify_all();
    }

    pub(crate) fn is_on(&self) -> bool {
        *self.0.0.lock().unwrap()
    }

    /// Waits until the switch is on; says whether it had to wait.
    fn wait_until_on(&self) -> bool {
        let (state, changed) = &*self.0;
        let mut on = state.lock().unwrap();
        let waited = !*on;
        while !*on {
            on = changed.wait(on).unwrap();
        }
        waited
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
    use prism_gpu::{GpuInventoryProvider, GpuTelemetryProvider};

    use super::*;

    /// Simulated GPUs whose fixture a test can change between reads.
    #[derive(Clone)]
    struct Host(Arc<Mutex<SimulatedFixture>>);

    impl Host {
        fn new(fixture: SimulatedFixture) -> Self {
            Self(Arc::new(Mutex::new(fixture)))
        }

        fn becomes(&self, fixture: SimulatedFixture) {
            *self.0.lock().unwrap() = fixture;
        }

        fn gpus(&self) -> SimulatedGpus {
            SimulatedGpus::new(*self.0.lock().unwrap(), || Duration::ZERO)
        }

        fn sampler(&self) -> Sampler {
            Sampler::new(
                GpuProviders::empty()
                    .with_inventory(self.clone())
                    .with_telemetry(self.clone()),
            )
        }
    }

    impl GpuInventoryProvider for Host {
        fn inventory(&self) -> GpuInventoryResponse {
            self.gpus().inventory()
        }
    }

    impl GpuTelemetryProvider for Host {
        fn performance(&self) -> GpuPerformanceResponse {
            self.gpus().performance()
        }
    }

    /// The ids of the GPUs in each inventory among `readings`.
    fn inventories(readings: &[SamplerReading]) -> Vec<Vec<String>> {
        readings
            .iter()
            .filter_map(|reading| match reading {
                SamplerReading::Inventory(inventory) => {
                    Some(gpu_ids(&inventory.gpus, |gpu| &gpu.id))
                }
                SamplerReading::Telemetry(_) => None,
            })
            .collect()
    }

    fn ids(fixture: SimulatedFixture) -> Vec<String> {
        let inventory = SimulatedGpus::new(fixture, || Duration::ZERO).inventory();
        gpu_ids(&inventory.gpus, |gpu| &gpu.id)
    }

    #[test]
    fn the_inventory_is_read_first_and_not_again_while_the_gpus_stay() {
        let host = Host::new(SimulatedFixture::TwoIdenticalGpus);
        let mut sampler = host.sampler();

        let first = sampler.read();
        assert!(matches!(first[0], SamplerReading::Inventory(_)));
        assert!(matches!(first[1], SamplerReading::Telemetry(_)));
        for _ in 0..3 {
            assert_eq!(inventories(&sampler.read()), Vec::<Vec<String>>::new());
        }
    }

    #[test]
    fn gpus_found_after_discovery_failed_are_handed_on() {
        let host = Host::new(SimulatedFixture::NvmlMissing);
        let mut sampler = host.sampler();
        assert_eq!(inventories(&sampler.read()), vec![Vec::<String>::new()]);
        assert_eq!(inventories(&sampler.read()), Vec::<Vec<String>>::new());

        host.becomes(SimulatedFixture::TunableGpu);
        let readings = sampler.read();

        assert_eq!(
            inventories(&readings),
            vec![ids(SimulatedFixture::TunableGpu)]
        );
        // The inventory comes first, so the sample lands on GPUs Prism knows.
        assert!(matches!(
            readings.last(),
            Some(SamplerReading::Telemetry(_))
        ));
        assert_eq!(inventories(&sampler.read()), Vec::<Vec<String>>::new());
    }

    #[test]
    fn gpus_that_arrive_or_depart_while_sampling_are_handed_on() {
        let host = Host::new(SimulatedFixture::TunableGpu);
        let mut sampler = host.sampler();
        sampler.read();

        host.becomes(SimulatedFixture::TwoIdenticalGpus);
        assert_eq!(
            inventories(&sampler.read()),
            vec![ids(SimulatedFixture::TwoIdenticalGpus)]
        );

        host.becomes(SimulatedFixture::EmptyInventory);
        assert_eq!(inventories(&sampler.read()), vec![Vec::<String>::new()]);
    }

    /// Telemetry that keeps missing a GPU the inventory lists hints at a change each read.
    struct Unlisted {
        inventory_reads: Arc<AtomicUsize>,
    }

    impl GpuInventoryProvider for Unlisted {
        fn inventory(&self) -> GpuInventoryResponse {
            self.inventory_reads.fetch_add(1, Ordering::SeqCst);
            SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO).inventory()
        }
    }

    impl GpuTelemetryProvider for Unlisted {
        fn performance(&self) -> GpuPerformanceResponse {
            GpuPerformanceResponse::default()
        }
    }

    #[test]
    fn a_reading_that_misses_a_listed_gpu_hands_on_no_inventory() {
        let inventory_reads = Arc::new(AtomicUsize::new(0));
        let mut sampler = Sampler::new(
            GpuProviders::empty()
                .with_inventory(Unlisted {
                    inventory_reads: inventory_reads.clone(),
                })
                .with_telemetry(Unlisted {
                    inventory_reads: inventory_reads.clone(),
                }),
        );
        sampler.read();

        assert_eq!(inventories(&sampler.read()), Vec::<Vec<String>>::new());
        assert_eq!(inventory_reads.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn a_resumed_sampler_reads_the_inventory_again() {
        let host = Host::new(SimulatedFixture::TunableGpu);
        let mut sampler = host.sampler();
        sampler.read();

        sampler.resumed();

        assert_eq!(
            inventories(&sampler.read()),
            vec![ids(SimulatedFixture::TunableGpu)]
        );
    }
}
