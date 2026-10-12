//! Copies the core's view models into the Slint globals after every input.

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use prism_protocol::ProfileSettingKey;
use prism_ui::format::{Readout, UNKNOWN};
use prism_ui::telemetry::{LimitKind, Metric, TelemetryRange};
use prism_ui::{
    ActivityItem, AppearanceMode as CoreAppearance, Content, CoolingProblem, Core, Dialog,
    DriverConfirmation, Fact, FanCurveEditor, FanModeControl, GameRow, ReviewChange,
    Section as CoreSection, ServiceAction as CoreServiceAction,
    SettingsSection as CoreSettingsSection, Stepper, SwitchingStatus, Track, Tunable,
};

use crate::{
    AboutView, ActivityItemView, AppState, AppearanceMode, ChangeStripView, ChartId, ChartView,
    ContentKind, CoolingProblemView, CurvePointView, DaemonView, DetailView, DialogKind,
    DialogView, DriversView, ExplainedStateView, FactView, FanCurveView, FanModeView, GameLinkView,
    GameView, GamesView, LimitMark, MainWindow, ManagedProfileView, MenuEntryView,
    PaletteEntryView, ProgramView, ReadoutView, ReviewChangeView, SegmentView, ServiceAction,
    SettingsSection, SidebarGpuView, SteamAppView, StepperView, SwitchingView, Theme, TrackView,
    TuningSetting, WorkspaceView,
};

