//! The app harness: the real window and glue on simulated GPUs, a fake prismd connection, a
//! fixed clock and fake settings, under Slint's testing backend with no display.

// Each test binary uses part of the harness.
#![allow(dead_code)]

pub mod audit;
pub mod bus;
pub mod display;
pub mod prismd;
pub mod screenshot;

use std::cell::{Cell, RefCell};
use std::ops::ControlFlow;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle, ElementRoot};
use i_slint_core::input::{InternalKeyEvent, KeyEventType};
use prism::settings::SettingsStore;
use prism::{
    App, Clock, Environment, FileChooser, HostServices, Launches, NotificationEvent, Notifier,
    Opener, Sampling, WindowActivation,
};
use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_gpu::{
    GpuInventoryProvider, GpuInventoryResponse, GpuPerformanceDevice, GpuPerformanceResponse,
    GpuProviders, GpuTelemetryProvider,
};

use self::prismd::{DaemonState, FakeHost, FakePrismd};
use prism_ui::{
    AppearanceMode, GuiSettings, Link, Notification, NotificationId, PrismdInstall, ProgramFile,
    UnitState,
};
use slint::ComponentHandle;
use slint::platform::{Key, WindowEvent};

/// Prism's default window size, half of a 1080p screen, as when snapped beside another window,
/// and its minimum, in pixels.
pub const DEFAULT_SIZE: (u32, u32) = (1280, 800);
pub const HALF_1080P: (u32, u32) = (960, 1080);
pub const MINIMUM_SIZE: (u32, u32) = (960, 600);

/// A clock that only moves when the test advances it.
#[derive(Default)]
pub struct ManualClock(AtomicU64);

impl ManualClock {
    pub fn advance(&self, by: Duration) {
        self.0.fetch_add(by.as_millis() as u64, Ordering::SeqCst);
    }
}

/// When Prism starts in view tests, as a wall clock time in Unix milliseconds.
pub const STARTED_UNIX_MS: u64 = 1_791_804_860_000;

impl Clock for ManualClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::SeqCst))
    }

    fn started_unix_ms(&self) -> u64 {
        STARTED_UNIX_MS
    }
}

/// The GUI's settings file, which outlives the Prism that saves to it, so the next one loads what
/// it saved.
#[derive(Clone, Default)]
pub struct FakeSettings(Arc<Mutex<GuiSettings>>);

impl FakeSettings {
    pub fn new(settings: GuiSettings) -> Self {
        Self(Arc::new(Mutex::new(settings)))
    }

    /// The settings saved last, or the ones it started with.
    pub fn saved(&self) -> GuiSettings {
        *self.0.lock().unwrap()
    }
}

impl SettingsStore for FakeSettings {
    fn load(&self) -> GuiSettings {
        self.saved()
    }

    fn save(&self, settings: &GuiSettings) {
        *self.0.lock().unwrap() = *settings;
    }
}

/// What the desktop was asked to open, oldest first.
#[derive(Clone, Default)]
pub struct FakeOpener(Arc<Mutex<Vec<Link>>>);

impl FakeOpener {
    pub fn opened(&self) -> Vec<Link> {
        self.0.lock().unwrap().clone()
    }
}

impl Opener for FakeOpener {
    fn open(&self, link: Link) {
        self.0.lock().unwrap().push(link);
    }
}

/// Later launches of Prism that the test makes, as the single-instance bus would hand them over.
#[derive(Clone, Default)]
pub struct FakeLaunches(Listener);

type Listener = Arc<Mutex<Option<Box<dyn Fn(Option<String>) + Send + Sync>>>>;

impl FakeLaunches {
    /// The side Prism holds, which owns the app ID's name until Prism drops it.
    fn name(&self) -> Box<dyn Launches> {
        Box::new(FakeName(self.0.clone()))
    }

    /// Launches Prism again with `activation_token`, which hands over to the running one.
    pub fn launch(&self, activation_token: Option<&str>) {
        let listener = self.0.lock().unwrap();
        let launched = listener
            .as_ref()
            .expect("the running Prism owns the name and listens for later launches");
        launched(activation_token.map(String::from));
    }
}

struct FakeName(Listener);

