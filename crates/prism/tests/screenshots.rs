//! Screenshot tests: the window drawn by Slint's software renderer at a fixed size and a scale
//! factor of 1, in Light and Dark, compared against the reference PNGs in `tests/screenshots/`.
//! They check layout and drawing, not FemtoVG's production pixels.
//!
//! They run in one pinned CI image, where the references come from: regenerate them with
//! `scripts/update-screenshots.sh`, which runs these tests there with `PRISM_UPDATE_SCREENSHOTS=1`.
//! Every test sits in the `screenshot` module so CI's other test job can skip them by name.

mod support;

mod screenshot {
    use std::time::Duration;

    use i_slint_backend_testing::AccessibleRole;
    use prism_gpu::GpuTelemetryProvider;
    use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
    use prism_ui::{AppearanceMode, PrismdInstall, UnitState};
    use slint::ComponentHandle;

    use crate::support::prismd::DaemonState;
    use crate::support::screenshot::{assert_screenshot, init_rendering_backend};
    use crate::support::{DEFAULT_SIZE, HALF_1080P, Harness, MINIMUM_SIZE};

    /// The GPU workspace with its five tracks.
    fn workspace(appearance: AppearanceMode, name: &str) {
        init_rendering_backend();
        let harness = Harness::with_appearance(SimulatedFixture::TwoIdenticalGpus, appearance);
        // Half a minute of readings, so every chart draws its series.
        harness.ticks(30);

        assert_screenshot(name, harness.app.window().window(), DEFAULT_SIZE);
    }

    #[test]
    fn the_workspace_in_light() {
        workspace(AppearanceMode::Light, "workspace-light");
    }

    #[test]
    fn the_workspace_in_dark() {
        workspace(AppearanceMode::Dark, "workspace-dark");
    }

    #[test]
    fn the_workspace_at_half_a_1080p_screen_in_light() {
        narrow_workspace(
            AppearanceMode::Light,
            "workspace-half-1080p-light",
            HALF_1080P,
        );
    }

    #[test]
    fn the_workspace_at_half_a_1080p_screen_in_dark() {
        narrow_workspace(
            AppearanceMode::Dark,
            "workspace-half-1080p-dark",
            HALF_1080P,
        );
    }

    #[test]
    fn the_workspace_at_the_minimum_size_in_light() {
        narrow_workspace(
            AppearanceMode::Light,
            "workspace-minimum-light",
            MINIMUM_SIZE,
        );
    }

    #[test]
    fn the_workspace_at_the_minimum_size_in_dark() {
        narrow_workspace(AppearanceMode::Dark, "workspace-minimum-dark", MINIMUM_SIZE);
    }

    /// The workspace with tuning access, connected `30` seconds into the tunable GPU's simulation.
    fn tunable_workspace(appearance: AppearanceMode) -> Harness {
        let harness = Harness::with_daemon_in(
            SimulatedFixture::TunableGpu,
            appearance,
            DaemonState::Running,
        );
        for second in 0..30 {
            harness.prismd_tick(
                SimulatedGpus::new(SimulatedFixture::TunableGpu, move || {
                    Duration::from_secs(second)
                })
                .performance(),
            );
        }
        harness
    }

    /// The workspace with tuning access and pending changes: a power limit, a graphics clock
    /// offset and the memory clock offset's default. The change strip with Discard and Apply, the
    /// pending limit's line and EDIT HOLD.
    fn pending_workspace(appearance: AppearanceMode, name: &str) {
        init_rendering_backend();
        let harness = tunable_workspace(appearance);
        harness
            .find(AccessibleRole::Spinbox, "Power limit")
            .set_accessible_value("450");
        harness
            .find(AccessibleRole::Spinbox, "Graphics clock offset")
            .set_accessible_value("150");
        harness
            .find(AccessibleRole::Button, "Use default memory clock offset")
            .invoke_accessible_default_action();

        assert_screenshot(name, harness.app.window().window(), DEFAULT_SIZE);
    }

    #[test]
    fn the_workspace_with_a_pending_power_limit_in_light() {
        pending_workspace(AppearanceMode::Light, "workspace-pending-light");
    }

    #[test]
    fn the_workspace_with_a_pending_power_limit_in_dark() {
        pending_workspace(AppearanceMode::Dark, "workspace-pending-dark");
    }