pub(crate) fn sync(window: &MainWindow, core: &Core) {
    window.global::<Theme>().set_mode(match core.appearance() {
        CoreAppearance::System => AppearanceMode::System,
        CoreAppearance::Light => AppearanceMode::Light,
        CoreAppearance::Dark => AppearanceMode::Dark,
    });

    let state = window.global::<AppState>();
    let sidebar = core.sidebar();
    let gpus: Vec<_> = sidebar
        .gpus
        .iter()
        .map(|gpu| SidebarGpuView {
            id: gpu.id.as_str().into(),
            index: gpu.index.as_str().into(),
            name: gpu.name.as_str().into(),
            identity: gpu.identity.as_str().into(),
            label: gpu.label.as_str().into(),
            temperature: readout(&gpu.temperature),
            utilization: readout(&gpu.utilization),
            power_draw: readout(&gpu.power_draw),
            fan_speed: readout(&gpu.fan_speed),
            vitals: gpu.vitals().into(),
            paused: gpu.paused(),
            selected: gpu.selected,
        })
        .collect();
    if let Some(model) = update_in_place(&state.get_gpus(), gpus, PartialEq::eq) {
        state.set_gpus(model);
    }
    // Empty while there are GPUs, or before the inventory is read.
    state.set_no_gpus(
        sidebar
            .no_gpus
            .as_ref()
            .map(|no_gpus| ExplainedStateView {
                title: no_gpus.title.as_str().into(),
                reason: no_gpus.reason.as_str().into(),
                hint: no_gpus.hint.as_str().into(),
            })
            .unwrap_or_default(),
    );

    state.set_content(match core.content() {
        Content::Searching => ContentKind::Searching,
        Content::NoGpus(_) => ContentKind::NoGpus,
        Content::Gpu(workspace) => {
            let changes = &workspace.changes;
            let profiles = &workspace.profiles;
            state.set_workspace(WorkspaceView {
                id: workspace.id.as_str().into(),
                title: workspace.title.as_str().into(),
                subtitle: workspace.subtitle.as_str().into(),
                has_profiles: !profiles.choices.is_empty() || profiles.current.is_some(),
                profiles_read_only: profiles.read_only.is_some(),
                profiles_reason: profiles.read_only.as_deref().unwrap_or_default().into(),
                current_profile: profiles.current.as_deref().unwrap_or("None").into(),
                profiles_not_saved: profiles.not_saved_to_profile,
                changes: ChangeStripView {
                    summary: changes.summary.as_str().into(),
                    pending: changes.pending,
                    outcome: changes.outcome.as_deref().unwrap_or_default().into(),
                    review: changes.review.as_deref().unwrap_or_default().into(),
                    can_apply: changes.apply_unavailable.is_none(),
                    apply_reason: changes
                        .apply_unavailable
                        .as_deref()
                        .unwrap_or_default()
                        .into(),
                    can_discard: changes.discard_unavailable.is_none(),
                    discard_reason: changes
                        .discard_unavailable
                        .as_deref()
                        .unwrap_or_default()
                        .into(),
                },
                no_tuning_access: workspace
                    .no_tuning_access
                    .as_deref()
                    .unwrap_or_default()
                    .into(),
                has_switching: workspace.switching.is_some(),
                switching: workspace
                    .switching
                    .as_ref()
                    .map(switching_view)
                    .unwrap_or_default(),
                hardware_open: workspace.hardware.open,
                // Slint's int is 32 bits; wrapping still changes the value with every request.
                reveal_fans: workspace.reveal_fans as i32,
            });
            let current = state.get_tracks();
            let tracks: Vec<_> = workspace
                .tracks
                .iter()
                .enumerate()
                .map(|(index, track)| track_view(current.row_data(index), track))
                .collect();
            if let Some(model) = update_in_place(&current, tracks, PartialEq::eq) {
                state.set_tracks(model);
            }
            let ranges: Vec<_> = TelemetryRange::ALL
                .into_iter()
                .map(|range| SegmentView {
                    text: range.label().into(),
                    label: range.command().into(),
                    selected: range == workspace.range,
                    unavailable_reason: SharedString::new(),
                    mark: SharedString::new(),
                })
                .collect();
            if let Some(model) = update_in_place(&state.get_ranges(), ranges, PartialEq::eq) {
                state.set_ranges(model);
            }
            let choices: Vec<_> = profiles
                .choices
                .iter()
                .map(|choice| SegmentView {
                    text: choice.name.as_str().into(),
                    label: format!("Apply profile {}", choice.name).into(),
                    selected: choice.current,
                    unavailable_reason: choice.unavailable.as_deref().unwrap_or_default().into(),
                    mark: if choice.startup {
                        "Startup profile".into()
                    } else {
                        SharedString::new()
                    },
                })
                .collect();
            if let Some(model) = update_in_place(&state.get_profiles(), choices, PartialEq::eq) {
                state.set_profiles(model);
            }
            if let Some(model) = update_in_place(
                &state.get_hardware_inventory(),
                facts(&workspace.hardware.inventory),
                PartialEq::eq,
            ) {
                state.set_hardware_inventory(model);
            }
            if let Some(model) = update_in_place(
                &state.get_hardware_capabilities(),
                facts(&workspace.hardware.capabilities),
                PartialEq::eq,
            ) {
                state.set_hardware_capabilities(model);
            }
            ContentKind::Gpu
        }
        Content::Section(CoreSection::Games) => ContentKind::Games,
        Content::Section(CoreSection::Drivers) => ContentKind::Drivers,
        Content::Section(CoreSection::Settings) => ContentKind::Settings,
    });

    state.set_settings_section(match core.settings_section() {
        CoreSettingsSection::Appearance => SettingsSection::Appearance,
        CoreSettingsSection::Startup => SettingsSection::Startup,
        CoreSettingsSection::Notifications => SettingsSection::Notifications,
        CoreSettingsSection::Daemon => SettingsSection::Daemon,
        CoreSettingsSection::About => SettingsSection::About,
    });
    state.set_settings_section_name(core.settings_section().name().into());
    let modes: Vec<_> = CoreAppearance::ALL
        .into_iter()
        .map(|mode| SegmentView {
            text: mode.name().into(),
            label: mode.name().into(),
            selected: mode == core.appearance(),
            unavailable_reason: SharedString::new(),
            mark: SharedString::new(),
        })
        .collect();
    if let Some(model) = update_in_place(&state.get_appearance_modes(), modes, PartialEq::eq) {
        state.set_appearance_modes(model);
    }
    state.set_desktop_notifications(core.desktop_notifications());
    let about = core.about();
    state.set_about(AboutView {
        version: about.version.into(),
        license: about.license.into(),
        slint_website: about.slint_website.into(),
    });
    let daemon = core.daemon_settings();
    let action = daemon.action.as_ref();
    state.set_daemon(DaemonView {
        title: daemon.title.as_str().into(),
        has_action: action.is_some(),
        action: match action.map(|action| action.action) {
            Some(CoreServiceAction::Restart) => ServiceAction::Restart,
            Some(CoreServiceAction::Enable) | None => ServiceAction::Enable,
        },
        action_title: action
            .map(|action| action.title.as_str())
            .unwrap_or_default()
            .into(),
        action_busy: action.is_some_and(|action| action.busy),
        error: daemon.error.as_deref().unwrap_or_default().into(),
    });
    let strings = |lines: &[String]| lines.iter().map(SharedString::from).collect::<Vec<_>>();
    if let Some(model) = update_in_place(
        &state.get_daemon_paragraphs(),
        strings(&daemon.paragraphs),
        PartialEq::eq,
    ) {
        state.set_daemon_paragraphs(model);
    }
    if let Some(model) = update_in_place(
        &state.get_daemon_commands(),
        strings(&daemon.commands),
        PartialEq::eq,
    ) {
        state.set_daemon_commands(model);
    }
    if let Some(model) = update_in_place(
        &state.get_daemon_facts(),
        facts(&daemon.facts),
        PartialEq::eq,
    ) {
        state.set_daemon_facts(model);
    }

    let section = core.games();
    state.set_games_section(GamesView {
        switching_known: section.automatic_switching.is_some(),
        automatic_switching: section.automatic_switching.unwrap_or_default(),
        read_only: section.read_only.is_some(),
        reason: section.read_only.as_deref().unwrap_or_default().into(),
        recognition_problem: section
            .recognition_problem
            .as_deref()
            .unwrap_or_default()
            .into(),
        outcome: section.outcome.as_deref().unwrap_or_default().into(),
        empty: section.empty.as_str().into(),
    });
    let current = state.get_games();
    let games: Vec<_> = section
        .games
        .iter()
        .enumerate()
        .map(|(index, game)| game_view(current.row_data(index), game))
        .collect();
    if let Some(model) = update_in_place(&current, games, PartialEq::eq) {
        state.set_games(model);
    }

    let drivers = core.drivers();
    let plan = drivers.plan.as_ref();
    let job = drivers.job.as_ref();
    state.set_drivers(DriversView {
        read_only: drivers.read_only.is_some(),
        reason: drivers.read_only.as_deref().unwrap_or_default().into(),
        status: drivers.status.as_deref().unwrap_or_default().into(),
        outcome: drivers.outcome.as_deref().unwrap_or_default().into(),
        has_plan: plan.is_some(),
        plan_title: plan
            .map(|plan| plan.title.as_str())
            .unwrap_or_default()
            .into(),
        can_run: plan.is_some_and(|plan| plan.run_unavailable.is_none()),
        run_reason: plan
            .and_then(|plan| plan.run_unavailable.as_deref())
            .unwrap_or_default()
            .into(),
        has_job: job.is_some(),
        job_title: job.map(|job| job.title.as_str()).unwrap_or_default().into(),
        job_state: job.map(|job| job.state.as_str()).unwrap_or_default().into(),
        job_active: job.is_some_and(|job| job.active),
        job_failed: job.is_some_and(|job| job.failed),
        job_step: job
            .and_then(|job| job.step.as_deref())
            .unwrap_or_default()
            .into(),
        job_explanation: job
            .map(|job| job.explanation.join(" "))
            .unwrap_or_default()
            .into(),
        can_cancel: job.is_some_and(|job| job.cancel_unavailable.is_none()),
        cancel_reason: job
            .and_then(|job| job.cancel_unavailable.as_deref())
            .unwrap_or_default()
            .into(),
        reboot: job.is_some_and(|job| job.reboot),
        can_reboot: job.is_some_and(|job| job.reboot_unavailable.is_none()),
        reboot_reason: job
            .and_then(|job| job.reboot_unavailable.as_deref())
            .unwrap_or_default()
            .into(),
    });
    if let Some(model) = update_in_place(
        &state.get_driver_facts(),
        facts(&drivers.host),
        PartialEq::eq,
    ) {
        state.set_driver_facts(model);
    }
    if let Some(model) = update_in_place(
        &state.get_driver_warnings(),
        strings(&drivers.warnings),
        PartialEq::eq,
    ) {
        state.set_driver_warnings(model);
    }
    if let Some(model) = update_in_place(
        &state.get_plan_blockers(),
        plan.map(|plan| strings(&plan.blockers)).unwrap_or_default(),
        PartialEq::eq,
    ) {
        state.set_plan_blockers(model);
    }
    if let Some(model) = update_in_place(
        &state.get_plan_warnings(),
        plan.map(|plan| strings(&plan.warnings)).unwrap_or_default(),
        PartialEq::eq,
    ) {
        state.set_plan_warnings(model);
    }
    if let Some(model) = update_in_place(
        &state.get_plan_steps(),
        plan.map(|plan| strings(&plan.manual_steps))
            .unwrap_or_default(),
        PartialEq::eq,
    ) {
        state.set_plan_steps(model);
    }
    if let Some(model) = update_in_place(
        &state.get_driver_log(),
        job.map(|job| strings(&job.log)).unwrap_or_default(),
        PartialEq::eq,
    ) {
        state.set_driver_log(model);
    }

    let palette = core.palette();
    state.set_palette_open(palette.is_some());
    let entries: Vec<_> = palette
        .map(|palette| palette.entries)
        .unwrap_or_default()
        .iter()
        .map(|entry| PaletteEntryView {
            title: entry.title.as_str().into(),
            keys: ModelRc::new(VecModel::from(
                entry
                    .keys
                    .iter()
                    .map(SharedString::from)
                    .collect::<Vec<_>>(),
            )),
            available: entry.unavailable.is_none(),
            reason: entry.unavailable.as_deref().unwrap_or_default().into(),
            description: entry.description().into(),
            highlighted: entry.highlighted,
        })
        .collect();
    if let Some(model) = update_in_place(&state.get_palette_entries(), entries, same_entry) {
        state.set_palette_entries(model);
    }

    let menu = core.profile_menu();
    state.set_profile_menu_open(menu.is_some());
    let entries: Vec<_> = menu
        .map(|menu| menu.entries)
        .unwrap_or_default()
        .iter()
        .map(|entry| MenuEntryView {
            title: entry.title.as_str().into(),
            available: entry.unavailable.is_none(),
            reason: entry.unavailable.as_deref().unwrap_or_default().into(),
        })
        .collect();
    if let Some(model) = update_in_place(&state.get_profile_menu(), entries, PartialEq::eq) {
        state.set_profile_menu(model);
    }

    let mut managed = Vec::new();
    let mut steam_apps = Vec::new();
    let mut programs = Vec::new();
    let mut details = Vec::new();
    let (dialog, changes) = match core.dialog() {
        None => (DialogView::default(), Vec::new()),
        Some(Dialog::Review(review)) => (
            DialogView {
                kind: DialogKind::Review,
                title: review.title.into(),
                message: review.message.into(),
                apply_text: review.apply_text.into(),
                can_apply: review.apply_unavailable.is_none(),
                apply_reason: review.apply_unavailable.unwrap_or_default().into(),
                can_discard: review.discard_unavailable.is_none(),
                discard_reason: review.discard_unavailable.unwrap_or_default().into(),
                ..DialogView::default()
            },
            review.changes.iter().map(review_change).collect(),
        ),
        Some(Dialog::ConfirmClose(confirm)) => (
            DialogView {
                kind: DialogKind::ConfirmClose,
                title: confirm.summary.into(),
                message: confirm.message.into(),
                ..DialogView::default()
            },
            Vec::new(),
        ),
        Some(Dialog::DiscardForProfile(question)) => (
            DialogView {
                kind: DialogKind::DiscardForProfile,
                title: question.title.into(),
                message: question.message.into(),
                ..DialogView::default()
            },
            Vec::new(),
        ),
        Some(Dialog::ProfileCorrections(review)) => (
            DialogView {
                kind: DialogKind::ProfileCorrections,
                title: review.title.into(),
                message: review.message.into(),
                ..DialogView::default()
            },
            review.corrections.iter().map(review_change).collect(),
        ),
        Some(Dialog::ProfileName(naming)) => (
            DialogView {
                kind: DialogKind::ProfileName,
                title: naming.title.into(),
                message: naming.message.into(),
                name: naming.name.into(),
                error: naming.error.unwrap_or_default().into(),
                confirm_text: naming.confirm_text.into(),
                can_confirm: naming.confirm_unavailable.is_none(),
                confirm_reason: naming.confirm_unavailable.unwrap_or_default().into(),
                ..DialogView::default()
            },
            Vec::new(),
        ),
        Some(Dialog::DeleteProfile(question)) => (
            DialogView {
                kind: DialogKind::DeleteProfile,
                title: question.title.into(),
                message: question.message.into(),
                error: question.error.unwrap_or_default().into(),
                can_confirm: question.confirm_unavailable.is_none(),
                confirm_reason: question.confirm_unavailable.unwrap_or_default().into(),
                ..DialogView::default()
            },
            Vec::new(),
        ),
        Some(Dialog::ManageProfiles(manage)) => {
            managed = manage
                .profiles
                .iter()
                .map(|profile| ManagedProfileView {
                    name: profile.name.as_str().into(),
                    marks: profile.marks.as_str().into(),
                })
                .collect();
            (
                DialogView {
                    kind: DialogKind::ManageProfiles,
                    title: manage.title.into(),
                    message: manage.message.into(),
                    confirm_reason: manage.actions_unavailable.unwrap_or_default().into(),
                    ..DialogView::default()
                },
                Vec::new(),
            )
        }
        Some(Dialog::AddSteamGame(add)) => {
            steam_apps = add
                .apps
                .iter()
                .map(|app| SteamAppView {
                    name: app.name.as_str().into(),
                    available: app.unavailable.is_none(),
                    reason: app.unavailable.as_deref().unwrap_or_default().into(),
                })
                .collect();
            (
                DialogView {
                    kind: DialogKind::AddSteamGame,
                    title: add.title.into(),
                    message: add.message.into(),
                    name: add.filter.into(),
                    error: add.error.unwrap_or_default().into(),
                    status: add.status.unwrap_or_default().into(),
                    ..DialogView::default()
                },
                Vec::new(),
            )
        }
        Some(Dialog::AddProgram(add)) => {
            programs = add
                .programs
                .iter()
                .map(|program| ProgramView {
                    name: program.name.as_str().into(),
                    detail: program.detail.as_str().into(),
                    warning: program.warning.as_deref().unwrap_or_default().into(),
                    available: program.unavailable.is_none(),
                    reason: program.unavailable.as_deref().unwrap_or_default().into(),
                })
                .collect();
            (
                DialogView {
                    kind: DialogKind::AddProgram,
                    title: add.title.into(),
                    message: add.message.into(),
                    name: add.filter.into(),
                    error: add.error.unwrap_or_default().into(),
                    status: add.status.unwrap_or_default().into(),
                    can_confirm: add.can_browse,
                    ..DialogView::default()
                },
                Vec::new(),
            )
        }
        Some(Dialog::RunDriverPlan(confirmation)) => {
            details = strings(&confirmation.details);
            (
                driver_confirmation(DialogKind::RunDriverPlan, confirmation),
                Vec::new(),
            )
        }
        Some(Dialog::Reboot(confirmation)) => {
            details = strings(&confirmation.details);
            (
                driver_confirmation(DialogKind::Reboot, confirmation),
                Vec::new(),
            )
        }
    };
    if let Some(model) = update_in_place(&state.get_dialog_details(), details, PartialEq::eq) {
        state.set_dialog_details(model);
    }
    if let Some(model) = update_in_place(&state.get_managed_profiles(), managed, PartialEq::eq) {
        state.set_managed_profiles(model);
    }
    if let Some(model) = update_in_place(&state.get_steam_apps(), steam_apps, PartialEq::eq) {
        state.set_steam_apps(model);
    }
    if let Some(model) = update_in_place(&state.get_programs(), programs, PartialEq::eq) {
        state.set_programs(model);
    }
    state.set_dialog(dialog);
    if let Some(model) = update_in_place(&state.get_review_changes(), changes, PartialEq::eq) {
        state.set_review_changes(model);
    }

    let status = core.status_bar();
    state.set_connection(status.connection.into());
    state.set_connected(status.connected);
    state.set_telemetry(status.telemetry.into());
    state.set_switching(status.switching.unwrap_or_default().into());
    state.set_edit_hold(status.edit_hold);
    state.set_problems(status.problems.try_into().unwrap_or(i32::MAX));
    state.set_problem_indicator(status.problem_indicator.into());

    let activity = core.activity();
    state.set_activity_open(activity.is_some());
    let (problems, startup, recent, outcome, empty) = activity
        .map(|panel| {
            (
                panel.problems,
                panel.startup,
                panel.recent,
                panel.outcome.unwrap_or_default(),
                panel.empty.unwrap_or_default(),
            )
        })
        .unwrap_or_default();
    let items = |items: Vec<ActivityItem>| items.iter().map(activity_item_view).collect::<Vec<_>>();
    if let Some(model) = update_in_place(
        &state.get_activity_problems(),
        items(problems),
        PartialEq::eq,
    ) {
        state.set_activity_problems(model);
    }
    if let Some(model) =
        update_in_place(&state.get_activity_startup(), items(startup), PartialEq::eq)
    {
        state.set_activity_startup(model);
    }
    if let Some(model) = update_in_place(&state.get_activity_recent(), items(recent), PartialEq::eq)
    {
        state.set_activity_recent(model);
    }
    state.set_activity_outcome(outcome.into());
    state.set_activity_empty(empty.into());
    // Slint's int is 32 bits; wrapping still changes the value on every input.
    state.set_revision(core.revision() as i32);
}

