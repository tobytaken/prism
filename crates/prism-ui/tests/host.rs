//! Reading what the host says about prismd: `systemctl show prismd.service` and os-release.

use prism_ui::{Distro, UnitState};

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
        unit("LoadState=loaded\nActiveState=inactive\nResult=success\n"),
        UnitState::Inactive
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
        UnitState::Inactive
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