    /// Drivers after a driver job left the host needing a restart: the job's outcome, its log and
    /// Reboot, then the recommended driver plan with its warnings, and the host.
    fn drivers(appearance: AppearanceMode, name: &str) {
        init_rendering_backend();
        let harness = tunable_workspace(appearance);
        let status: prism_drivers::DriverStatusResponse = serde_json::from_str(include_str!(
            "../../prism-protocol/tests/contracts/driver-status.json"
        ))
        .unwrap();
        let jobs: Vec<prism_drivers::DriverJob> = serde_json::from_str(include_str!(
            "../../prism-protocol/tests/contracts/driver-jobs.json"
        ))
        .unwrap();
        harness.ctrl_shift("d");
        harness
            .prismd
            .answer(prism_protocol::DaemonResponse::DriverStatus(status.clone()));
        let ended = jobs
            .into_iter()
            .find(|job| job.status == prism_drivers::DriverJobStatus::RebootRequired)
            .unwrap();
        harness
            .prismd
            .push(prism_protocol::DaemonEvent::DriverJobChanged(ended));
        harness.app.receive_posted();
        // The job ending reads the plan again.
        harness
            .prismd
            .answer(prism_protocol::DaemonResponse::DriverStatus(status));
        harness.app.receive_posted();

        assert_screenshot(name, harness.app.window().window(), DEFAULT_SIZE);
    }

    #[test]
    fn drivers_after_a_driver_job_in_light() {
        drivers(AppearanceMode::Light, "drivers-reboot-required-light");
    }

    #[test]
    fn drivers_after_a_driver_job_in_dark() {
        drivers(AppearanceMode::Dark, "drivers-reboot-required-dark");
    }

    /// The workspace with fixed fan speed pending: the Fans track's fan control mode with the mode
    /// pending, and the fixed speed's stepper with the speed pending.
    fn pending_cooling(appearance: AppearanceMode, name: &str) {
        init_rendering_backend();
        let harness = tunable_workspace(appearance);
        harness
            .find(AccessibleRole::Button, "Fixed speed")
            .invoke_accessible_default_action();
        harness
            .find(AccessibleRole::Spinbox, "Fixed fan speed")
            .set_accessible_value("55");

        assert_screenshot(name, harness.app.window().window(), DEFAULT_SIZE);
    }

    #[test]
    fn the_workspace_with_a_pending_fixed_fan_speed_in_light() {
        pending_cooling(AppearanceMode::Light, "workspace-cooling-pending-light");
    }

    #[test]
    fn the_workspace_with_a_pending_fixed_fan_speed_in_dark() {
        pending_cooling(AppearanceMode::Dark, "workspace-cooling-pending-dark");
    }

    /// The workspace after a cooling fault: the Fans track explaining the paused curve and each
    /// fan's fallback with Retry, PAUSED in the sidebar and the problem indicator.
    fn cooling_fault(appearance: AppearanceMode, name: &str) {
        use prism_gpu::cooling::{
            CoolingConfiguration, CurveSuspension, FanRestoration, FanSetting, RestorationStatus,
            SuspensionKind,
        };
        use prism_gpu::fan_curve::FanCurvePoint;
        use prism_protocol::{DaemonEvent, GpuCoolingControl};

        init_rendering_backend();
        let harness = tunable_workspace(appearance);
        let curve = CoolingConfiguration::Shared {
            setting: FanSetting::Curve {
                points: vec![FanCurvePoint::new(45, 30), FanCurvePoint::new(85, 100)],
            },
        };
        harness
            .prismd
            .push(DaemonEvent::CoolingControlChanged(GpuCoolingControl {
                gpu_id: "GPU-5a1b0000-0000-4000-8000-000000000001".into(),
                revision: 2,
                applied: Some(curve.clone()),
                suspension: Some(CurveSuspension {
                    kind: SuspensionKind::Fault,
                    reason: "NVML couldn't read the GPU's temperature: Unknown Error".into(),
                    restoration: vec![
                        FanRestoration {
                            fan_index: 0,
                            status: RestorationStatus::Restored,
                            detail: None,
                        },
                        FanRestoration {
                            fan_index: 1,
                            status: RestorationStatus::Unverified,
                            detail: Some("the driver still reports manual control".into()),
                        },
                    ],
                }),
                recovery: None,
                remembered_curve: Some(curve),
            }));
        harness.app.receive_posted();

        assert_screenshot(name, harness.app.window().window(), DEFAULT_SIZE);
    }

