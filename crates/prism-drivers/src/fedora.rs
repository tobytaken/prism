use crate::host::{DriverHostInfo, SecureBootState};
use crate::nvidia::{KernelModule, NvidiaDriverStatus, NvidiaGpuInfo, kernel_module_for};
use crate::{DriverAction, DriverPlan};

pub fn fedora_plans(host: &DriverHostInfo, nvidia: &NvidiaDriverStatus) -> Vec<DriverPlan> {
    let base_blockers = fedora_blockers(host, nvidia);
    let mut warnings = Vec::new();
    if host.secure_boot == SecureBootState::Enabled {
        warnings.push(
            "Secure Boot is enabled. Fedora may require Machine Owner Key enrollment before the NVIDIA kernel module can load.".into(),
        );
    }
    if nvidia.nouveau_loaded {
        warnings.push("Nouveau is loaded. A reboot is required after installation.".into());
    }

    let mut install_blockers = base_blockers.clone();
    let mut install_warnings = Vec::new();
    match fedora_module_package(&nvidia.gpus) {
        Ok(package) => install_warnings.push(format!(
            "Fedora automation will install nvidia-driver with {package}."
        )),
        Err(reason) => install_blockers.push(reason),
    }
    install_warnings.extend(warnings.iter().cloned());

    vec![
        DriverPlan {
            id: "fedora-install-recommended".into(),
            action: DriverAction::InstallRecommended,
            label: "Install recommended NVIDIA driver".into(),
            recommended: nvidia.driver_version.is_none(),
            requires_reboot: true,
            manual_steps: fedora_manual_steps(host),
            blockers: install_blockers,
            warnings: install_warnings,
        },
        DriverPlan {
            id: "fedora-update-installed".into(),
            action: DriverAction::UpdateInstalled,
            label: "Update installed NVIDIA driver packages".into(),
            recommended: nvidia.driver_version.is_some(),
            requires_reboot: true,
            manual_steps: fedora_manual_steps(host),
            blockers: base_blockers,
            warnings,
        },
    ]
}

/// The package that builds the kernel module every one of these GPUs runs on, or why Prism can't
/// choose one.
pub(crate) fn fedora_module_package(gpus: &[NvidiaGpuInfo]) -> Result<&'static str, String> {
    kernel_module_for(gpus).map(|module| match module {
        KernelModule::Open => "kmod-nvidia-open-dkms",
        KernelModule::Proprietary => "kmod-nvidia-latest-dkms",
    })
}

fn fedora_blockers(host: &DriverHostInfo, nvidia: &NvidiaDriverStatus) -> Vec<String> {
    let mut blockers = Vec::new();

    if host.unsupported_reason.is_some() {
        blockers.push(host.unsupported_reason.clone().unwrap());
    }
    if host.distro_version != "43" {
        blockers.push(format!(
            "Fedora {} is not in the initial tested automation scope; Fedora 43 is supported.",
            host.distro_version
        ));
    }
    if std::env::consts::ARCH != "x86_64" {
        blockers
            .push("Only x86_64 Fedora NVIDIA repository automation is currently supported.".into());
    }
    if nvidia.gpus.is_empty() {
        blockers.push("No NVIDIA GPU was detected.".into());
    }
    if nvidia.runfile_install_detected {
        blockers.push(
            "An NVIDIA runfile installation is present. Remove it before using distro packages."
                .into(),
        );
    }
    if nvidia.rpm_fusion_detected {
        blockers.push(
            "RPM Fusion NVIDIA packages are installed. Prism will not mix them with NVIDIA's official repository.".into(),
        );
    }

    blockers
}

fn fedora_manual_steps(host: &DriverHostInfo) -> Vec<String> {
    let mut steps =
        vec!["Reboot after installation or update before expecting the driver to load.".into()];
    if host.secure_boot == SecureBootState::Enabled {
        steps.push(
            "Complete Machine Owner Key enrollment from firmware if Fedora asks for it.".into(),
        );
    }
    steps
}

#[cfg(test)]
mod tests {
    use super::fedora_plans;
    use crate::host::{DriverHostInfo, PackageManager, SecureBootState};
    use crate::nvidia::{NvidiaDriverStatus, NvidiaGpuInfo};

