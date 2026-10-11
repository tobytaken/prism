//! Screenshot tests: the window drawn by Slint's software renderer at a fixed size and a scale
//! factor of 1, in Light and Dark, compared against the reference PNGs in `tests/screenshots/`.
//! They check layout and drawing, not FemtoVG's production pixels.
//!
//! They run in one pinned CI image, where the references come from: regenerate them with
//! `scripts/update-screenshots.sh`, which runs these tests there with `PRISM_UPDATE_SCREENSHOTS=1`.
//! Every test sits in the `screenshot` module so CI's other test job can skip them by name.

mod support;

mod screenshot {
    use prism_gpu::simulated::SimulatedFixture;
    use prism_ui::{AppearanceMode, PrismdInstall, UnitState};
    use slint::ComponentHandle;

    use crate::support::Harness;
    use crate::support::screenshot::{assert_screenshot, init_rendering_backend};

    /// Prism's default window size.
    const WINDOW: (u32, u32) = (1280, 800);

    /// The GPU workspace with its five tracks.
    fn workspace(appearance: AppearanceMode, name: &str) {
        init_rendering_backend();
        let harness = Harness::with_appearance(SimulatedFixture::TwoIdenticalGpus, appearance);
        // Half a minute of readings, so every chart draws its series.
        harness.ticks(30);

        assert_screenshot(name, harness.app.window().window(), WINDOW);
    }

    #[test]
    fn the_workspace_in_light() {
        workspace(AppearanceMode::Light, "workspace-light");
    }

    #[test]
    fn the_workspace_in_dark() {
        workspace(AppearanceMode::Dark, "workspace-dark");
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
        window.set_size(slint::PhysicalSize::new(WINDOW.0, WINDOW.1));
        window.dispatch_event(slint::platform::WindowEvent::PointerScrolled {
            position: slint::LogicalPosition::new(700.0, 400.0),
            delta_x: 0.0,
            delta_y: -2000.0,
        });

        assert_screenshot(name, window, WINDOW);
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

        assert_screenshot(name, harness.app.window().window(), WINDOW);
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

        assert_screenshot(name, harness.app.window().window(), WINDOW);
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

        assert_screenshot(name, harness.app.window().window(), WINDOW);
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