/// The Run or Reboot confirmation as its dialog shows it, apart from its details.
fn driver_confirmation(kind: DialogKind, confirmation: DriverConfirmation) -> DialogView {
    DialogView {
        kind,
        title: confirmation.title.into(),
        message: confirmation.message.into(),
        error: confirmation.error.unwrap_or_default().into(),
        confirm_text: confirmation.confirm_text.into(),
        can_confirm: confirmation.confirm_unavailable.is_none(),
        confirm_reason: confirmation.confirm_unavailable.unwrap_or_default().into(),
        ..DialogView::default()
    }
}

/// A GPU's automatic switching status as its workspace shows it.
fn switching_view(status: &SwitchingStatus) -> SwitchingView {
    SwitchingView {
        summary: status.summary.as_str().into(),
        detail: status.detail.join(" ").into(),
        failed: status.failed,
        resumable: status.resumable,
        can_pause: status.pause_unavailable.is_none(),
        pause_reason: status
            .pause_unavailable
            .as_deref()
            .unwrap_or_default()
            .into(),
        can_retry: status.retry_unavailable.is_none(),
        retry_reason: status
            .retry_unavailable
            .as_deref()
            .unwrap_or_default()
            .into(),
        outcome: status.outcome.as_deref().unwrap_or_default().into(),
    }
}