    #[test]
    fn a_cooling_fault_in_light() {
        cooling_fault(AppearanceMode::Light, "workspace-cooling-fault-light");
    }

    #[test]
    fn a_cooling_fault_in_dark() {
        cooling_fault(AppearanceMode::Dark, "workspace-cooling-fault-dark");
    }

    /// The workspace in automatic switching's failure pause: the change strip saying it failed
    /// with Retry, PAUSED in the sidebar, and the status bar's item and problem indicator.
    fn switching_failure(appearance: AppearanceMode, name: &str) {
        use prism_protocol::{
            AutomaticSwitchingStatus, DaemonEvent, GpuSwitchingState, GpuSwitchingStatus,
            SwitchingCondition,
        };

        init_rendering_backend();
        let harness = tunable_workspace(appearance);
        harness.prismd.push(DaemonEvent::AutomaticSwitchingChanged(
            AutomaticSwitchingStatus {
                condition: SwitchingCondition::Ready {},
                gpus: vec![GpuSwitchingStatus {
                    gpu_id: "GPU-5a1b0000-0000-4000-8000-000000000001".into(),
                    state: GpuSwitchingState::Failed {
                        message: "Power limit is outside the current allowed range.".into(),
                    },
                    application: None,
                }],
            },
        ));
        harness.app.receive_posted();

        assert_screenshot(name, harness.app.window().window(), DEFAULT_SIZE);
    }

    #[test]
    fn a_switching_failure_in_light() {
        switching_failure(AppearanceMode::Light, "workspace-switching-failure-light");
    }

    #[test]
    fn a_switching_failure_in_dark() {
        switching_failure(AppearanceMode::Dark, "workspace-switching-failure-dark");
    }

    /// The workspace with fan 1's own fixed speed pending, after another session changed the
    /// GPU's cooling: the fan picker with per-fan settings pending, and the change strip asking
    /// for a review before Apply.
    fn per_fan_review(appearance: AppearanceMode, name: &str) {
        init_rendering_backend();
        let harness = tunable_workspace(appearance);
        let mut state = crate::support::prismd::snapshot().tuning_states[0].clone();
        state.reported.cooling = Some(prism_gpu::cooling::CoolingConfiguration::Shared {
            setting: prism_gpu::cooling::FanSetting::Automatic {},
        });
        harness
            .prismd
            .push(prism_protocol::DaemonEvent::TuningStateChanged(
                state.clone(),
            ));
        harness.app.receive_posted();
        harness
            .find(AccessibleRole::Button, "Fan 1")
            .invoke_accessible_default_action();
        harness
            .find(AccessibleRole::Button, "Fixed speed")
            .invoke_accessible_default_action();
        harness
            .find(AccessibleRole::Spinbox, "Fan 1 fixed speed")
            .set_accessible_value("55");
        state.revision = "elsewhere".into();
        state.reported.cooling = Some(prism_gpu::cooling::CoolingConfiguration::Shared {
            setting: prism_gpu::cooling::FanSetting::Fixed { speed_percent: 70 },
        });
        harness
            .prismd
            .push(prism_protocol::DaemonEvent::TuningStateChanged(state));
        harness.app.receive_posted();

        assert_screenshot(name, harness.app.window().window(), DEFAULT_SIZE);
    }

    #[test]
    fn per_fan_settings_held_for_a_cooling_review_in_light() {
        per_fan_review(AppearanceMode::Light, "workspace-per-fan-review-light");
    }

    #[test]
    fn per_fan_settings_held_for_a_cooling_review_in_dark() {
        per_fan_review(AppearanceMode::Dark, "workspace-per-fan-review-dark");
    }