impl Launches for FakeName {
    fn listen(&self, launched: Box<dyn Fn(Option<String>) + Send + Sync>) {
        *self.0.lock().unwrap() = Some(launched);
    }
}

impl Drop for FakeName {
    /// Like closing the bus connection, gives up the name.
    fn drop(&mut self) {
        *self.0.lock().unwrap() = None;
    }
}

/// Records each time the window is focused through the desktop, with the activation token, and
/// lets the test say when the window gains or loses focus.
#[derive(Clone, Default)]
pub struct RecordedActivation(Rc<RefCell<Activation>>);

#[derive(Default)]
struct Activation {
    tokens: Vec<Option<String>>,
    focus_changed: Option<Box<dyn Fn(bool)>>,
}

impl RecordedActivation {
    pub fn tokens(&self) -> Vec<Option<String>> {
        self.0.borrow().tokens.clone()
    }

    /// The window gains or loses focus, as the desktop would say.
    pub fn set_focused(&self, focused: bool) {
        let activation = self.0.borrow();
        let changed = activation
            .focus_changed
            .as_ref()
            .expect("Prism watches the window's focus");
        changed(focused);
    }
}

impl WindowActivation for RecordedActivation {
    fn activate(&self, _: &slint::Window, activation_token: Option<&str>) {
        self.0
            .borrow_mut()
            .tokens
            .push(activation_token.map(String::from));
    }

    fn watch_focus(&self, _: &slint::Window, changed: Box<dyn Fn(bool)>) {
        self.0.borrow_mut().focus_changed = Some(changed);
    }
}

/// The desktop's notification server, as Prism's notifier would reach it.
#[derive(Clone, Default)]
pub struct FakeNotifier(Arc<Mutex<Notifications>>);

#[derive(Default)]
struct Notifications {
    listener: Option<Arc<dyn Fn(NotificationEvent) + Send + Sync>>,
    sent: Vec<Notification>,
    /// The notifications showing, by Prism's id.
    live: Vec<NotificationId>,
}

impl FakeNotifier {
    /// Every notification Prism sent, oldest first.
    pub fn sent(&self) -> Vec<Notification> {
        self.0.lock().unwrap().sent.clone()
    }

    /// The notifications showing.
    pub fn live(&self) -> Vec<NotificationId> {
        self.0.lock().unwrap().live.clone()
    }

    /// The user clicks the notification `id`, which the server closes, handing over
    /// `activation_token`.
    pub fn click(&self, id: NotificationId, activation_token: Option<&str>) {
        let mut notifications = self.0.lock().unwrap();
        assert!(notifications.live.contains(&id), "{id:?} isn't showing");
        notifications.live.retain(|live| *live != id);
        let listener = notifications.listener.clone().expect("Prism listens");
        drop(notifications);
        listener(NotificationEvent::Clicked {
            id,
            activation_token: activation_token.map(String::from),
        });
    }
}

impl Notifier for FakeNotifier {
    fn listen(&self, event: Box<dyn Fn(NotificationEvent) + Send + Sync>) {
        self.0.lock().unwrap().listener = Some(Arc::from(event));
    }

    fn notify(&self, notification: Notification) {
        let mut notifications = self.0.lock().unwrap();
        notifications
            .live
            .retain(|live| Some(*live) != notification.replaces);
        notifications.live.push(notification.id);
        notifications.sent.push(notification);
    }

    fn withdraw(&self, id: NotificationId) {
        self.0.lock().unwrap().live.retain(|live| *live != id);
    }
}

/// What a file chooser calls with the program file chosen.
type Chosen = Box<dyn FnOnce(ProgramFile) + Send>;

/// The desktop's file chooser, which stays open until the test answers it.
#[derive(Clone, Default)]
pub struct FakeFileChooser(Arc<Mutex<Vec<Chosen>>>);

impl FakeFileChooser {
    /// How many file choosers are open.
    pub fn open(&self) -> usize {
        self.0.lock().unwrap().len()
    }

    /// The user closes the file chooser opened first, with `file`.
    pub fn answer(&self, file: ProgramFile) {
        let chosen = self.0.lock().unwrap().remove(0);
        chosen(file);
    }
}

