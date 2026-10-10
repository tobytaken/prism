//! The composition root: builds the window over the prism-ui core and performs its effects.

use std::cell::{Ref, RefCell};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use slint::{CloseRequestResponse, ComponentHandle, PlatformError};

use prism_ui::{Content, Core, Effect, Input, Intent, Section as CoreSection, TelemetrySource};

use crate::chart::series_commands;
use crate::environment::{Clock, Environment, Sampling};
use crate::sampler::{Sampler, SamplerReading};
use crate::sync::sync;
use crate::{AppState, Inbox, Intents, MainWindow, Section};

/// Prism's GUI: the main window over the core, wired to an [`Environment`].
pub struct App {
    inner: Rc<Inner>,
}

struct Inner {
    window: MainWindow,
    core: RefCell<Core>,
    clock: Arc<dyn Clock>,
    sampling: Sampling,
    /// The sampler before it starts, and afterwards while sampling is on demand.
    sampler: RefCell<Option<Sampler>>,
    /// Readings worker threads posted for the UI thread.
    inbox: Receiver<SamplerReading>,
    outbox: Sender<SamplerReading>,
}

impl App {
    /// Builds the window and launches the core, which starts sampling. The window isn't shown
    /// until [`App::run`].
    pub fn new(environment: Environment) -> Result<Self, PlatformError> {
        let Environment {
            gpus,
            // Neither is used until the GUI connection lands (#120, step 7).
            prismd: _,
            host: _,
            settings,
            clock,
            sampling,
        } = environment;
        let (outbox, inbox) = mpsc::channel();
        let inner = Rc::new(Inner {
            window: MainWindow::new()?,
            core: RefCell::default(),
            clock,
            sampling,
            sampler: RefCell::new(Some(Sampler::new(gpus))),
            inbox,
            outbox,
        });
        Inner::connect(&inner);
        inner.handle(Input::Launched {
            appearance: settings.load().appearance,
        });
        Ok(Self { inner })
    }

    pub fn window(&self) -> &MainWindow {
        &self.inner.window
    }

    pub fn core(&self) -> Ref<'_, Core> {
        self.inner.core.borrow()
    }

    /// Reads every GPU once on this thread and takes in the readings, as the sampler thread
    /// would; only for [`Sampling::OnDemand`].
    ///
    /// # Panics
    ///
    /// When sampling isn't on demand, or the core hasn't started the sampler.
    pub fn sample_gpus(&self) {
        assert_eq!(
            self.inner.sampling,
            Sampling::OnDemand,
            "only on-demand sampling reads when asked"
        );
        let readings = self
            .inner
            .sampler
            .borrow_mut()
            .as_mut()
            .expect("the core started the sampler")
            .read();
        for reading in readings {
            self.inner.receive(reading);
        }
    }

    /// Shows the window and runs the event loop until Prism quits.
    pub fn run(self) -> Result<(), PlatformError> {
        self.inner.window.run()
    }
}

impl Inner {
    fn connect(this: &Rc<Self>) {
        let window = &this.window;

        let inner = Rc::downgrade(this);
        window
            .global::<AppState>()
            .on_temperature_series(move |width, height, _revision| {
                with_inner(&inner, |inner| match inner.core.borrow().content() {
                    Content::Gpu(workspace) => {
                        series_commands(&workspace.temperature.chart, width, height).into()
                    }
                    _ => Default::default(),
                })
                .unwrap_or_default()
            });

        let intents = window.global::<Intents>();
        let inner = Rc::downgrade(this);
        intents.on_select_gpu(move |id| {
            with_inner(&inner, |inner| {
                inner.handle(Input::Intent(Intent::SelectGpu(id.into())))
            });
        });
        let inner = Rc::downgrade(this);
        intents.on_go_to(move |section| {
            let section = match section {
                Section::Games => CoreSection::Games,
                Section::Drivers => CoreSection::Drivers,
                Section::Settings => CoreSection::Settings,
            };
            with_inner(&inner, |inner| {
                inner.handle(Input::Intent(Intent::GoTo(section)))
            });
        });
        let inner = Rc::downgrade(this);
        intents.on_close_window(move || {
            with_inner(&inner, |inner| {
                inner.handle(Input::Intent(Intent::CloseWindow))
            });
        });
        let inner = Rc::downgrade(this);
        window.window().on_close_requested(move || {
            with_inner(&inner, |inner| {
                inner.handle(Input::Intent(Intent::CloseWindow))
            });
            // Quitting hides the window itself.
            CloseRequestResponse::KeepWindowShown
        });
        let window_handle = window.as_weak();
        intents.on_minimize_window(move || {
            if let Some(window) = window_handle.upgrade() {
                window.window().set_minimized(true);
            }
        });
        let window_handle = window.as_weak();
        intents.on_toggle_maximized(move || {
            if let Some(window) = window_handle.upgrade() {
                let window = window.window();
                window.set_maximized(!window.is_maximized());
            }
        });

        let inner = Rc::downgrade(this);
        window.global::<Inbox>().on_inputs_posted(move || {
            with_inner(&inner, |inner| {
                while let Ok(reading) = inner.inbox.try_recv() {
                    inner.receive(reading);
                }
            });
        });
    }

    /// Takes in one input, updates the view and performs the effects it calls for.
    fn handle(&self, input: Input) {
        let effects = self.core.borrow_mut().handle(input);
        sync(&self.window, &self.core.borrow());
        for effect in effects {
            self.perform(effect);
        }
    }

    /// Takes in a reading, stamped with the time it reached the UI thread.
    fn receive(&self, reading: SamplerReading) {
        self.handle(match reading {
            SamplerReading::Inventory(inventory) => Input::InventoryRead(inventory),
            SamplerReading::Telemetry(gpus) => Input::TelemetrySampled {
                gpus,
                source: TelemetrySource::InProcess,
                received_at: self.clock.now(),
            },
        });
    }

    fn perform(&self, effect: Effect) {
        match effect {
            Effect::StartSampler => self.start_sampler(),
            Effect::Quit => {
                let _ = self.window.hide();
                // There is no event loop to quit when the window never ran, as in tests.
                let _ = slint::quit_event_loop();
            }
        }
    }

    fn start_sampler(&self) {
        let Sampling::Every(interval) = self.sampling else {
            // On demand, the sampler stays here for `App::sample_gpus`.
            return;
        };
        let Some(sampler) = self.sampler.borrow_mut().take() else {
            return;
        };
        let outbox = self.outbox.clone();
        let window = self.window.as_weak();
        let started = sampler.spawn(interval, move |reading| {
            outbox.send(reading).is_ok()
                && window
                    .upgrade_in_event_loop(|window| window.global::<Inbox>().invoke_inputs_posted())
                    .is_ok()
        });
        if let Err(error) = started {
            eprintln!("prism: can't start the GPU sampler: {error}");
        }
    }
}

fn with_inner<T>(inner: &Weak<Inner>, f: impl FnOnce(&Inner) -> T) -> Option<T> {
    inner.upgrade().map(|inner| f(&inner))
}