/// A problem or an event as Activity shows it.
fn activity_item_view(item: &ActivityItem) -> ActivityItemView {
    let detail = item.detail.join("\n");
    let button = item.action.as_ref();
    ActivityItemView {
        title: item.title.as_str().into(),
        spoken: [
            item.when.clone().unwrap_or_default(),
            item.detail.join(" · "),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
        .into(),
        detail: detail.into(),
        when: item.when.as_deref().unwrap_or_default().into(),
        has_action: button.is_some(),
        action_title: button
            .map(|button| button.title.as_str())
            .unwrap_or_default()
            .into(),
        action_available: button.is_some_and(|button| button.unavailable.is_none()),
        action_reason: button
            .and_then(|button| button.unavailable.as_deref())
            .unwrap_or_default()
            .into(),
    }
}

/// A game as the Games section shows it. Its links and their choices go into the models `current`
/// has, if any, so Slint keeps their elements and the focus of the choice just made.
fn game_view(current: Option<GameView>, game: &GameRow) -> GameView {
    let current_links = current.map(|current| current.links);
    let links = game
        .links
        .iter()
        .enumerate()
        .map(|(index, link)| {
            let choices = link
                .choices
                .iter()
                .map(|choice| SegmentView {
                    text: choice.name.as_str().into(),
                    label: choice.name.as_str().into(),
                    selected: choice.linked,
                    unavailable_reason: SharedString::new(),
                    mark: SharedString::new(),
                })
                .collect();
            let current_choices = current_links
                .as_ref()
                .and_then(|links| links.row_data(index))
                .map(|current| current.choices);
            GameLinkView {
                label: format!("{} on {}", game.name, link.gpu).into(),
                gpu: link.gpu.as_str().into(),
                linked: link.linked.as_str().into(),
                choices: reuse(current_choices, choices),
            }
        })
        .collect();
    GameView {
        name: game.name.as_str().into(),
        detail: game.detail.as_str().into(),
        links: reuse(current_links, links),
    }
}

/// A changed value as a review shows it, with what the draft stages for it, if anything.
fn review_change(change: &ReviewChange) -> ReviewChangeView {
    ReviewChangeView {
        name: change.name.as_str().into(),
        change: change.change.as_str().into(),
        yours: change
            .yours
            .as_ref()
            .map(|yours| format!("Yours: {yours}"))
            .unwrap_or_default()
            .into(),
    }
}

/// A track as the view shows it. Its details and charts go into the models `current` has, if any,
/// so Slint keeps their elements.
fn track_view(current: Option<TrackView>, track: &Track) -> TrackView {
    let details = track
        .details
        .iter()
        .map(|detail| DetailView {
            label: detail.label.as_str().into(),
            name: detail.name.as_str().into(),
            readout: readout(&detail.readout),
            unknown_reason: detail.unknown_reason.as_deref().unwrap_or_default().into(),
            mark: match detail.limit {
                None => LimitMark::None,
                Some(LimitKind::Configured | LimitKind::Pending) => LimitMark::Solid,
                Some(LimitKind::Enforced) => LimitMark::Dashed,
            },
            has_stepper: detail.stepper.is_some(),
            stepper: detail
                .stepper
                .as_ref()
                .map(stepper_view)
                .unwrap_or_default(),
        })
        .collect();
    // The power limit's line can be dragged while its stepper can be changed.
    let editable = track
        .details
        .iter()
        .filter_map(|detail| detail.stepper.as_ref())
        .any(|stepper| {
            stepper.setting == Tunable::Setting(ProfileSettingKey::PowerLimitWatts)
                && stepper.read_only.is_none()
        });
    let charts = track
        .charts
        .iter()
        .map(|chart| {
            let line = |kind| chart.limits.iter().find(|limit| limit.kind == kind);
            let handle = line(LimitKind::Pending).or(line(LimitKind::Configured));
            ChartView {
                id: chart_id(chart.metric),
                label: chart.label.unwrap_or_default().into(),
                area: chart.area,
                limit: line(LimitKind::Configured).is_some(),
                enforced_limit: line(LimitKind::Enforced).is_some(),
                pending_limit: line(LimitKind::Pending).is_some(),
                draggable: editable && chart.metric == Metric::PowerDraw && handle.is_some(),
                handle_y: handle.map_or(0.0, |limit| limit.y),
            }
        })
        .collect();
    let [max_label, avg_label, min_label] = track.statistic_labels().map(SharedString::from);
    let current_fan_mode = current.as_ref().map(|current| current.fan_mode.clone());
    let current_points = current
        .as_ref()
        .map(|current| current.fan_curve.points.clone());
    TrackView {
        name: track.kind.name().into(),
        label: track.kind.name().to_uppercase().into(),
        readout: readout(&track.readout),
        readout_label: track.readout_label().into(),
        unknown_reason: track.unknown_reason.as_deref().unwrap_or_default().into(),
        details: reuse(
            current.as_ref().map(|current| current.details.clone()),
            details,
        ),
        charts: reuse(current.map(|current| current.charts), charts),
        range: track.range.into(),
        max: readout(&track.max),
        avg: readout(&track.avg),
        min: readout(&track.min),
        max_label,
        avg_label,
        min_label,
        has_fan_mode: track.fan_mode.is_some(),
        fan_mode: track
            .fan_mode
            .as_ref()
            .map(|fan_mode| fan_mode_view(current_fan_mode, fan_mode, track))
            .unwrap_or_default(),
        has_fan_curve: track.fan_curve.is_some(),
        fan_curve: track
            .fan_curve
            .as_ref()
            .map(|curve| fan_curve_view(current_points, curve))
            .unwrap_or_default(),
    }
}

/// The fan curve editor as the view shows it, its points in the model `current` has, if any, so
/// Slint keeps the point being dragged or with focus.
fn fan_curve_view(
    current: Option<ModelRc<CurvePointView>>,
    curve: &FanCurveEditor,
) -> FanCurveView {
    let points = curve
        .points
        .iter()
        .map(|point| CurvePointView {
            label: point.name.as_str().into(),
            value_text: point.value_text.as_str().into(),
            x: point.at.x,
            y: point.at.y,
            faulty: point.faulty,
        })
        .collect();
    FanCurveView {
        open: curve.open,
        points: reuse(current, points),
        pending: curve.pending,
        in_effect: curve.in_effect_line.is_some(),
        temperature_from: curve.temperature_labels[0].as_str().into(),
        temperature_to: curve.temperature_labels[1].as_str().into(),
        has_full_speed: curve.full_speed_x.is_some(),
        full_speed_x: curve.full_speed_x.unwrap_or_default(),
        has_marker: curve.marker.is_some(),
        marker_x: curve.marker.as_ref().map_or(0.0, |marker| marker.at.x),
        marker_y: curve.marker.as_ref().map_or(0.0, |marker| marker.at.y),
        marker_text: curve
            .marker
            .as_ref()
            .map(|marker| marker.text.as_str())
            .unwrap_or_default()
            .into(),
        problems: curve.problems_text().into(),
        read_only: curve.read_only.is_some(),
        reason: curve.read_only.as_deref().unwrap_or_default().into(),
        speed_minimum: clamped(curve.speeds.0),
        speed_maximum: clamped(curve.speeds.1),
    }
}

/// The fan control mode as the view shows it, its segments in the model `current` has, if any.
fn fan_mode_view(
    current: Option<FanModeView>,
    fan_mode: &FanModeControl,
    track: &Track,
) -> FanModeView {
    let modes = fan_mode
        .modes
        .iter()
        .map(|choice| SegmentView {
            text: choice.mode.short_name().into(),
            label: choice.name.into(),
            selected: choice.selected,
            unavailable_reason: choice.unavailable.as_deref().unwrap_or_default().into(),
            mark: SharedString::new(),
        })
        .collect();
    let picked = fan_mode.modes.iter().find(|choice| choice.selected);
    let arrangement = fan_mode.arrangement.as_ref();
    let fans = arrangement
        .map(|arrangement| {
            arrangement
                .choices
                .iter()
                .map(|choice| SegmentView {
                    text: choice.text.as_str().into(),
                    label: choice.name.as_str().into(),
                    selected: choice.selected,
                    unavailable_reason: choice.unavailable.as_deref().unwrap_or_default().into(),
                    mark: SharedString::new(),
                })
                .collect()
        })
        .unwrap_or_default();
    let (current_modes, current_fans) = match current {
        Some(current) => (Some(current.modes), Some(current.fans)),
        None => (None, None),
    };
    FanModeView {
        modes: reuse(current_modes, modes),
        mode: picked
            .map_or(UNKNOWN, |choice| choice.mode.short_name())
            .into(),
        mode_text: picked.map_or("unknown", |choice| choice.name).into(),
        has_fixed_speed: track
            .details
            .iter()
            .filter_map(|detail| detail.stepper.as_ref())
            .any(|stepper| stepper.setting == Tunable::FixedFanSpeed),
        pending: fan_mode.pending.as_deref().unwrap_or_default().into(),
        failure: fan_mode.failed.as_deref().unwrap_or_default().into(),
        read_only: fan_mode.read_only.is_some(),
        reason: fan_mode.read_only.as_deref().unwrap_or_default().into(),
        note: fan_mode.note.as_deref().unwrap_or_default().into(),
        has_arrangement: arrangement.is_some(),
        arrangement_pending: arrangement
            .and_then(|arrangement| arrangement.pending.as_deref())
            .unwrap_or_default()
            .into(),
        fans: reuse(current_fans, fans),
        can_recall: fan_mode.can_recall,
        has_problem: fan_mode.problem.is_some(),
        problem: fan_mode
            .problem
            .as_ref()
            .map(cooling_problem_view)
            .unwrap_or_default(),
    }
}

/// Why Prism stopped following a GPU's fan curve, as the Fans track shows it.
fn cooling_problem_view(problem: &CoolingProblem) -> CoolingProblemView {
    CoolingProblemView {
        title: problem.title.as_str().into(),
        detail: problem.detail.join(" ").into(),
        action: problem.action.as_str().into(),
        available: problem.unavailable.is_none(),
        reason: problem.unavailable.as_deref().unwrap_or_default().into(),
        outcome: problem.outcome.as_deref().unwrap_or_default().into(),
    }
}

fn stepper_view(stepper: &Stepper) -> StepperView {
    StepperView {
        setting: match stepper.setting {
            Tunable::Setting(ProfileSettingKey::PowerLimitWatts) => TuningSetting::PowerLimit,
            Tunable::Setting(ProfileSettingKey::GraphicsOffsetMhz) => TuningSetting::GraphicsOffset,
            Tunable::Setting(ProfileSettingKey::MemoryOffsetMhz) => TuningSetting::MemoryOffset,
            Tunable::FixedFanSpeed => TuningSetting::FanSpeed,
        },
        label: stepper.name.as_str().into(),
        value: clamped(stepper.value),
        value_text: stepper.value_text.as_str().into(),
        minimum: clamped(stepper.min),
        maximum: clamped(stepper.max),
        step: clamped(stepper.step),
        text: stepper.readout.text().into(),
        pending: stepper.pending.as_deref().unwrap_or_default().into(),
        read_only: stepper.read_only.is_some(),
        reason: stepper.read_only.as_deref().unwrap_or_default().into(),
        can_use_default: stepper.can_use_default,
        default_label: format!("Use default {}", stepper.name.to_lowercase()).into(),
        failure: stepper.failed.as_deref().unwrap_or_default().into(),
    }
}

/// `value` within Slint's 32-bit int.
fn clamped(value: i64) -> i32 {
    value.clamp(i32::MIN.into(), i32::MAX.into()) as i32
}

/// The chart the view asks to draw `metric` in.
fn chart_id(metric: Metric) -> ChartId {
    match metric {
        Metric::Temperature => ChartId::Temperature,
        Metric::Utilization => ChartId::Utilization,
        Metric::PowerDraw => ChartId::PowerDraw,
        Metric::GraphicsClock => ChartId::GraphicsClock,
        Metric::MemoryClock => ChartId::MemoryClock,
        Metric::FanSpeed => ChartId::FanSpeed,
    }
}

fn facts(facts: &[Fact]) -> Vec<FactView> {
    facts
        .iter()
        .map(|fact| FactView {
            label: fact.label.as_str().into(),
            value: fact.value.as_str().into(),
        })
        .collect()
}

/// `rows` in `model`, updated in place, when there is one this module made; otherwise in a new
/// model.
fn reuse<T: Clone + PartialEq + 'static>(model: Option<ModelRc<T>>, rows: Vec<T>) -> ModelRc<T> {
    match model {
        Some(model) => update_in_place(&model, rows, PartialEq::eq).unwrap_or(model),
        None => ModelRc::new(VecModel::from(rows)),
    }
}