impl FileChooser for FakeFileChooser {
    fn choose_program(&self, chosen: Box<dyn FnOnce(ProgramFile) + Send>) {
        self.0.lock().unwrap().push(chosen);
    }
}

pub struct Harness {
    pub app: App,
    pub launches: FakeLaunches,
    pub activation: RecordedActivation,
    pub notifier: FakeNotifier,
    pub file_chooser: FakeFileChooser,
    pub opener: FakeOpener,
    pub settings: FakeSettings,
    pub prismd: FakePrismd,
    pub host: FakeHost,
    clock: Arc<ManualClock>,
    /// How often Prism read the GPUs in-process.
    in_process_reads: Arc<AtomicUsize>,
}

impl Harness {
    /// Prism on `fixture` in Light, with prismd not running: the testing backend reports no
    /// system color scheme, so tests pin the appearance.
    pub fn new(fixture: SimulatedFixture) -> Self {
        Self::with_appearance(fixture, AppearanceMode::Light)
    }

    /// Prism on `fixture` in Light, with prismd in `daemon` from the start.
    pub fn with_daemon(fixture: SimulatedFixture, daemon: DaemonState) -> Self {
        Self::build(AppearanceMode::Light, daemon, simulated(fixture))
    }

    /// Prism on `fixture` in Light, with prismd in `daemon` from the start, on a host whose
    /// prismd is as `install` says.
    pub fn with_install(
        fixture: SimulatedFixture,
        daemon: DaemonState,
        install: PrismdInstall,
    ) -> Self {
        Self::build_on(AppearanceMode::Light, daemon, install, simulated(fixture))
    }

    /// Prism on `fixture` in `appearance`, with prismd in `daemon` from the start.
    pub fn with_daemon_in(
        fixture: SimulatedFixture,
        appearance: AppearanceMode,
        daemon: DaemonState,
    ) -> Self {
        Self::build(appearance, daemon, simulated(fixture))
    }

    /// Prism on `fixture` in `appearance`, with prismd not running, on a host whose prismd is as
    /// `install` says.
    pub fn on_host(
        fixture: SimulatedFixture,
        appearance: AppearanceMode,
        install: PrismdInstall,
    ) -> Self {
        Self::build_on(
            appearance,
            DaemonState::NotRunning,
            install,
            simulated(fixture),
        )
    }

    pub fn with_appearance(fixture: SimulatedFixture, appearance: AppearanceMode) -> Self {
        Self::build(appearance, DaemonState::NotRunning, simulated(fixture))
    }

    /// Prism on `fixture`, with prismd in `daemon` from the start, loading its settings from
    /// `settings`.
    pub fn with_settings(
        fixture: SimulatedFixture,
        daemon: DaemonState,
        settings: FakeSettings,
    ) -> Self {
        let install = PrismdInstall::Native(UnitState::Inactive { enabled: false });
        Self::build_with(settings, daemon, install, simulated(fixture))
    }

    /// Prism on the tunable fixture with its temperature sensor gone.
    pub fn without_temperature() -> Self {
        Self::with_changed_readings(|gpu| gpu.telemetry.temperature_c = None)
    }

    /// Prism on the tunable fixture with every reading and current tuning value gone.
    pub fn without_readings() -> Self {
        Self::with_changed_readings(|gpu| {
            let telemetry = &mut gpu.telemetry;
            telemetry.temperature_c = None;
            telemetry.gpu_utilization_percent = None;
            telemetry.power_draw_watts = None;
            telemetry.core_clock_mhz = None;
            telemetry.memory_clock_mhz = None;
            telemetry.fan_speed_percent = None;
            gpu.power.current_draw_watts = None;
            gpu.power.current_limit_watts = None;
            gpu.power.enforced_limit_watts = None;
            gpu.clocks.graphics.current_offset_mhz = None;
            gpu.clocks.memory.current_offset_mhz = None;
            for fan in &mut gpu.cooling.fans {
                fan.current_speed_percent = None;
            }
        })
    }