    /// The workspace with a fan curve in effect and an edit of it pending: the Fans track's curve
    /// editor with the curve in effect behind the pending one, the full-speed boundary, the live
    /// marker, and the fan speed history beside it, or in its place when `size` is narrow.
    fn pending_fan_curve(appearance: AppearanceMode, name: &str, size: (u32, u32)) {
        init_rendering_backend();
        let harness = tunable_workspace(appearance);
        let mut state = crate::support::prismd::snapshot().tuning_states[0].clone();
        state.revision = "curve".into();
        state.reported.cooling = Some(prism_gpu::cooling::CoolingConfiguration::Shared {
            setting: prism_gpu::cooling::FanSetting::Curve {
                points: [(40, 30), (60, 50), (75, 80), (85, 100)]
                    .map(|(temperature, speed)| {
                        prism_gpu::fan_curve::FanCurvePoint::new(temperature, speed)
                    })
                    .to_vec(),
            },
        });
        harness
            .prismd
            .push(prism_protocol::DaemonEvent::TuningStateChanged(state));
        harness.app.receive_posted();
        let point = harness.find(AccessibleRole::Slider, "Fan curve point 2");
        for _ in 0..15 {
            point.invoke_accessible_increment_action();
        }

        assert_screenshot(name, harness.app.window().window(), size);
    }

    #[test]
    fn the_workspace_with_a_pending_fan_curve_in_light() {
        pending_fan_curve(
            AppearanceMode::Light,
            "workspace-fan-curve-light",
            DEFAULT_SIZE,
        );
    }

    #[test]
    fn the_workspace_with_a_pending_fan_curve_in_dark() {
        pending_fan_curve(
            AppearanceMode::Dark,
            "workspace-fan-curve-dark",
            DEFAULT_SIZE,
        );
    }

    #[test]
    fn the_workspace_with_a_pending_fan_curve_at_half_a_1080p_screen_in_light() {
        pending_fan_curve(
            AppearanceMode::Light,
            "workspace-fan-curve-half-1080p-light",
            HALF_1080P,
        );
    }

    #[test]
    fn the_workspace_with_a_pending_fan_curve_at_half_a_1080p_screen_in_dark() {
        pending_fan_curve(
            AppearanceMode::Dark,
            "workspace-fan-curve-half-1080p-dark",
            HALF_1080P,
        );
    }

    /// The review a stale Apply opens: the power limit changed outside Prism under a pending one.
    fn review(appearance: AppearanceMode, name: &str) {
        init_rendering_backend();
        let harness = tunable_workspace(appearance);
        harness
            .find(AccessibleRole::Spinbox, "Power limit")
            .set_accessible_value("450");
        harness
            .find(AccessibleRole::Button, "Apply")
            .invoke_accessible_default_action();
        let mut state = prism_protocol::GpuTuningState {
            revision: "changed".into(),
            ..crate::support::prismd::snapshot().tuning_states[0].clone()
        };
        state.reported.power_limit_watts = Some(500);
        harness
            .prismd
            .push(prism_protocol::DaemonEvent::TuningStateChanged(state));
        harness.prismd.answer(prism_protocol::DaemonResponse::Error(
            prism_gpu::GpuCommandError::new(
                prism_gpu::GpuCommandErrorCode::StaleRevision,
                "The GPU's tuning values changed since these changes were made.",
            ),
        ));
        harness.app.receive_posted();

        assert_screenshot(name, harness.app.window().window(), DEFAULT_SIZE);
    }

    #[test]
    fn the_review_of_a_stale_apply_in_light() {
        review(AppearanceMode::Light, "review-light");
    }

    #[test]
    fn the_review_of_a_stale_apply_in_dark() {
        review(AppearanceMode::Dark, "review-dark");
    }

    /// The workspace with the GPU's profiles in its header: Boost current and Quiet, the startup
    /// profile, with its dot.
    fn profiles_workspace(appearance: AppearanceMode) -> Harness {
        let harness = tunable_workspace(appearance);
        harness
            .prismd
            .push(prism_protocol::DaemonEvent::ProfilesChanged(
                serde_json::from_str(include_str!(
                    "../../prism-protocol/tests/contracts/profiles-state.json"
                ))
                .unwrap(),
            ));
        harness.app.receive_posted();
        harness
    }

    fn profiles(appearance: AppearanceMode, name: &str) {
        init_rendering_backend();
        let harness = profiles_workspace(appearance);

        assert_screenshot(name, harness.app.window().window(), DEFAULT_SIZE);
    }

    #[test]
    fn the_workspace_with_profiles_in_light() {
        profiles(AppearanceMode::Light, "workspace-profiles-light");
    }

    #[test]
    fn the_workspace_with_profiles_in_dark() {
        profiles(AppearanceMode::Dark, "workspace-profiles-dark");
    }

