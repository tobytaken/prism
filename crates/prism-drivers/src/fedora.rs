use crate::host::{DriverHostInfo, SecureBootState};
use crate::nvidia::{NvidiaDriverStatus, gpu_supports_open_modules};
use crate::{DriverAction, DriverPlan};

pub fn fedora_plans(host: &DriverHostInfo, nvidia: &NvidiaDriverStatus) -> Vec<DriverPlan> {
    let base_blockers = fedora_blockers(host, nvidia);
    let module_package = if gpu_supports_open_modules(&nvidia.gpus) {
        "kmod-nvidia-open-dkms"
    } else {
        "kmod-nvidia-latest-dkms"
    };
    let mut warnings = vec![format!(
        "Fedora automation will install nvidia-driver with {module_package}."
    )];

    if nvidia.gpus.iter().any(|gpu| gpu.architecture.is_none()) {
        warnings.push(
            "GPU generation is unknown before the NVIDIA driver is loaded; Prism will use the proprietary DKMS module for compatibility.".into(),
        );
    }
    if host.secure_boot == SecureBootState::Enabled {
        warnings.push(
            "Secure Boot is enabled. Fedora may require Machine Owner Key enrollment before the NVIDIA kernel module can load.".into(),
        );
    }
    if nvidia.nouveau_loaded {
        warnings.push("Nouveau is loaded. A reboot is required after installation.".into());
    }

    vec![
        DriverPlan {
            id: "fedora-install-recommended".into(),
            action: DriverAction::InstallRecommended,
            label: "Install recommended NVIDIA driver".into(),
            recommended: nvidia.driver_version.is_none(),
            requires_reboot: true,
            manual_steps: fedora_manual_steps(host),
            blockers: base_blockers.clone(),
            warnings: warnings.clone(),
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
            }],
            plans: vec![],
            warnings: vec![],
        }
    }
}
