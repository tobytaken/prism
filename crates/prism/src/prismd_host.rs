//! What the host says about prismd and what systemd does with it: `systemctl show` natively,
//! whether `/run/prism` is visible in Flatpak, the loaded NVIDIA driver's version, and
//! `systemctl enable --now` or `restart` under polkit. Each runs on a thread of its own, since systemctl waits for polkit's prompt.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;

use prism_protocol::client;
use prism_ui::{
    Distro, PrismdHost, PrismdInstall, ServiceAction, ServiceOutcome, UnitState,
    nvidia_driver_version,
};

/// Asks the host about prismd, and has systemd enable or restart it.
pub trait PrismdService {
    /// Asks the host about prismd, calling `checked` with the answer from any thread.
    fn check(&self, checked: Box<dyn FnOnce(PrismdHost) + Send>);

    /// Has systemd run `action` on prismd.service under polkit, calling `done` with how it ended
    /// from any thread.
    fn run(&self, action: ServiceAction, done: Box<dyn FnOnce(ServiceOutcome) + Send>);
}

/// The host Prism runs on, natively or in a Flatpak sandbox.
#[derive(Debug, Clone)]
pub struct HostPrismdService {
    /// prismd's runtime directory, which holds its socket.
    run_dir: PathBuf,
}

impl HostPrismdService {
    pub fn new(run_dir: impl Into<PathBuf>) -> Self {
        Self {
            run_dir: run_dir.into(),
        }
    }

    /// The directory of the socket Prism connects to.
    pub fn production() -> Self {
        let socket = PathBuf::from(client::socket_path());
        Self::new(socket.parent().unwrap_or(Path::new("/run/prism")))
    }

    fn read(&self) -> PrismdHost {
        // Flatpak puts this file in every sandbox.
        let flatpak = Path::new("/.flatpak-info").exists();
        let os_release = if flatpak {
            vec!["/run/host/os-release"]
        } else {
            vec!["/etc/os-release", "/usr/lib/os-release"]
        };
        let distro = os_release
            .into_iter()
            .find_map(|path| fs::read_to_string(path).ok())
            .map(|contents| Distro::from_os_release(&contents))
            .unwrap_or_else(|| Distro::from_os_release(""));
        let install = if flatpak {
            PrismdInstall::Flatpak {
                run_dir_visible: self.run_dir.is_dir(),
            }
        } else {
            PrismdInstall::Native(unit_state())
        };
        // Flatpak leaves /sys/module out of the sandbox, but not /proc/driver/nvidia.
        let nvidia_driver = ["/sys/module/nvidia/version", "/proc/driver/nvidia/version"]
            .into_iter()
            .find_map(|path| nvidia_driver_version(&fs::read_to_string(path).ok()?));
        PrismdHost {
            install,
            distro,
            nvidia_driver,
        }
    }
}

impl PrismdService for HostPrismdService {
    fn check(&self, checked: Box<dyn FnOnce(PrismdHost) + Send>) {
        let host = self.clone();
        let started = thread::Builder::new()
            .name("prism-host-check".into())
            .spawn(move || checked(host.read()));
        if let Err(error) = started {
            eprintln!("prism: can't check prismd on the host: {error}");
        }
    }

    fn run(&self, action: ServiceAction, done: Box<dyn FnOnce(ServiceOutcome) + Send>) {
        let started = thread::Builder::new()
            .name("prism-systemctl".into())
            .spawn(move || {
                let mut ended = ServiceOutcome::Done;
                for &arguments in systemctl_runs(action) {
                    ended = match Command::new("systemctl").args(arguments).output() {
                        Ok(output) => outcome(&output),
                        Err(error) => ServiceOutcome::Failed {
                            message: format!("can't run systemctl: {error}"),
                        },
                    };
                    if ended != ServiceOutcome::Done {
                        break;
                    }
                }
                done(ended);
            });
        if let Err(error) = started {
            eprintln!("prism: can't run systemctl: {error}");
        }
    }
}

/// The systemctl runs that do `action`, in order. polkit keeps the first run's authorization for
/// the second, so it asks once.
fn systemctl_runs(action: ServiceAction) -> &'static [&'static [&'static str]] {
    match action {
        ServiceAction::Enable => &[&["enable", "--now", "prismd.service"]],
        // Resetting first also clears systemd's start limit, which a plain restart can't get
        // past.
        ServiceAction::Restart => &[
            &["reset-failed", "prismd.service"],
            &["restart", "prismd.service"],
        ],
    }
}

/// prismd.service's state, as `systemctl show` gives it.
fn unit_state() -> UnitState {
    let shown = Command::new("systemctl")
        .args([
            "show",
            "prismd.service",
            "--property=LoadState,ActiveState,Result,UnitFileState",
        ])
        .output();
    match shown {
        Ok(output) if output.status.success() => {
            UnitState::from_systemctl_show(&String::from_utf8_lossy(&output.stdout))
        }
        Ok(output) => UnitState::Unknown {
            message: failure(&output),
        },
        Err(error) => UnitState::Unknown {
            message: format!("can't run systemctl: {error}"),
        },
    }
}

/// How a systemctl run under polkit ended. A canceled prompt leaves systemd saying "Access
/// denied", which isn't an error to show.
fn outcome(output: &Output) -> ServiceOutcome {
    if output.status.success() {
        return ServiceOutcome::Done;
    }
    let message = failure(output);
    if message.contains("Access denied") {
        ServiceOutcome::Canceled
    } else {
        ServiceOutcome::Failed { message }
    }
}

/// What a failed systemctl said, or its exit status when it said nothing.
fn failure(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    match stderr.trim() {
        "" => format!("systemctl {}", output.status),
        said => said.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::ExitStatusExt;
    use std::process::ExitStatus;

    use super::*;

    fn output(code: i32, stderr: &str) -> Output {
        Output {
            status: ExitStatus::from_raw(code << 8),
            stdout: Vec::new(),
            stderr: stderr.into(),
        }
    }

    #[test]
    fn enable_enables_and_starts_prismd_and_restart_resets_it_first() {
        assert_eq!(
            systemctl_runs(ServiceAction::Enable),
            [["enable", "--now", "prismd.service"].as_slice()]
        );
        assert_eq!(
            systemctl_runs(ServiceAction::Restart),
            [
                ["reset-failed", "prismd.service"].as_slice(),
                ["restart", "prismd.service"].as_slice()
            ]
        );
    }

    #[test]
    fn a_canceled_polkit_prompt_isnt_an_error() {
        assert_eq!(outcome(&output(0, "")), ServiceOutcome::Done);
        assert_eq!(
            outcome(&output(1, "Failed to enable unit: Access denied\n")),
            ServiceOutcome::Canceled
        );
        assert_eq!(
            outcome(&output(
                1,
                "Failed to enable unit: Interactive authentication required.\n"
            )),
            ServiceOutcome::Failed {
                message: "Failed to enable unit: Interactive authentication required.".into()
            }
        );
        assert_eq!(
            outcome(&output(4, "")),
            ServiceOutcome::Failed {
                message: "systemctl exit status: 4".into()
            }
        );
    }
}