    fn fresh_install(pci_device_ids: &[u16]) -> NvidiaDriverStatus {
        let mut status = nvidia();
        status.gpus = pci_device_ids
            .iter()
            .enumerate()
            .map(|(index, &pci_device_id)| NvidiaGpuInfo {
                id: format!("0000:0{}:00.0", index + 1),
                name: format!("NVIDIA GPU 0000:0{}:00.0", index + 1),
                architecture: None,
                bus_id: Some(format!("0000:0{}:00.0", index + 1)),
                pci_device_id: Some(pci_device_id),
            })
            .collect();
        status
    }

    #[test]
    fn audit_an_unidentified_gpu_must_not_blindly_offer_the_proprietary_module() {
        let mut status = nvidia();
        // Before the driver is installed, sysfs discovery gives every GPU this state,
        // including Blackwell GPUs that require the open module.
        status.gpus[0].architecture = None;
        let plans = fedora_plans(&host(), &status);
        let install = &plans[0];
        assert!(
            !install.blockers.is_empty()
                || !install
                    .warnings
                    .iter()
                    .any(|warning| warning.contains("kmod-nvidia-latest-dkms")),
            "unknown hardware gets an unblocked proprietary-module install: {install:?}"
        );
    }

    #[test]
    fn a_fresh_install_on_a_gpu_of_unknown_generation_is_blocked() {
        let mut status = nvidia();
        status.gpus[0].architecture = None;

        let install = &fedora_plans(&host(), &status)[0];

        assert!(
            install
                .blockers
                .iter()
                .any(|blocker| blocker.contains("couldn't identify the generation")),
            "{install:?}"
        );
        assert!(
            !install
                .warnings
                .iter()
                .any(|warning| warning.contains("kmod-nvidia")),
            "{install:?}"
        );
    }

    #[test]
    fn a_fresh_install_chooses_the_module_from_the_device_id() {
        for (pci_device_id, package) in [
            (0x2B85, "kmod-nvidia-open-dkms"),
            (0x2684, "kmod-nvidia-open-dkms"),
            (0x1B80, "kmod-nvidia-latest-dkms"),
        ] {
            let install = &fedora_plans(&host(), &fresh_install(&[pci_device_id]))[0];

            assert!(install.blockers.is_empty(), "{install:?}");
            assert_eq!(
                install
                    .warnings
                    .iter()
                    .filter(|warning| warning.contains("kmod-nvidia"))
                    .collect::<Vec<_>>(),
                [&format!(
                    "Fedora automation will install nvidia-driver with {package}."
                )],
                "{pci_device_id:#06x}"
            );
        }
    }

    #[test]
    fn a_fresh_install_on_a_pre_turing_and_a_newer_gpu_is_blocked() {
        let install = &fedora_plans(&host(), &fresh_install(&[0x1B80, 0x2684]))[0];

        assert!(
            install
                .blockers
                .iter()
                .any(|blocker| blocker.contains("No single NVIDIA kernel module")),
            "{install:?}"
        );
    }

    #[test]
    fn fedora_plan_blocks_rpm_fusion_mixing() {
        let mut status = nvidia();
        status.rpm_fusion_detected = true;

        let plans = fedora_plans(&host(), &status);

        assert!(
            plans[0]
                .blockers
                .iter()
                .any(|blocker| blocker.contains("RPM Fusion"))
        );
    }

    fn host() -> DriverHostInfo {
        DriverHostInfo {
            distro_id: "fedora".into(),
            distro_version: "43".into(),
            kernel_release: "6.17.1".into(),
            package_manager: PackageManager::Dnf,
            secure_boot: SecureBootState::Disabled,
            is_flatpak: false,
            supported: false,
            unsupported_reason: None,
        }
    }

    fn nvidia() -> NvidiaDriverStatus {
        NvidiaDriverStatus {
            detected: true,
            driver_version: None,
            nvml_available: false,
            nouveau_loaded: false,
            runfile_install_detected: false,
            rpm_fusion_detected: false,
            gpus: vec![NvidiaGpuInfo {
                id: "GPU-1".into(),
                name: "NVIDIA GPU".into(),
                architecture: Some("Blackwell".into()),
                bus_id: None,
                pci_device_id: None,
            }],
            plans: vec![],
            warnings: vec![],
        }
    }
}
