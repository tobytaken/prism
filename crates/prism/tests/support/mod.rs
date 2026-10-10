//! The app harness: the real window and glue on simulated GPUs, a fixed clock and fake settings,
//! under Slint's testing backend with no display.

// Each test binary uses part of the harness.
#![allow(dead_code)]

pub mod audit;
pub mod screenshot;

use std::cell::Cell;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle, ElementRoot};
use prism::settings::{GuiSettings, SettingsStore};
use prism::{App, Clock, Environment, HostServices, PrismdConnector, Sampling};
use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_gpu::{GpuPerformanceResponse, GpuProviders, GpuTelemetryProvider};
use prism_ui::AppearanceMode;
use slint::ComponentHandle;

/// A clock that only moves when the test advances it.
#[derive(Default)]
pub struct ManualClock(AtomicU64);

impl ManualClock {
    pub fn advance(&self, by: Duration) {
        self.0.fetch_add(by.as_millis() as u64, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
}

struct FixedSettings(GuiSettings);

impl SettingsStore for FixedSettings {
    fn load(&self) -> GuiSettings {
        self.0
    }
}

pub struct Harness {
    pub app: App,
    clock: Arc<ManualClock>,
}

impl Harness {
    /// Prism on `fixture` in Light: the testing backend reports no system color scheme, so tests
    /// pin the appearance.
    pub fn new(fixture: SimulatedFixture) -> Self {
        Self::with_appearance(fixture, AppearanceMode::Light)
    }

    pub fn with_appearance(fixture: SimulatedFixture, appearance: AppearanceMode) -> Self {
        Self::with_gpus(appearance, |clock| {
            let gpus = Arc::new(SimulatedGpus::new(fixture, move || clock.now()));
            GpuProviders::empty()
                .with_inventory(gpus.clone())
                .with_telemetry(gpus)
        })
    }

    /// Prism on the tunable fixture with its temperature sensor gone.
    pub fn without_temperature() -> Self {
        Self::with_gpus(AppearanceMode::Light, |clock| {
            let tunable = || {
                let clock = clock.clone();
                SimulatedGpus::new(SimulatedFixture::TunableGpu, move || clock.now())
            };
            GpuProviders::empty()
                .with_inventory(tunable())
                .with_telemetry(WithoutTemperature(tunable()))
        })
    }

    /// Prism on the GPUs `gpus` builds, which can read the harness's clock.
    pub fn with_gpus(
        appearance: AppearanceMode,
        gpus: impl FnOnce(Arc<ManualClock>) -> GpuProviders,
    ) -> Self {
        init_testing_backend();
        let clock = Arc::new(ManualClock::default());
        let app = App::new(Environment {
            gpus: gpus(clock.clone()),
            prismd: PrismdConnector::Placeholder,
            host: HostServices::default(),
            settings: Box::new(FixedSettings(GuiSettings { appearance })),
            clock: clock.clone(),
            sampling: Sampling::OnDemand,
        })
        .expect("build the main window");
        app.window().show().expect("show the main window");
        ensure_element_tree(app.window());
        Self { app, clock }
    }

    /// Lets a second pass, then reads every GPU once.
    pub fn tick(&self) {
        self.clock.advance(Duration::from_secs(1));
        self.app.sample_gpus();
    }

    pub fn ticks(&self, count: usize) {
        for _ in 0..count {
            self.tick();
        }
    }

    /// The one element with `role` labeled `label`.
    pub fn find(&self, role: AccessibleRole, label: &str) -> ElementHandle {
        let mut found = ElementHandle::find_by_accessible_label(self.app.window(), label)
            .filter(|element| element.accessible_role() == Some(role));
        let element = found
            .next()
            .unwrap_or_else(|| panic!("no {role:?} is labeled {label:?}"));
        assert!(
            found.next().is_none(),
            "more than one {role:?} is labeled {label:?}"
        );
        element
    }

    /// How many elements with `role` are labeled `label`.
    pub fn count(&self, role: AccessibleRole, label: &str) -> usize {
        ElementHandle::find_by_accessible_label(self.app.window(), label)
            .filter(|element| element.accessible_role() == Some(role))
            .count()
    }

    /// Whether some text reads `text`.
    pub fn shows(&self, text: &str) -> bool {
        self.count(AccessibleRole::Text, text) > 0
    }
}

/// The tunable fixture's GPUs, with their temperature sensor gone.
struct WithoutTemperature(SimulatedGpus);

impl GpuTelemetryProvider for WithoutTemperature {
    fn performance(&self) -> GpuPerformanceResponse {
        let mut performance = self.0.performance();
        for gpu in &mut performance.gpus {
            gpu.telemetry.temperature_c = None;
        }
        performance
    }
}

/// The testing backend a thread runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    /// No renderer, with mocked text metrics.
    Mock,
    /// The software renderer, for screenshots.
    Software,
}

thread_local! {
    static BACKEND: Cell<Option<Backend>> = const { Cell::new(None) };
}

/// Sets up Slint's testing backend for this thread unless it has one: each test thread gets its
/// own backend and mocked time.
pub fn init_testing_backend() {
    if BACKEND.get().is_none() {
        init_backend(Backend::Mock, i_slint_backend_testing::init_no_event_loop);
    }
}

/// Sets up `backend` for this thread with `init`, once.
fn init_backend(backend: Backend, init: impl FnOnce()) {
    match BACKEND.get() {
        None => {
            init();
            BACKEND.set(Some(backend));
        }
        Some(current) => assert_eq!(
            current, backend,
            "this thread already runs another testing backend"
        ),
    }
}

/// The text shown inside `element`, in tree order.
pub fn texts(element: &ElementHandle) -> Vec<String> {
    element
        .query_descendants()
        .match_accessible_role(AccessibleRole::Text)
        .find_all()
        .iter()
        .filter_map(|text| text.accessible_label().map(String::from))
        .collect()
}

/// Panics unless the component has elements to find. Without Slint's debug info every search
/// finds nothing, so tests would otherwise pass without checking anything.
pub fn ensure_element_tree(component: &impl ElementRoot) {
    let found = component
        .root_element()
        .visit_descendants(|_| ControlFlow::Break(()));
    assert!(
        found.is_some(),
        "the element tree is empty: build the UI with Slint debug info (prism's build script \
         enables it outside release builds; SLINT_EMIT_DEBUG_INFO=1 forces it)"
    );
}