    /// Prism on the tunable fixture, with each of its GPUs' readings changed by `change`.
    fn with_changed_readings(change: fn(&mut GpuPerformanceDevice)) -> Self {
        Self::with_gpus(AppearanceMode::Light, |clock| {
            let tunable = || {
                let clock = clock.clone();
                SimulatedGpus::new(SimulatedFixture::TunableGpu, move || clock.now())
            };
            GpuProviders::empty()
                .with_inventory(tunable())
                .with_telemetry(ChangedReadings(tunable(), change))
        })
    }

    /// Prism on the GPUs `gpus` builds, which can read the harness's clock.
    pub fn with_gpus(
        appearance: AppearanceMode,
        gpus: impl FnOnce(Arc<ManualClock>) -> GpuProviders,
    ) -> Self {
        Self::build(appearance, DaemonState::NotRunning, gpus)
    }

    /// With systemd saying prismd is installed but not running.
    fn build(
        appearance: AppearanceMode,
        daemon: DaemonState,
        gpus: impl FnOnce(Arc<ManualClock>) -> GpuProviders,
    ) -> Self {
        let install = PrismdInstall::Native(UnitState::Inactive { enabled: false });
        Self::build_on(appearance, daemon, install, gpus)
    }

    fn build_on(
        appearance: AppearanceMode,
        daemon: DaemonState,
        install: PrismdInstall,
        gpus: impl FnOnce(Arc<ManualClock>) -> GpuProviders,
    ) -> Self {
        let settings = FakeSettings::new(GuiSettings {
            appearance,
            ..GuiSettings::default()
        });
        Self::build_with(settings, daemon, install, gpus)
    }

    fn build_with(
        settings: FakeSettings,
        daemon: DaemonState,
        install: PrismdInstall,
        gpus: impl FnOnce(Arc<ManualClock>) -> GpuProviders,
    ) -> Self {
        init_testing_backend();
        let clock = Arc::new(ManualClock::default());
        let launches = FakeLaunches::default();
        let activation = RecordedActivation::default();
        let notifier = FakeNotifier::default();
        let file_chooser = FakeFileChooser::default();
        let opener = FakeOpener::default();
        let prismd = FakePrismd::new(daemon);
        let host = FakeHost::new(install, prismd.clone());
        let in_process_reads = Arc::default();
        let counted = Arc::new(Counted {
            gpus: gpus(clock.clone()),
            reads: Arc::clone(&in_process_reads),
        });
        let app = App::new(Environment {
            gpus: GpuProviders::empty()
                .with_inventory(counted.clone())
                .with_telemetry(counted),
            prismd: prismd.connector(),
            host: HostServices {
                prismd: host.service(),
                launches: launches.name(),
                activation: Box::new(activation.clone()),
                notifier: Box::new(notifier.clone()),
                file_chooser: Box::new(file_chooser.clone()),
                opener: Box::new(opener.clone()),
            },
            settings: Box::new(settings.clone()),
            clock: clock.clone(),
            sampling: Sampling::OnDemand,
        })
        .expect("build the main window");
        app.window().show().expect("show the main window");
        ensure_element_tree(app.window());
        // What connecting to prismd reported, as the event loop would take it in.
        app.receive_posted();
        Self {
            app,
            launches,
            activation,
            notifier,
            file_chooser,
            opener,
            settings,
            prismd,
            host,
            clock,
            in_process_reads,
        }
    }

    /// Resizes the window to `(width, height)` pixels, as the desktop would.
    pub fn resize(&self, (width, height): (u32, u32)) {
        self.app
            .window()
            .window()
            .set_size(slint::PhysicalSize::new(width, height));
    }

    /// Lets a second pass, running Slint's timers, then takes in what was posted and, while
    /// Prism samples in-process, reads every GPU once.
    pub fn tick(&self) {
        self.clock.advance(Duration::from_secs(1));
        i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
        self.app.receive_posted();
        self.app.sample_gpus();
    }

    pub fn ticks(&self, count: usize) {
        for _ in 0..count {
            self.tick();
        }
    }

    /// Lets a second pass with prismd pushing `gpus` as its telemetry.
    pub fn prismd_tick(&self, gpus: GpuPerformanceResponse) {
        self.clock.advance(Duration::from_secs(1));
        i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
        self.prismd.sample(gpus);
        self.app.receive_posted();
        self.app.sample_gpus();
    }