    /// The one review of a profile's out-of-range values: Cancel, Adjust profile and apply.
    fn profile_corrections(appearance: AppearanceMode, name: &str) {
        init_rendering_backend();
        let harness = profiles_workspace(appearance);
        harness
            .find(AccessibleRole::Button, "Apply profile Quiet")
            .invoke_accessible_default_action();
        let unattempted = Some(prism_gpu::TuningOutcome {
            status: prism_gpu::TuningOutcomeStatus::Unattempted,
            error: Some("out of range".into()),
        });
        let correction = |setting, saved_value, proposed_value, bound| {
            prism_protocol::ProfileRangeCorrection::Numeric(
                prism_protocol::NumericRangeCorrection {
                    setting,
                    saved_value,
                    proposed_value,
                    bound,
                },
            )
        };
        harness
            .prismd
            .answer(prism_protocol::DaemonResponse::ProfileApplication(
                prism_protocol::ProfileApplicationResponse {
                    rollback: None,
                    cooling: None,
                    power: unattempted.clone(),
                    graphics: unattempted.clone(),
                    memory: unattempted,
                    corrections: vec![
                        correction(
                            prism_protocol::ProfileSettingKey::PowerLimitWatts,
                            650,
                            600,
                            prism_protocol::RangeBound::Maximum,
                        ),
                        correction(
                            prism_protocol::ProfileSettingKey::GraphicsOffsetMhz,
                            -1200,
                            -1000,
                            prism_protocol::RangeBound::Minimum,
                        ),
                    ],
                },
            ));
        harness.app.receive_posted();

        assert_screenshot(name, harness.app.window().window(), DEFAULT_SIZE);
    }

    #[test]
    fn the_profile_correction_review_in_light() {
        profile_corrections(AppearanceMode::Light, "profile-corrections-light");
    }

    #[test]
    fn the_profile_correction_review_in_dark() {
        profile_corrections(AppearanceMode::Dark, "profile-corrections-dark");
    }

    /// The header's `⋯` menu open under its button, with every profile action.
    fn profile_menu(appearance: AppearanceMode, name: &str) {
        init_rendering_backend();
        let harness = profiles_workspace(appearance);
        harness
            .find(AccessibleRole::Button, "Profile actions")
            .invoke_accessible_default_action();

        assert_screenshot(name, harness.app.window().window(), DEFAULT_SIZE);
    }

    #[test]
    fn the_profile_menu_in_light() {
        profile_menu(AppearanceMode::Light, "profile-menu-light");
    }

    #[test]
    fn the_profile_menu_in_dark() {
        profile_menu(AppearanceMode::Dark, "profile-menu-dark");
    }

    /// Manage profiles: Quiet, the startup profile, and Boost, the current one, each with Rename
    /// and Delete.
    fn manage_profiles(appearance: AppearanceMode, name: &str) {
        init_rendering_backend();
        let harness = profiles_workspace(appearance);
        harness
            .find(AccessibleRole::Button, "Profile actions")
            .invoke_accessible_default_action();
        harness
            .find(AccessibleRole::ListItem, "Manage profiles…")
            .invoke_accessible_default_action();

        assert_screenshot(name, harness.app.window().window(), DEFAULT_SIZE);
    }

    #[test]
    fn manage_profiles_in_light() {
        manage_profiles(AppearanceMode::Light, "manage-profiles-light");
    }

    #[test]
    fn manage_profiles_in_dark() {
        manage_profiles(AppearanceMode::Dark, "manage-profiles-dark");
    }

    /// Saving as a new profile under a name another profile has: the clash shows under the field.
    fn save_as_new_profile(appearance: AppearanceMode, name: &str) {
        init_rendering_backend();
        let harness = profiles_workspace(appearance);
        harness
            .find(AccessibleRole::Button, "Profile actions")
            .invoke_accessible_default_action();
        harness
            .find(AccessibleRole::ListItem, "Save as new profile…")
            .invoke_accessible_default_action();
        harness.type_text("quiet");

        assert_screenshot(name, harness.app.window().window(), DEFAULT_SIZE);
    }

    #[test]
    fn saving_as_a_new_profile_in_light() {
        save_as_new_profile(AppearanceMode::Light, "save-as-new-profile-light");
    }

