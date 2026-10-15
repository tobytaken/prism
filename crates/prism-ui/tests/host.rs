//! Reading what the host says apart from prismd's socket: `systemctl show prismd.service`,
//! os-release and the NVIDIA driver's version file.

use prism_ui::{Distro, UnitState, nvidia_driver_version};

fn unit(show: &str) -> UnitState {
    UnitState::from_systemctl_show(show)
}

#[test]
fn systemctl_show_tells_the_unit_states_apart() {
    assert_eq!(
        unit("LoadState=not-found\nActiveState=inactive\nResult=success\n"),
        UnitState::NotFound
    );
    assert_eq!(
        unit("LoadState=loaded\nActiveState=inactive\nResult=success\nUnitFileState=disabled\n"),
        UnitState::Inactive { enabled: false }
    );
    assert_eq!(
        unit("LoadState=loaded\nActiveState=inactive\nResult=success\nUnitFileState=enabled\n"),
        UnitState::Inactive { enabled: true }
    );
    assert_eq!(
        unit("LoadState=loaded\nActiveState=active\nResult=success\n"),
        UnitState::Active
    );
    assert_eq!(
        unit("LoadState=loaded\nActiveState=activating\nResult=success\n"),
        UnitState::Active
    );
    assert_eq!(
        unit("LoadState=loaded\nActiveState=failed\nResult=watchdog\n"),
        UnitState::Failed {
            result: "watchdog".into()
        }
    );
    // A masked unit is installed; enabling it says why it can't start.
    assert_eq!(
        unit("LoadState=masked\nActiveState=inactive\nResult=success\n"),
        UnitState::Inactive { enabled: false }
    );
}

#[test]
fn output_systemctl_doesnt_normally_give_is_unknown() {
    assert_eq!(
        unit(""),
        UnitState::Unknown {
            message: "systemctl didn't say how prismd.service is".into()
        }
    );
    assert_eq!(
        unit("LoadState=loaded\nActiveState=exploding\n"),
        UnitState::Unknown {
            message: "systemctl says prismd.service is exploding".into()
        }
    );
}

#[test]
fn os_release_gives_the_distribution() {
    let ubuntu = "PRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\nNAME=\"Ubuntu\"\nVERSION_ID=\"24.04\"\n\
                  ID=ubuntu\nID_LIKE=debian\n";
    assert_eq!(
        Distro::from_os_release(ubuntu),
        Distro {
            id: "ubuntu".into(),
            id_like: vec!["debian".into()],
            name: "Ubuntu 24.04.1 LTS".into(),
        }
    );

    let alma = "# A comment\nNAME='AlmaLinux'\nID=\"almalinux\"\nID_LIKE=\"rhel centos fedora\"\n";
    assert_eq!(
        Distro::from_os_release(alma),
        Distro {
            id: "almalinux".into(),
            id_like: vec!["rhel".into(), "centos".into(), "fedora".into()],
            name: "AlmaLinux".into(),
        }
    );

    assert_eq!(
        Distro::from_os_release(""),
        Distro {
            id: "linux".into(),
            id_like: Vec::new(),
            name: "Linux".into(),
        }
    );
}

#[test]
fn the_nvidia_driver_version_comes_from_sysfs_or_procfs() {
    assert_eq!(
        nvidia_driver_version("615.78.08\n"),
        Some("615.78.08".into())
    );
    // Flatpak leaves /sys/module out of the sandbox, but not /proc/driver/nvidia.
    assert_eq!(
        nvidia_driver_version(
            "NVRM version: NVIDIA UNIX Open Kernel Module for x86_64  580.82.07  Release Build  \
             (dvs-builder@U16-I3-B03-4-3)  Wed Aug 27 21:03:12 UTC 2025\n\
             GCC version:  gcc version 13.3.0 (Ubuntu 13.3.0-6ubuntu2~24.04)\n"
        ),
        Some("580.82.07".into())
    );
    assert_eq!(
        nvidia_driver_version(
            "NVRM version: NVIDIA UNIX x86_64 Kernel Module  550.54.14  Thu Feb 22 01:44:30 UTC \
             2024\n"
        ),
        Some("550.54.14".into())
    );
    assert_eq!(nvidia_driver_version(""), None);
    assert_eq!(nvidia_driver_version("not a version\n"), None);
}
