//! The in-process sampler, which reads the GPUs while there is no prismd connection.

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

    /// Reads on a thread of its own once every `interval`, handing each reading to `post` until
    /// it returns false.
    pub(crate) fn spawn(
        mut self,
        interval: Duration,
        mut post: impl FnMut(SamplerReading) -> bool + Send + 'static,
    ) -> std::io::Result<()> {
        thread::Builder::new()
            .name("prism-sampler".into())
            .spawn(move || {
                let mut next = Instant::now();
                loop {
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
