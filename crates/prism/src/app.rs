//! The composition root: builds the window over the prism-ui core and performs its effects.

use std::cell::{Ref, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use slint::{CloseRequestResponse, ComponentHandle, PlatformError, SharedString, TimerMode};

use prism_protocol::ProfileSettingKey;
use prism_ui::telemetry::{LimitKind, Metric, TelemetryRange};
use prism_ui::{
    ActivityAction, Content, Core, Dialog, Effect, FanCurveEditor, FanMode, Input, Intent, Key,
    PrismdHost, ProgramFile, Section as CoreSection, ServiceAction as CoreServiceAction,
    ServiceOutcome, SettingsSection as CoreSettingsSection, Shortcut, TelemetrySource, Timer,
    TrackChart,
};

use crate::chart::{area_commands, limit_commands, line_commands, series_commands};
use crate::environment::{Clock, Environment, HostServices, Launches, Sampling, WindowActivation};
use crate::file_chooser::FileChooser;
use crate::notifications::{NotificationEvent, Notifier};
use crate::prismd::{PrismdConnector, PrismdReport};
use crate::prismd_host::PrismdService;
use crate::sampler::{Sampler, SamplerReading, SamplerSwitch};
use crate::single_instance::APP_ID;
use crate::sync::sync;
use crate::{
    ActivityList, AppState, ChartId, ChartPart, CurvePart, Inbox, Intents, KeyPress, MainWindow,
    Section, ServiceAction, SettingsSection, TuningSetting,
};

/// Prism's GUI: the main window over the core, wired to an [`Environment`].
pub struct App {
    inner: Rc<Inner>,
}

struct Inner {
    window: MainWindow,
    core: RefCell<Core>,
    clock: Arc<dyn Clock>,
    sampling: Sampling,
    /// The sampler before its thread starts, and for good while sampling is on demand.
    sampler: RefCell<Option<Sampler>>,
    /// Whether the core has the sampler reading.
    sampler_switch: SamplerSwitch,
    prismd: Box<dyn PrismdConnector>,
    /// What the host says about prismd, and systemd enabling or restarting it.
    prismd_service: Box<dyn PrismdService>,
    /// Kept while Prism runs, since it holds the single-instance name.
    launches: Box<dyn Launches>,
    activation: Box<dyn WindowActivation>,
    notifier: Box<dyn Notifier>,
    file_chooser: Box<dyn FileChooser>,
    /// The core's timers, made the first time each starts.
    timers: RefCell<HashMap<Timer, slint::Timer>>,
    /// The selected GPU's charts and fan curve editor as of the core revision they were made at,
    /// so drawing every part of every chart derives them once.
    drawn: RefCell<Option<Drawn>>,
    /// What other threads posted for the UI thread.
    inbox: Receiver<Posted>,
    outbox: Sender<Posted>,
}

/// What the selected GPU's charts and fan curve editor draw, as of a core revision.
struct Drawn {
    revision: u64,
    charts: Vec<TrackChart>,
    fan_curve: Option<FanCurveEditor>,
}

/// Something another thread posted for the UI thread.
enum Posted {
    Reading(SamplerReading),
    /// Boxed, since a snapshot is far larger than anything else posted.
    Prismd(Box<PrismdReport>),
    /// Prism was launched again, with this activation token.
    LaunchedAgain(Option<String>),
    /// The host said this about prismd.
    PrismdHost(PrismdHost),
    /// A service action ended.
    ServiceDone(CoreServiceAction, ServiceOutcome),
    Notification(NotificationEvent),
    /// The file chooser closed, with the program file chosen, if any.
    ProgramFile(ProgramFile),
    /// The window gained or lost focus.
    Focused(bool),
    TimerElapsed(Timer),
}

impl App {
    /// Builds the window and launches the core, which starts sampling. The window isn't shown
    /// until [`App::run`].
    pub fn new(environment: Environment) -> Result<Self, PlatformError> {
        let Environment {
            gpus,
            prismd,
            host:
                HostServices {
                    prismd: prismd_service,
                    launches,
                    activation,
                    notifier,
                    file_chooser,
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
            sampler_switch: SamplerSwitch::default(),
            prismd,
            prismd_service,
            launches,
            activation,
            notifier,
            file_chooser,
            timers: RefCell::default(),
            drawn: RefCell::default(),
            inbox,
            outbox,
        });
        Inner::connect(&inner);
        let post = inner.poster();
        inner.launches.listen(Box::new(move |activation_token| {
            post(Posted::LaunchedAgain(activation_token))
        }));
        let post = inner.poster();
        inner
            .notifier
            .listen(Box::new(move |event| post(Posted::Notification(event))));
        let post = inner.poster();
        inner.activation.watch_focus(
            inner.window.window(),
            Box::new(move |focused| post(Posted::Focused(focused))),
        );
        inner.handle(Input::Launched {
            appearance: settings.load().appearance,
            started_unix_ms: inner.clock.started_unix_ms(),
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
    /// would, unless the core has sampling off; only for [`Sampling::OnDemand`].
    ///
    /// # Panics
    ///
    /// When sampling isn't on demand.
    pub fn sample_gpus(&self) {
        assert_eq!(
            self.inner.sampling,
            Sampling::OnDemand,
            "only on-demand sampling reads when asked"
        );
        if !self.inner.sampler_switch.is_on() {
            return;
        }
        let readings = self
            .inner
            .sampler
            .borrow_mut()
            .as_mut()
            .expect("on demand, the sampler stays on the UI thread")
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
        window.global::<AppState>().on_chart_path(
            move |chart, part, width, height, dash, gap, _revision| {
                with_inner(&inner, |inner| {
                    inner.chart_path(chart, part, (width, height), (dash, gap))
                })
                .unwrap_or_default()
            },
        );

        let inner = Rc::downgrade(this);
        window
            .global::<AppState>()
            .on_curve_path(move |part, width, height, _revision| {
                with_inner(&inner, |inner| inner.curve_path(part, (width, height)))
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
        intents.on_go_to_settings(move |section| {
            let section = match section {
                SettingsSection::Appearance => CoreSettingsSection::Appearance,
                SettingsSection::Startup => CoreSettingsSection::Startup,
                SettingsSection::Notifications => CoreSettingsSection::Notifications,
                SettingsSection::Daemon => CoreSettingsSection::Daemon,
                SettingsSection::About => CoreSettingsSection::About,
            };
            with_inner(&inner, |inner| {
                inner.handle(Input::Intent(Intent::GoToSettings(section)))
            });
        });
        let inner = Rc::downgrade(this);
        intents.on_run_service(move |action| {
            let action = match action {
                ServiceAction::Enable => CoreServiceAction::Enable,
                ServiceAction::Restart => CoreServiceAction::Restart,
            };
            with_inner(&inner, |inner| {
                inner.handle(Input::Intent(Intent::RunService(action)))
            });
        });
        let inner = Rc::downgrade(this);
        intents.on_copy(move |text| {
            with_inner(&inner, |inner| {
                inner.handle(Input::Intent(Intent::Copy(text.into())))
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
        intents.on_toggle_hardware_details(to_core(Intent::ToggleHardwareDetails));
        intents.on_apply_changes(to_core(Intent::ApplyChanges));
        intents.on_discard_changes(to_core(Intent::DiscardChanges));
        intents.on_discard_and_close(to_core(Intent::DiscardAndClose));
        intents.on_open_activity(to_core(Intent::OpenActivity));
        let inner = Rc::downgrade(this);
        intents.on_run_activity_action(move |list, index| {
            with_inner(&inner, |inner| {
                let action = activity_action(&inner.core.borrow(), list, index);
                if let Some(action) = action {
                    inner.handle(Input::Intent(Intent::RunActivityAction(action)));
                }
            });
        });
        intents.on_toggle_fan_curve(to_core(Intent::ToggleFanCurve));
        intents.on_recall_remembered_curve(to_core(Intent::RecallRememberedCurve));
        intents.on_confirm_profile(to_core(Intent::ConfirmProfile));
        intents.on_toggle_profile_menu(to_core(Intent::ToggleProfileMenu));
        let inner = Rc::downgrade(this);
        intents.on_run_profile_menu_entry(move |index| {
            if let Ok(index) = usize::try_from(index) {
                with_inner(&inner, |inner| {
                    inner.handle(Input::Intent(Intent::RunProfileMenuEntry(index)))
                });
            }
        });
        let inner = Rc::downgrade(this);
        intents.on_edit_profile_name(move |name| {
            with_inner(&inner, |inner| {
                inner.handle(Input::Intent(Intent::EditProfileName(name.into())))
            });
        });
        let inner = Rc::downgrade(this);
        intents.on_rename_profile(move |index| {
            with_inner(&inner, |inner| {
                let profile_id = managed_profile_id(&inner.core.borrow(), index);
                if let Some(profile_id) = profile_id {
                    inner.handle(Input::Intent(Intent::RenameProfile(profile_id)));
                }
            });
        });
        let inner = Rc::downgrade(this);
        intents.on_delete_profile(move |index| {
            with_inner(&inner, |inner| {
                let profile_id = managed_profile_id(&inner.core.borrow(), index);
                if let Some(profile_id) = profile_id {
                    inner.handle(Input::Intent(Intent::DeleteProfile(profile_id)));
                }
            });
        });
        let inner = Rc::downgrade(this);
        intents.on_set_automatic_switching(move |enabled| {
            with_inner(&inner, |inner| {
                inner.handle(Input::Intent(Intent::SetAutomaticSwitching(enabled)))
            });
        });
        let inner = Rc::downgrade(this);
        intents.on_link_game(move |game, link, choice| {
            with_inner(&inner, |inner| {
                // The rows list games, links and choices in the order the core gives them.
                let intent = {
                    let section = inner.core.borrow().games();
                    let at = |index: i32| usize::try_from(index).ok();
                    at(game)
                        .and_then(|game| section.games.get(game))
                        .and_then(|game| {
                            let link = game.links.get(at(link)?)?;
                            let choice = link.choices.get(at(choice)?)?;
                            Some(Intent::LinkGame {
                                game: game.game.clone(),
                                gpu_id: link.gpu_id.clone(),
                                profile_id: choice.profile_id.clone(),
                            })
                        })
                };
                if let Some(intent) = intent {
                    inner.handle(Input::Intent(intent));
                }
            });
        });
        let inner = Rc::downgrade(this);
        intents.on_remove_game(move |index| {
            with_inner(&inner, |inner| {
                let game = usize::try_from(index).ok().and_then(|index| {
                    Some(inner.core.borrow().games().games.get(index)?.game.clone())
                });
                if let Some(game) = game {
                    inner.handle(Input::Intent(Intent::RemoveGame(game)));
                }
            });
        });
        intents.on_add_steam_game(to_core(Intent::AddSteamGame));
        let inner = Rc::downgrade(this);
        intents.on_filter_steam_apps(move |query| {
            with_inner(&inner, |inner| {
                inner.handle(Input::Intent(Intent::FilterSteamApps(query.into())))
            });
        });
        let inner = Rc::downgrade(this);
        intents.on_add_steam_app(move |index| {
            with_inner(&inner, |inner| {
                // The rows list the apps in the order the core gives them.
                let app_id = match inner.core.borrow().dialog() {
                    Some(Dialog::AddSteamGame(add)) => usize::try_from(index)
                        .ok()
                        .and_then(|index| add.apps.get(index))
                        .map(|app| app.app_id),
                    _ => None,
                };
                if let Some(app_id) = app_id {
                    inner.handle(Input::Intent(Intent::AddSteamApp(app_id)));
                }
            });
        });
        intents.on_add_program(to_core(Intent::AddProgram));
        let inner = Rc::downgrade(this);
        intents.on_filter_programs(move |query| {
            with_inner(&inner, |inner| {
                inner.handle(Input::Intent(Intent::FilterPrograms(query.into())))
            });
        });
        let inner = Rc::downgrade(this);
        intents.on_add_running_program(move |index| {
            with_inner(&inner, |inner| {
                // The rows list the programs in the order the core gives them.
                let path = match inner.core.borrow().dialog() {
                    Some(Dialog::AddProgram(add)) => usize::try_from(index)
                        .ok()
                        .and_then(|index| add.programs.get(index)?.path.clone()),
                    _ => None,
                };
                if let Some(path) = path {
                    inner.handle(Input::Intent(Intent::AddRunningProgram(path)));
                }
            });
        });
        intents.on_browse_for_program(to_core(Intent::BrowseForProgram));
        let inner = Rc::downgrade(this);
        intents.on_apply_profile(move |index| {
            with_inner(&inner, |inner| {
                // The buttons list the GPU's profiles in the order the core gives them.
                let profile_id = match inner.core.borrow().content() {
                    Content::Gpu(workspace) => usize::try_from(index)
                        .ok()
                        .and_then(|index| workspace.profiles.choices.get(index))
                        .map(|choice| choice.id.clone()),
                    _ => None,
                };
                if let Some(profile_id) = profile_id {
                    inner.handle(Input::Intent(Intent::ApplyProfile(profile_id)));
                }
            });
        });
        let inner = Rc::downgrade(this);
        intents.on_drag_curve_point(move |point, x, y| {
            if let Ok(point) = usize::try_from(point) {
                with_inner(&inner, |inner| {
                    inner.handle(Input::Intent(Intent::DragCurvePoint { point, x, y }))
                });
            }
        });
        let inner = Rc::downgrade(this);
        intents.on_move_curve_point(move |point, temperature_by, speed_by| {
            if let Ok(point) = usize::try_from(point) {
                with_inner(&inner, |inner| {
                    inner.handle(Input::Intent(Intent::MoveCurvePoint {
                        point,
                        temperature_by: temperature_by.into(),
                        speed_by: speed_by.into(),
                    }))
                });
            }
        });
        let inner = Rc::downgrade(this);
        intents.on_stage(move |setting, value| {
            let intent = stage_intent(setting, value.into());
            with_inner(&inner, |inner| inner.handle(Input::Intent(intent)));
        });
        let inner = Rc::downgrade(this);
        intents.on_use_default(move |setting| {
            if let Some(intent) = use_default_intent(setting) {
                with_inner(&inner, |inner| inner.handle(Input::Intent(intent)));
            }
        });
        let inner = Rc::downgrade(this);
        intents.on_stage_fan_mode(move |index| {
            let mode = usize::try_from(index)
                .ok()
                .and_then(|index| FanMode::ALL.get(index));
            if let Some(&mode) = mode {
                with_inner(&inner, |inner| {
                    inner.handle(Input::Intent(Intent::StageFanMode(mode)))
                });
            }
        });
        let inner = Rc::downgrade(this);
        intents.on_pick_fans(move |index| {
            with_inner(&inner, |inner| {
                // The segments list the fan picker's choices in the order the core gives them.
                let fans = match inner.core.borrow().content() {
                    Content::Gpu(workspace) => usize::try_from(index).ok().and_then(|index| {
                        let arrangement = workspace.fan_mode()?.arrangement.as_ref()?;
                        Some(arrangement.choices.get(index)?.fan)
                    }),
                    _ => None,
                };
                if let Some(fans) = fans {
                    inner.handle(Input::Intent(Intent::PickFans(fans)));
                }
            });
        });
        let inner = Rc::downgrade(this);
        intents.on_drag_power_limit(move |y| {
            with_inner(&inner, |inner| {
                inner.handle(Input::Intent(Intent::DragPowerLimit(y)))
            });
        });
        let inner = Rc::downgrade(this);
        intents.on_show_range(move |index| {
            let range = usize::try_from(index)
                .ok()
                .and_then(|index| TelemetryRange::ALL.get(index));
            if let Some(&range) = range {
                with_inner(&inner, |inner| {
                    inner.handle(Input::Intent(Intent::ShowRange(range)))
                });
            }
        });
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

    /// A part of the selected GPU's chart as path commands for a chart `size` pixels, with
    /// `dashes` (dash and gap lengths) where the part is dashed.
    fn chart_path(
        &self,
        chart: ChartId,
        part: ChartPart,
        (width, height): (f32, f32),
        dashes: (f32, f32),
    ) -> SharedString {
        let metric = match chart {
            ChartId::Temperature => Metric::Temperature,
            ChartId::Utilization => Metric::Utilization,
            ChartId::PowerDraw => Metric::PowerDraw,
            ChartId::GraphicsClock => Metric::GraphicsClock,
            ChartId::MemoryClock => Metric::MemoryClock,
            ChartId::FanSpeed => Metric::FanSpeed,
        };
        let drawn = self.drawn();
        let Some(chart) = drawn.charts.iter().find(|chart| chart.metric == metric) else {
            return SharedString::default();
        };
        let limit = |kind, dashes| {
            chart
                .limits
                .iter()
                .find(|limit| limit.kind == kind)
                .map(|limit| limit_commands(limit.y, width, height, dashes))
                .unwrap_or_default()
        };
        match part {
            ChartPart::Series => series_commands(&chart.chart, width, height),
            ChartPart::Area => area_commands(&chart.chart, width, height),
            ChartPart::Limit => limit(LimitKind::Configured, None),
            ChartPart::EnforcedLimit => limit(LimitKind::Enforced, Some(dashes)),
            ChartPart::PendingLimit => limit(LimitKind::Pending, None),
        }
        .into()
    }

    /// A part of the selected GPU's fan curve editor as path commands for an editor `size` pixels.
    fn curve_path(&self, part: CurvePart, (width, height): (f32, f32)) -> SharedString {
        let drawn = self.drawn();
        let Some(curve) = &drawn.fan_curve else {
            return SharedString::default();
        };
        let line = match part {
            CurvePart::InEffect => &curve.in_effect_line,
            CurvePart::Pending => &curve.pending_line,
        };
        line.as_deref()
            .map(|points| line_commands(points, width, height))
            .unwrap_or_default()
            .into()
    }

    /// What the selected GPU's charts and fan curve editor draw now, derived once per core
    /// revision.
    fn drawn(&self) -> Ref<'_, Drawn> {
        let revision = self.core.borrow().revision();
        let stale = self
            .drawn
            .borrow()
            .as_ref()
            .is_none_or(|drawn| drawn.revision != revision);
        if stale {
            let (charts, fan_curve) = match self.core.borrow().content() {
                Content::Gpu(workspace) => {
                    let mut fan_curve = None;
                    let charts = workspace
                        .tracks
                        .into_iter()
                        .flat_map(|track| {
                            fan_curve = fan_curve.take().or(track.fan_curve);
                            track.charts
                        })
                        .collect();
                    (charts, fan_curve)
                }
                _ => (Vec::new(), None),
            };
            *self.drawn.borrow_mut() = Some(Drawn {
                revision,
                charts,
                fan_curve,
            });
        }
        Ref::map(self.drawn.borrow(), |drawn| {
            drawn.as_ref().expect("derived above")
        })
    }

    /// Whether the palette, a dialog or Activity is over the window.
    fn overlay_open(&self) -> bool {
        let core = self.core.borrow();
        core.palette_open()
            || core.dialog_open()
            || core.profile_menu_open()
            || core.activity_open()
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
        if shortcut == Shortcut::Dismiss && !self.overlay_open() {
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
        let overlay_was_open = self.overlay_open();
        let effects = self.core.borrow_mut().handle(input);
        sync(&self.window, &self.core.borrow());
        if overlay_was_open && !self.overlay_open() {
            // The palette's field or a dialog's button had the keyboard.
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
            Posted::Prismd(report) => match *report {
                PrismdReport::Connected {
                    welcome,
                    snapshot,
                    socket,
                } => Input::PrismdConnected {
                    welcome,
                    snapshot,
                    socket,
                },
                PrismdReport::NotConnected(reason) => Input::PrismdNotConnected(reason),
                PrismdReport::Event(event) => Input::PrismdEvent {
                    event,
                    received_at: self.clock.now(),
                },
                PrismdReport::Answered { id, response } => Input::PrismdAnswered { id, response },
            },
            Posted::LaunchedAgain(activation_token) => Input::LaunchedAgain { activation_token },
            Posted::PrismdHost(host) => Input::PrismdHostChecked(host),
            Posted::ServiceDone(action, outcome) => Input::PrismdServiceDone { action, outcome },
            Posted::Notification(NotificationEvent::Clicked {
                id,
                activation_token,
            }) => Input::NotificationClicked {
                id,
                activation_token,
            },
            Posted::Notification(NotificationEvent::Closed(id)) => Input::NotificationClosed(id),
            Posted::Focused(focused) => Input::WindowFocusChanged(focused),
            Posted::TimerElapsed(timer) => Input::TimerElapsed(timer),
            Posted::ProgramFile(file) => Input::ProgramFileChosen(file),
        });
    }

    fn perform(&self, effect: Effect) {
        match effect {
            Effect::ConnectPrismd => {
                let outbox = self.outbox.clone();
                let window = self.window.as_weak();
                self.prismd.start(Box::new(move |report| {
                    outbox.send(Posted::Prismd(Box::new(report))).is_ok() && wake(&window)
                }));
            }
            Effect::SendToPrismd(request) => self.prismd.send(request),
            Effect::CommandPrismd { id, request } => self.prismd.command(id, request),
            Effect::CheckPrismdHost => {
                let post = self.poster();
                self.prismd_service
                    .check(Box::new(move |host| post(Posted::PrismdHost(host))));
            }
            Effect::RunService(action) => {
                let post = self.poster();
                self.prismd_service.run(
                    action,
                    Box::new(move |outcome| post(Posted::ServiceDone(action, outcome))),
                );
            }
            Effect::Copy(text) => self.window.invoke_copy_to_clipboard(text.into()),
            Effect::StartSampler => self.start_sampler(),
            Effect::StopSampler => self.sampler_switch.set(false),
            Effect::RaiseWindow { activation_token } => {
                let window = self.window.window();
                window.set_minimized(false);
                if let Err(error) = self.window.show() {
                    eprintln!("prism: can't show the window: {error}");
                }
                self.activation
                    .activate(window, activation_token.as_deref());
            }
            Effect::StartTimer { timer, after } => {
                let post = self.poster();
                // Starting a running timer starts it over.
                self.timers.borrow_mut().entry(timer).or_default().start(
                    TimerMode::SingleShot,
                    after,
                    move || post(Posted::TimerElapsed(timer)),
                );
            }
            Effect::StopTimer(timer) => {
                if let Some(running) = self.timers.borrow().get(&timer) {
                    running.stop();
                }
            }
            Effect::Notify(notification) => self.notifier.notify(notification),
            Effect::WithdrawNotification(id) => self.notifier.withdraw(id),
            Effect::ChooseProgramFile => {
                let post = self.poster();
                self.file_chooser
                    .choose_program(Box::new(move |file| post(Posted::ProgramFile(file))));
            }
            Effect::Quit => {
                let _ = self.window.hide();
                // There is no event loop to quit when the window never ran, as in tests.
                let _ = slint::quit_event_loop();
            }
        }
    }

    /// Posts for the UI thread from any thread, and wakes it to take in what was posted.
    fn poster(&self) -> impl Fn(Posted) + Send + 'static {
        let outbox = self.outbox.clone();
        let window = self.window.as_weak();
        move |posted| {
            if outbox.send(posted).is_ok() {
                wake(&window);
            }
        }
    }

    /// Turns the sampler on, starting its thread the first time.
    fn start_sampler(&self) {
        self.sampler_switch.set(true);
        let Sampling::Every(interval) = self.sampling else {
            // On demand, the sampler stays here for `App::sample_gpus`.
            return;
        };
        let Some(sampler) = self.sampler.borrow_mut().take() else {
            return;
        };
        let outbox = self.outbox.clone();
        let window = self.window.as_weak();
        let started = sampler.spawn(interval, self.sampler_switch.clone(), move |reading| {
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

/// What staging `value` with the stepper of `setting` asks for.
fn stage_intent(setting: TuningSetting, value: i64) -> Intent {
    match setting {
        TuningSetting::PowerLimit => Intent::Stage(ProfileSettingKey::PowerLimitWatts, value),
        TuningSetting::GraphicsOffset => Intent::Stage(ProfileSettingKey::GraphicsOffsetMhz, value),
        TuningSetting::MemoryOffset => Intent::Stage(ProfileSettingKey::MemoryOffsetMhz, value),
        TuningSetting::FanSpeed => Intent::StageFanSpeed(value),
    }
}

/// What Use default on the stepper of `setting` asks for; the fixed fan speed has no default.
fn use_default_intent(setting: TuningSetting) -> Option<Intent> {
    let key = match setting {
        TuningSetting::PowerLimit => ProfileSettingKey::PowerLimitWatts,
        TuningSetting::GraphicsOffset => ProfileSettingKey::GraphicsOffsetMhz,
        TuningSetting::MemoryOffset => ProfileSettingKey::MemoryOffsetMhz,
        TuningSetting::FanSpeed => return None,
    };
    Some(Intent::UseDefault(key))
}

/// The id of the profile at `index` in the open Manage profiles, which lists them in the order the
/// core gives them.
/// What the button of the item at `index` in one of Activity's lists does.
fn activity_action(core: &Core, list: ActivityList, index: i32) -> Option<ActivityAction> {
    let panel = core.activity()?;
    let items = match list {
        ActivityList::Problems => panel.problems,
        ActivityList::Startup => panel.startup,
        ActivityList::Recent => panel.recent,
    };
    let item = items.into_iter().nth(usize::try_from(index).ok()?)?;
    item.action.map(|button| button.action)
}

fn managed_profile_id(core: &Core, index: i32) -> Option<String> {
    let Some(Dialog::ManageProfiles(manage)) = core.dialog() else {
        return None;
    };
    let index = usize::try_from(index).ok()?;
    manage.profiles.get(index).map(|profile| profile.id.clone())
}