/// Updates the rows of a model this module made, so Slint keeps the elements built for unchanged
/// rows (`same`) and their focus; returns a new model to set when `current` is another kind.
fn update_in_place<T: Clone + 'static>(
    current: &ModelRc<T>,
    rows: Vec<T>,
    same: impl Fn(&T, &T) -> bool,
) -> Option<ModelRc<T>> {
    let Some(model) = current.as_any().downcast_ref::<VecModel<T>>() else {
        return Some(ModelRc::new(VecModel::from(rows)));
    };
    while model.row_count() > rows.len() {
        model.remove(model.row_count() - 1);
    }
    let mut changed = false;
    for (index, row) in rows.into_iter().enumerate() {
        match model.row_data(index) {
            Some(existing) if same(&existing, &row) => {}
            Some(_) => {
                model.set_row_data(index, row);
                changed = true;
            }
            None => model.push(row),
        }
    }
    // Slint 1.18 drops a changed row of a list inside another view model, such as a track's
    // details, when that view model changed too and nothing has drawn since: the list's repeater
    // waits to update the row, then finds its model the same and skips it. Adding and removing a
    // row makes it update every row it's waiting on, keeping the others and their focus.
    if changed && let Some(last) = model.row_data(model.row_count() - 1) {
        model.push(last);
        model.remove(model.row_count() - 1);
    }
    None
}

/// Whether two palette entries show the same, comparing their keys by content: models compare by
/// identity.
fn same_entry(a: &PaletteEntryView, b: &PaletteEntryView) -> bool {
    a.title == b.title
        && a.keys.iter().eq(b.keys.iter())
        && a.available == b.available
        && a.reason == b.reason
        && a.description == b.description
        && a.highlighted == b.highlighted
}

fn readout(readout: &Readout) -> ReadoutView {
    ReadoutView {
        value: readout.value.as_str().into(),
        unit: readout.unit.into(),
        suffix: readout.suffix().into(),
        known: readout.is_known(),
        text: readout.text().into(),
        spoken: readout.spoken().into(),
    }
}