    /// How many times Prism read the GPUs itself, inventory and telemetry together.
    pub fn in_process_reads(&self) -> usize {
        self.in_process_reads.load(Ordering::SeqCst)
    }
}

/// The simulated fixture's GPUs, played against the harness's clock.
fn simulated(fixture: SimulatedFixture) -> impl FnOnce(Arc<ManualClock>) -> GpuProviders {
    move |clock| {
        let gpus = Arc::new(SimulatedGpus::new(fixture, move || clock.now()));
        GpuProviders::empty()
            .with_inventory(gpus.clone())
            .with_telemetry(gpus)
    }
}

/// GPUs whose every read is counted.
struct Counted {
    gpus: GpuProviders,
    reads: Arc<AtomicUsize>,
}

impl GpuInventoryProvider for Counted {
    fn inventory(&self) -> GpuInventoryResponse {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.gpus.inventory()
    }
}

impl GpuTelemetryProvider for Counted {
    fn performance(&self) -> GpuPerformanceResponse {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.gpus.performance()
    }
}

impl Harness {
    /// Presses `key` with `modifiers` held, then lets go of them all.
    pub fn press(&self, modifiers: &[Key], key: impl Into<slint::SharedString>) {
        let window = self.app.window().window();
        let key = key.into();
        let dispatch = |event| window.dispatch_event(event);
        for modifier in modifiers {
            dispatch(WindowEvent::KeyPressed {
                text: (*modifier).into(),
            });
        }
        dispatch(WindowEvent::KeyPressed { text: key.clone() });
        dispatch(WindowEvent::KeyReleased { text: key });
        for modifier in modifiers.iter().rev() {
            dispatch(WindowEvent::KeyReleased {
                text: (*modifier).into(),
            });
        }
    }

    pub fn ctrl(&self, key: &str) {
        self.press(&[Key::Control], key);
    }

    pub fn ctrl_shift(&self, key: &str) {
        self.press(&[Key::Control, Key::Shift], key);
    }

    /// Types `text` one key at a time.
    pub fn type_text(&self, text: &str) {
        for letter in text.chars() {
            self.press(&[], letter.to_string());
        }
    }

    /// Composes `preedit` with an input method, as it shows before it's committed; an empty
    /// `preedit` cancels the composition.
    pub fn compose(&self, preedit: &str) {
        self.app
            .window()
            .window()
            .dispatch_event(WindowEvent::internal(InternalKeyEvent {
                event_type: KeyEventType::UpdateComposition,
                preedit_text: preedit.into(),
                ..Default::default()
            }));
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

    /// What's on the clipboard, as pasting it into the palette's field shows; the palette closes
    /// again.
    pub fn clipboard(&self) -> String {
        self.ctrl("k");
        self.ctrl("v");
        let query = self
            .app
            .core()
            .palette()
            .expect("the palette is open")
            .query;
        self.press(&[], Key::Escape);
        query
    }

    /// Whether some text reads `text`.
    pub fn shows(&self, text: &str) -> bool {
        self.count(AccessibleRole::Text, text) > 0
    }
}

/// Simulated GPUs whose readings are changed before Prism gets them.
struct ChangedReadings(SimulatedGpus, fn(&mut GpuPerformanceDevice));

impl GpuTelemetryProvider for ChangedReadings {
    fn performance(&self) -> GpuPerformanceResponse {
        let mut performance = self.0.performance();
        for gpu in &mut performance.gpus {
            (self.1)(gpu);
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

/// The smallest size the window lets the desktop resize it to, as Slint hands it to winit. The
/// testing backend resizes to any size, so this reads what the desktop is asked to enforce.
pub fn minimum_size(window: &slint::Window) -> slint::LogicalSize {
    use i_slint_core::item_tree::ItemTreeRc;
    use i_slint_core::layout::{Orientation, min_max_size_for_layout_constraints};

    let component = i_slint_core::window::WindowInner::from_pub(window).component();
    let component = ItemTreeRc::borrow_pin(&component);
    let (minimum, _) = min_max_size_for_layout_constraints(
        component.as_ref().layout_info(Orientation::Horizontal),
        component.as_ref().layout_info(Orientation::Vertical),
    );
    minimum.expect("the window has a minimum size")
}
