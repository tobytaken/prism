//! The composition root: builds the window over the prism-ui core and performs its effects.

use std::cell::{Ref, RefCell};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use slint::{CloseRequestResponse, ComponentHandle, PlatformError};

use prism_ui::{
    Content, Core, Effect, Input, Intent, Key, Section as CoreSection, Shortcut, TelemetrySource,
};

use crate::chart::series_commands;
use crate::environment::{Clock, Environment, HostServices, Launches, Sampling, WindowActivation};
use crate::sampler::{Sampler, SamplerReading};
use crate::single_instance::APP_ID;
use crate::sync::sync;
use crate::{AppState, Inbox, Intents, KeyPress, MainWindow, Section};

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
    /// Kept while Prism runs, since it holds the single-instance name.
    launches: Box<dyn Launches>,
    activation: Box<dyn WindowActivation>,
    /// What other threads posted for the UI thread.
    inbox: Receiver<Posted>,
    outbox: Sender<Posted>,
}

/// Something another thread posted for the UI thread.
enum Posted {
    Reading(SamplerReading),
    /// Prism was launched again, with this activation token.
    LaunchedAgain(Option<String>),
}

impl App {
    /// Builds the window and launches the core, which starts sampling. The window isn't shown
    /// until [`App::run`].
    pub fn new(environment: Environment) -> Result<Self, PlatformError> {
        let Environment {
            gpus,
            // Not used until the GUI connection lands (#120, step 7).
            prismd: _,
            host: HostServices {
                launches,
                activation,
            },
            settings,
            clock,
            sampling,
        } = environment;
        let window = MainWindow::new()?;
        // Desktops match the window to Prism's desktop file by this ID.
        slint::set_xdg_app_id(APP_ID)?;
        let (outbox, inbox) = mpsc::channel();
        let inner = Rc::new(Inner {
            window,
            core: RefCell::default(),
            clock,
            sampling,
            sampler: RefCell::new(Some(Sampler::new(gpus))),
            launches,
            activation,
            inbox,
            outbox,
        });
        Inner::connect(&inner);
        let outbox = inner.outbox.clone();
        let window = inner.window.as_weak();
        inner.launches.listen(Box::new(move |activation_token| {
            if outbox.send(Posted::LaunchedAgain(activation_token)).is_ok() {
                wake(&window);
            }
        }));
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
            self.inner.receive(Posted::Reading(reading));
        }
    }

    /// Takes in everything other threads posted, as the event loop does when they wake it; for
    /// tests, which run no event loop.
    pub fn receive_posted(&self) {
        self.inner.receive_posted();
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
        // A callback that hands the core `intent`.
        let to_core = |intent: Intent| {
            let inner = Rc::downgrade(this);
            move || {
                with_inner(&inner, |inner| inner.handle(Input::Intent(intent.clone())));
            }
        };
        intents.on_close_window(to_core(Intent::CloseWindow));
        intents.on_highlight_next(to_core(Intent::HighlightNext));
        intents.on_highlight_previous(to_core(Intent::HighlightPrevious));
        intents.on_run_highlighted(to_core(Intent::RunHighlighted));
        intents.on_dismiss(to_core(Intent::Dismiss));
        let inner = Rc::downgrade(this);
        intents.on_filter_palette(move |query| {
            with_inner(&inner, |inner| {
                inner.handle(Input::Intent(Intent::FilterPalette(query.into())))
            });
        });
        let inner = Rc::downgrade(this);
        intents.on_run_palette_entry(move |index| {
            with_inner(&inner, |inner| {
                if let Ok(index) = usize::try_from(index) {
                    inner.handle(Input::Intent(Intent::RunPaletteEntry(index)));
                }
            });
        });
        let inner = Rc::downgrade(this);
        intents.on_key_pressed(move |key, in_text_field| {
            with_inner(&inner, |inner| inner.key_pressed(key, in_text_field)).unwrap_or(false)
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
            with_inner(&inner, Inner::receive_posted);
        });
    }

    /// Runs the global shortcut `key` presses, if any, and says whether the key was spoken for.
    fn key_pressed(&self, key: KeyPress, in_text_field: bool) -> bool {
        let key = Key {
            text: key.text.into(),
            control: key.control,
            shift: key.shift,
            alt: key.alt,
            meta: key.meta,
        };
        let Some(shortcut) = Shortcut::for_key(&key) else {
            return false;
        };
        // With nothing to close, Escape is left to whoever else wants it.
        if shortcut == Shortcut::Dismiss && !self.core.borrow().palette_open() {
            return false;
        }
        // A focused text field keeps the keys it edits with.
        if !(in_text_field && shortcut.clashes_with_text_editing()) {
            self.handle(Input::Intent(Intent::Press(shortcut)));
        }
        true
    }

    /// Takes in one input, updates the view and performs the effects it calls for.
    fn handle(&self, input: Input) {
        let palette_was_open = self.core.borrow().palette_open();
        let effects = self.core.borrow_mut().handle(input);
        sync(&self.window, &self.core.borrow());
        if palette_was_open && !self.core.borrow().palette_open() {
            // The palette's field had the keyboard.
            self.window.invoke_focus_window();
        }
        for effect in effects {
            self.perform(effect);
        }
    }

    fn receive_posted(&self) {
        while let Ok(posted) = self.inbox.try_recv() {
            self.receive(posted);
        }
    }

    /// Takes in what another thread posted; a reading is stamped with the time it reached the UI
    /// thread.
    fn receive(&self, posted: Posted) {
        self.handle(match posted {
            Posted::Reading(SamplerReading::Inventory(inventory)) => {
                Input::InventoryRead(inventory)
            }
            Posted::Reading(SamplerReading::Telemetry(gpus)) => Input::TelemetrySampled {
                gpus,
                source: TelemetrySource::InProcess,
                received_at: self.clock.now(),
            },
            Posted::LaunchedAgain(activation_token) => Input::LaunchedAgain { activation_token },
        });
    }

    fn perform(&self, effect: Effect) {
        match effect {
            Effect::StartSampler => self.start_sampler(),
            Effect::RaiseWindow { activation_token } => {
                let window = self.window.window();
                window.set_minimized(false);
                if let Err(error) = self.window.show() {
                    eprintln!("prism: can't show the window: {error}");
                }
                self.activation
                    .activate(window, activation_token.as_deref());
            }
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
            outbox.send(Posted::Reading(reading)).is_ok() && wake(&window)
        });
        if let Err(error) = started {
            eprintln!("prism: can't start the GPU sampler: {error}");
        }
    }
}

/// Wakes the UI thread to take in what was posted; false once the event loop has ended.
fn wake(window: &slint::Weak<MainWindow>) -> bool {
    window
        .upgrade_in_event_loop(|window| window.global::<Inbox>().invoke_inputs_posted())
        .is_ok()
}

fn with_inner<T>(inner: &Weak<Inner>, f: impl FnOnce(&Inner) -> T) -> Option<T> {
    inner.upgrade().map(|inner| f(&inner))
}