    #[test]
    fn saving_as_a_new_profile_in_dark() {
        save_as_new_profile(AppearanceMode::Dark, "save-as-new-profile-dark");
    }

    /// Games over the tunable GPU's profiles: automatic switching on, Dota 2 linked to Quiet, and
    /// Portal 2, not installed, while running games can't be recognized.
    fn games_section(appearance: AppearanceMode) -> Harness {
        use prism_protocol::{DaemonEvent, GameEntry, GameId, GamesState, RunningGamesState};

        let harness = profiles_workspace(appearance);
        harness.prismd.push(DaemonEvent::GamesChanged(GamesState {
            automatic_switching: true,
            games: vec![
                GameEntry {
                    game: GameId::Steam { app_id: 570 },
                    name: "Dota 2".into(),
                    installed: true,
                    profiles: [(
                        "GPU-5a1b0000-0000-4000-8000-000000000001".to_owned(),
                        "quiet".to_owned(),
                    )]
                    .into(),
                },
                GameEntry {
                    game: GameId::Steam { app_id: 620 },
                    name: "Portal 2".into(),
                    installed: false,
                    profiles: Default::default(),
                },
            ],
        }));
        harness.prismd.push(DaemonEvent::RunningGamesChanged(
            RunningGamesState::Unavailable {
                reason: "A complete host process scan could not be read.".into(),
            },
        ));
        harness.app.receive_posted();
        harness.ctrl_shift("g");
        harness
    }

    fn games(appearance: AppearanceMode, name: &str) {
        init_rendering_backend();
        let harness = games_section(appearance);

        assert_screenshot(name, harness.app.window().window(), DEFAULT_SIZE);
    }

    #[test]
    fn games_in_light() {
        games(AppearanceMode::Light, "games-light");
    }

    #[test]
    fn games_in_dark() {
        games(AppearanceMode::Dark, "games-dark");
    }

    /// Activity over the workspace: a startup outcome that applied in part, then prismd's recent
    /// events.
    fn activity(appearance: AppearanceMode, name: &str) {
        use prism_protocol::{ActivityEntry, DaemonEvent, StartupState};

        init_rendering_backend();
        let harness = profiles_workspace(appearance);
        let mut startup: StartupState = serde_json::from_str(include_str!(
            "../../prism-protocol/tests/contracts/startup-state.json"
        ))
        .unwrap();
        startup.outcomes[0].gpu_id = "GPU-5a1b0000-0000-4000-8000-000000000001".into();
        let entries: Vec<ActivityEntry> = serde_json::from_str(include_str!(
            "../../prism-protocol/tests/contracts/activity-entries.json"
        ))
        .unwrap();
        harness
            .prismd
            .push(DaemonEvent::StartupStateChanged(startup));
        for entry in entries {
            harness.prismd.push(DaemonEvent::ActivityRecorded(entry));
        }
        harness.app.receive_posted();
        harness.ctrl_shift("a");

        assert_screenshot(name, harness.app.window().window(), DEFAULT_SIZE);
    }

    #[test]
    fn activity_in_light() {
        activity(AppearanceMode::Light, "activity-light");
    }

    #[test]
    fn activity_in_dark() {
        activity(AppearanceMode::Dark, "activity-dark");
    }

    /// Add Steam game over Games, with Dota 2 and Portal 2 already added.
    fn add_steam_game(appearance: AppearanceMode, name: &str) {
        use prism_protocol::{DaemonResponse, SteamApp};

        init_rendering_backend();
        let harness = games_section(appearance);
        harness
            .find(AccessibleRole::Button, "Add Steam game…")
            .invoke_accessible_default_action();
        harness.prismd.answer(DaemonResponse::SteamApps(
            [
                (1086940, "Baldur's Gate 3"),
                (570, "Dota 2"),
                (1145360, "Hades"),
                (367520, "Hollow Knight"),
                (620, "Portal 2"),
            ]
            .map(|(app_id, name)| SteamApp {
                app_id,
                name: name.into(),
            })
            .into(),
        ));
        harness.app.receive_posted();

        assert_screenshot(name, harness.app.window().window(), DEFAULT_SIZE);
    }

    #[test]
    fn add_steam_game_in_light() {
        add_steam_game(AppearanceMode::Light, "add-steam-game-light");
    }

