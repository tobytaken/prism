use crate::drivers::host::{DriverHostInfo, SecureBootState};
use crate::drivers::nvidia::NvidiaDriverStatus;
use crate::drivers::{DriverAction, DriverPlan};

pub fn ubuntu_plans(host: &DriverHostInfo, nvidia: &NvidiaDriverStatus) -> Vec<DriverPlan> {
    let base_blockers = ubuntu_blockers(host, nvidia);
    let mut warnings = Vec::new();

    if nvidia.nouveau_loaded {
        warnings.push(
            "Nouveau is loaded. A reboot is required after installing the proprietary driver."
                .into(),
        );
    }
    if host.secure_boot == SecureBootState::Enabled {
        warnings.push(
            "Secure Boot is enabled. Ubuntu's recommended driver path normally uses signed modules, but firmware prompts may still appear on some systems.".into(),
        );
    }

    vec![
        DriverPlan {
            id: "ubuntu-install-recommended".into(),
            action: DriverAction::InstallRecommended,
            label: "Install recommended NVIDIA driver".into(),
            recommended: nvidia.driver_version.is_none(),
            requires_reboot: true,
            manual_steps: vec![
                "Reboot after installation before expecting the new driver to load.".into(),
            ],
            blockers: base_blockers.clone(),
            warnings: warnings.clone(),
        },
        DriverPlan {
            id: "ubuntu-update-installed".into(),
            action: DriverAction::UpdateInstalled,
            label: "Update installed NVIDIA driver packages".into(),
            recommended: nvidia.driver_version.is_some(),
            requires_reboot: true,
            manual_steps: vec![
                "Reboot after updating if the kernel module or driver package changed.".into(),
            ],
            blockers: base_blockers,
            warnings,
        },
    ]
}

fn ubuntu_blockers(host: &DriverHostInfo, nvidia: &NvidiaDriverStatus) -> Vec<String> {
    let mut blockers = Vec::new();

    if host.unsupported_reason.is_some() {
        blockers.push(host.unsupported_reason.clone().unwrap());
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

    blockers
}

#[cfg(test)]
mod tests {
    use super::ubuntu_plans;
    use crate::drivers::host::{DriverHostInfo, PackageManager, SecureBootState};
    use crate::drivers::nvidia::{NvidiaDriverStatus, NvidiaGpuInfo};

    #[test]
    fn ubuntu_plan_recommends_install_when_driver_is_missing() {
        let plans = ubuntu_plans(&host(), &nvidia(None));

        assert!(plans
            .iter()
            .any(|plan| plan.id == "ubuntu-install-recommended" && plan.recommended));
    }

    fn host() -> DriverHostInfo {
        DriverHostInfo {
            distro_id: "ubuntu".into(),
            distro_version: "24.04".into(),
            kernel_release: "6.8.0".into(),
            package_manager: PackageManager::Apt,
            secure_boot: SecureBootState::Disabled,
            is_flatpak: false,
            supported: false,
            unsupported_reason: None,
        }
    }

    fn nvidia(driver_version: Option<&str>) -> NvidiaDriverStatus {
        NvidiaDriverStatus {
            detected: true,
            driver_version: driver_version.map(str::to_string),
            nvml_available: driver_version.is_some(),
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
