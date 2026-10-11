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

    /// The narrow workspace: no max/avg/min, and the fan curve's place in the Fans track.
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