    #[test]
    fn add_steam_game_in_dark() {
        add_steam_game(AppearanceMode::Dark, "add-steam-game-dark");
    }

    /// Add program over Games, with a few running programs, one whose host location is unknown,
    /// and a script prismd may not recognize.
    fn add_program(appearance: AppearanceMode, name: &str) {
        use prism_protocol::{DaemonResponse, RunningProgram};

        init_rendering_backend();
        let harness = games_section(appearance);
        harness
            .find(AccessibleRole::Button, "Add program…")
            .invoke_accessible_default_action();
        let program = |pid, name: &str, path: Option<&str>, warning: Option<&str>| RunningProgram {
            pid,
            name: name.into(),
            path: path.map(Into::into),
            launcher: None,
            environment: Vec::new(),
            helper: false,
            recognition_warning: warning.map(Into::into),
        };
        harness.prismd.answer(DaemonResponse::RunningPrograms(vec![
            program(
                4100,
                "Balatro.exe",
                Some("/home/alice/Games/Balatro/Balatro.exe"),
                None,
            ),
            program(
                2001,
                "factorio",
                Some("/opt/games/factorio/bin/factorio"),
                None,
            ),
            program(
                3000,
                "game.exe",
                None,
                Some(
                    "The host program location could not be recovered; browse for the program \
                     file instead.",
                ),
            ),
            program(
                3100,
                "python3",
                Some("/usr/bin/python3"),
                Some(
                    "Scripts, virtual machines and emulators identify the runtime, not the \
                     individual game; recognition may be unreliable.",
                ),
            ),
        ]));
        harness.app.receive_posted();

        assert_screenshot(name, harness.app.window().window(), DEFAULT_SIZE);
    }

    #[test]
    fn add_program_in_light() {
        add_program(AppearanceMode::Light, "add-program-light");
    }

    #[test]
    fn add_program_in_dark() {
        add_program(AppearanceMode::Dark, "add-program-dark");
    }

    /// Closing with unapplied changes: "1 unapplied change", Discard and close, Keep editing.
    fn close_confirmation(appearance: AppearanceMode, name: &str) {
        init_rendering_backend();
        let harness = tunable_workspace(appearance);
        harness
            .find(AccessibleRole::Spinbox, "Power limit")
            .set_accessible_value("450");
        harness
            .find(AccessibleRole::Button, "Close")
            .invoke_accessible_default_action();

        assert_screenshot(name, harness.app.window().window(), DEFAULT_SIZE);
    }

    #[test]
    fn the_close_confirmation_in_light() {
        close_confirmation(AppearanceMode::Light, "close-confirmation-light");
    }

    #[test]
    fn the_close_confirmation_in_dark() {
        close_confirmation(AppearanceMode::Dark, "close-confirmation-dark");
    }

    /// The narrow workspace: no max/avg/min, and the fan speed history while the fans follow no
    /// curve.
    fn narrow_workspace(appearance: AppearanceMode, name: &str, size: (u32, u32)) {
        init_rendering_backend();
        let harness = Harness::with_appearance(SimulatedFixture::TwoIdenticalGpus, appearance);
        harness.ticks(30);

        assert_screenshot(name, harness.app.window().window(), size);
    }

    /// The workspace over the last minute, scrolled down to its open hardware details.
    fn hardware_details(appearance: AppearanceMode, name: &str) {
        init_rendering_backend();
        let harness = Harness::with_appearance(SimulatedFixture::TwoIdenticalGpus, appearance);
        // A minute and a half of readings, so the last minute leaves some out.
        harness.ticks(90);
        harness
            .find(
                i_slint_backend_testing::AccessibleRole::Button,
                "Show last 1 minute",
            )
            .invoke_accessible_default_action();
        harness
            .find(
                i_slint_backend_testing::AccessibleRole::Button,
                "Hardware details",
            )
            .invoke_accessible_default_action();
        let window = harness.app.window().window();
        window.set_size(slint::PhysicalSize::new(DEFAULT_SIZE.0, DEFAULT_SIZE.1));
        window.dispatch_event(slint::platform::WindowEvent::PointerScrolled {
            position: slint::LogicalPosition::new(700.0, 400.0),
            delta_x: 0.0,
            delta_y: -2000.0,
        });

        assert_screenshot(name, window, DEFAULT_SIZE);
    }

