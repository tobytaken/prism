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

/// Reads the GPU inventory once, then every GPU's telemetry on each read.
pub(crate) struct Sampler {
    gpus: GpuProviders,
    inventory_read: bool,
}

impl Sampler {
    pub(crate) fn new(gpus: GpuProviders) -> Self {
        Self {
            gpus,
            inventory_read: false,
        }
    }

    /// The next readings: the inventory first, then a telemetry sample.
    pub(crate) fn read(&mut self) -> Vec<SamplerReading> {
        let mut readings = Vec::with_capacity(2);
        if !self.inventory_read {
            self.inventory_read = true;
            readings.push(SamplerReading::Inventory(self.gpus.inventory()));
        }
        readings.push(SamplerReading::Telemetry(self.gpus.performance()));
        readings
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
