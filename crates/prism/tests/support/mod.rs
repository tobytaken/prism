//! The app harness: the real window and glue on simulated GPUs, a fixed clock and fake settings,
//! under Slint's testing backend with no display.

use std::cell::Cell;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle, ElementRoot};
use prism::settings::{GuiSettings, SettingsStore};
use prism::{App, Clock, Environment, HostServices, PrismdConnector, Sampling};
use prism_gpu::GpuProviders;
use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
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

    /// The one element with `role` labelled `label`.
    pub fn find(&self, role: AccessibleRole, label: &str) -> ElementHandle {
        let mut found = ElementHandle::find_by_accessible_label(self.app.window(), label)
            .filter(|element| element.accessible_role() == Some(role));
        let element = found
            .next()
            .unwrap_or_else(|| panic!("no {role:?} is labelled {label:?}"));
        assert!(
            found.next().is_none(),
            "more than one {role:?} is labelled {label:?}"
        );
        element
    }

    /// How many elements with `role` are labelled `label`.
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

/// Sets up Slint's testing backend for this thread, once: each test thread gets its own backend
/// and mocked time.
pub fn init_testing_backend() {
    thread_local! {
        static INITIALIZED: Cell<bool> = const { Cell::new(false) };
    }
    if !INITIALIZED.replace(true) {
        i_slint_backend_testing::init_no_event_loop();
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