    #[test]
    fn the_hardware_details_in_light() {
        hardware_details(AppearanceMode::Light, "hardware-details-light");
    }

    #[test]
    fn the_hardware_details_in_dark() {
        hardware_details(AppearanceMode::Dark, "hardware-details-dark");
    }

    /// The palette over the workspace, with the open GPU's command dimmed.
    fn command_palette(appearance: AppearanceMode, name: &str) {
        init_rendering_backend();
        let harness = Harness::with_appearance(SimulatedFixture::TwoIdenticalGpus, appearance);
        harness.ticks(30);
        harness.ctrl("k");

        assert_screenshot(name, harness.app.window().window(), DEFAULT_SIZE);
    }

    #[test]
    fn the_command_palette_in_light() {
        command_palette(AppearanceMode::Light, "command-palette-light");
    }

    #[test]
    fn the_command_palette_in_dark() {
        command_palette(AppearanceMode::Dark, "command-palette-dark");
    }

    /// Settings → Daemon on an Ubuntu host without prismd: why it's needed and the commands that
    /// install it, each with a copy button.
    fn daemon_not_installed(appearance: AppearanceMode, name: &str) {
        init_rendering_backend();
        let harness = Harness::on_host(
            SimulatedFixture::TwoIdenticalGpus,
            appearance,
            PrismdInstall::Native(UnitState::NotFound),
        );
        harness.tick();
        harness.ctrl(",");
        harness.tick();
        harness
            .find(i_slint_backend_testing::AccessibleRole::ListItem, "Daemon")
            .invoke_accessible_default_action();
        harness.tick();

        assert_screenshot(name, harness.app.window().window(), DEFAULT_SIZE);
    }

    #[test]
    fn settings_daemon_without_prismd_in_light() {
        daemon_not_installed(AppearanceMode::Light, "daemon-not-installed-light");
    }

    #[test]
    fn settings_daemon_without_prismd_in_dark() {
        daemon_not_installed(AppearanceMode::Dark, "daemon-not-installed-dark");
    }

    /// Settings → Daemon connected to prismd without tuning access: the fix, then the
    /// connection's facts.
    fn daemon_not_allowed(appearance: AppearanceMode, name: &str) {
        init_rendering_backend();
        let harness = Harness::with_daemon_in(
            SimulatedFixture::TwoIdenticalGpus,
            appearance,
            crate::support::prismd::DaemonState::ReadOnly,
        );
        harness.tick();
        harness
            .find(
                i_slint_backend_testing::AccessibleRole::Button,
                "PRISMD · NOT ALLOWED",
            )
            .invoke_accessible_default_action();
        harness.tick();

        assert_screenshot(name, harness.app.window().window(), DEFAULT_SIZE);
    }

    #[test]
    fn settings_daemon_without_tuning_access_in_light() {
        daemon_not_allowed(AppearanceMode::Light, "daemon-not-allowed-light");
    }

    #[test]
    fn settings_daemon_without_tuning_access_in_dark() {
        daemon_not_allowed(AppearanceMode::Dark, "daemon-not-allowed-dark");
    }

    mod popup_surface {
        use super::*;

        slint::slint! {
            import { Theme, AppearanceMode } from "ui/theme.slint";
            import { PopupSurface, StyledText } from "ui/widgets.slint";

            export { Theme, AppearanceMode }

            export component PopupSurfaceDemo inherits Window {
                background: Theme.color.surface;
                default-font-family: Theme.font-ui;

                PopupSurface {
                    x: 24px;
                    y: 24px;
                    width: 184px;
                    height: 56px;

                    StyledText {
                        x: Theme.space.xl;
                        text: "Hard offset shadow";
                    }
                }
            }
        }

        fn popup_surface(mode: AppearanceMode, name: &str) {
            init_rendering_backend();
            let demo = PopupSurfaceDemo::new().unwrap();
            demo.global::<Theme>().set_mode(mode);
            demo.show().unwrap();

            assert_screenshot(name, demo.window(), (240, 112));
        }

        #[test]
        fn a_popup_surface_draws_its_hard_shadow_in_light() {
            popup_surface(AppearanceMode::Light, "popup-surface-light");
        }

        #[test]
        fn a_popup_surface_draws_its_hard_shadow_in_dark() {
            popup_surface(AppearanceMode::Dark, "popup-surface-dark");
        }
    }
}
