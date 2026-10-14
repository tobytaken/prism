use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

use nvml_wrapper::Nvml;
use serde::{Deserialize, Serialize};

use crate::DriverPlan;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NvidiaDriverStatus {
    pub detected: bool,
    pub driver_version: Option<String>,
    pub nvml_available: bool,
    pub nouveau_loaded: bool,
    pub runfile_install_detected: bool,
    pub rpm_fusion_detected: bool,
    pub gpus: Vec<NvidiaGpuInfo>,
    pub plans: Vec<DriverPlan>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NvidiaGpuInfo {
    pub id: String,
    pub name: String,
    pub architecture: Option<String>,
    pub bus_id: Option<String>,
}

pub fn collect_nvidia_status() -> NvidiaDriverStatus {
    let mut warnings = Vec::new();
    let mut gpus = collect_sysfs_gpus(Path::new("/sys/bus/pci/devices"));
    let driver_version = read_driver_version(Path::new("/proc/driver/nvidia/version"));
    let nvml_available = enrich_with_nvml(&mut gpus, &mut warnings);

    NvidiaDriverStatus {
        detected: !gpus.is_empty(),
        driver_version,
        nvml_available,
        nouveau_loaded: module_loaded("nouveau"),
        runfile_install_detected: Path::new("/usr/bin/nvidia-uninstall").exists()
            || Path::new("/usr/lib/nvidia/pre-install").exists(),
        rpm_fusion_detected: detect_rpm_fusion_packages(),
        gpus,
        plans: vec![],
        warnings,
    }
}

pub fn collect_sysfs_gpus(devices_path: &Path) -> Vec<NvidiaGpuInfo> {
    let entries = match fs::read_dir(devices_path) {
        Ok(entries) => entries,
        Err(_) => return vec![],
    };

    let mut gpus = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let vendor = read_trimmed(path.join("vendor"));
        if vendor.as_deref() != Some("0x10de") {
            continue;
        }

        let class = read_trimmed(path.join("class")).unwrap_or_default();
        if !is_display_or_3d_controller(&class) {
            continue;
        }

        let bus_id = entry.file_name().to_string_lossy().into_owned();
        gpus.push(NvidiaGpuInfo {
            id: bus_id.clone(),
            name: format!("NVIDIA GPU {bus_id}"),
            architecture: None,
            bus_id: Some(bus_id),
        });
    }

    gpus.sort_by(|left, right| left.id.cmp(&right.id));
    gpus
}

pub fn read_driver_version(path: &Path) -> Option<String> {
    let contents = fs::read_to_string(path).ok()?;
    for token in contents.split_whitespace() {
        if token
            .chars()
            .next()
            .is_some_and(|char| char.is_ascii_digit())
            && token.contains('.')
        {
            return Some(token.to_string());
        }
    }
    None
}

pub fn gpu_supports_open_modules(gpus: &[NvidiaGpuInfo]) -> bool {
    !gpus.is_empty()
        && gpus.iter().all(|gpu| {
            matches!(
                gpu.architecture.as_deref(),
                Some("Turing" | "Ampere" | "Ada" | "Hopper" | "Blackwell")
            )
        })
}

fn enrich_with_nvml(gpus: &mut Vec<NvidiaGpuInfo>, warnings: &mut Vec<String>) -> bool {
    let nvml = match Nvml::init() {
        Ok(nvml) => nvml,
        Err(error) => {
            warnings.push(format!("NVML is unavailable: {error}"));
            return false;
        }
    };

    let count = match nvml.device_count() {
        Ok(count) => count,
        Err(error) => {
            warnings.push(format!("Unable to query NVIDIA device count: {error}"));
            return false;
        }
    };

    let mut seen = BTreeSet::new();

    for index in 0..count {
        let Ok(device) = nvml.device_by_index(index) else {
            continue;
        };

        let uuid = device
            .uuid()
            .ok()
            .unwrap_or_else(|| format!("nvidia-{index}"));
        let name = device
            .name()
            .unwrap_or_else(|_| format!("NVIDIA GPU {index}"));
        let architecture = device
            .architecture()
            .ok()
            .map(|architecture| architecture.to_string());
        let bus_id = device.pci_info().ok().map(|info| info.bus_id);

        merge_nvml_gpu(
            gpus,
            &mut seen,
            NvidiaGpuInfo {
                id: uuid,
                name,
                architecture,
                bus_id,
            },
        );
    }

    true
}

/// Fills in the sysfs GPU at the same PCI address as one NVML reports, or adds it if there's none.
fn merge_nvml_gpu(gpus: &mut Vec<NvidiaGpuInfo>, seen: &mut BTreeSet<String>, gpu: NvidiaGpuInfo) {
    let address = gpu.bus_id.as_deref().and_then(PciAddress::parse);
    if let Some(address) = address
        && let Some(existing) = gpus.iter_mut().find(|existing| {
            existing.bus_id.as_deref().and_then(PciAddress::parse) == Some(address)
        })
    {
        existing.id = gpu.id;
        existing.name = gpu.name;
        existing.architecture = gpu.architecture;
        seen.insert(existing.id.clone());
        return;
    }

    if seen.insert(gpu.id.clone()) {
        gpus.push(gpu);
    }
}

/// A PCI device's domain, bus, device and function. Sysfs and NVML write the same address
/// differently: `0000:01:00.0` and `00000000:01:00.0`, with NVML's hex digits in uppercase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PciAddress {
    domain: u32,
    bus: u8,
    device: u8,
    function: u8,
}

impl PciAddress {
    fn parse(address: &str) -> Option<Self> {
        let (domain, rest) = address.trim().split_once(':')?;
        let (bus, rest) = rest.split_once(':')?;
        let (device, function) = rest.split_once('.')?;
        Some(Self {
            domain: u32::from_str_radix(domain, 16).ok()?,
            bus: u8::from_str_radix(bus, 16).ok()?,
            device: u8::from_str_radix(device, 16).ok()?,
            function: u8::from_str_radix(function, 16).ok()?,
        })
    }
}

fn is_display_or_3d_controller(class: &str) -> bool {
    class.starts_with("0x03")
}

fn module_loaded(module: &str) -> bool {
    fs::read_to_string("/proc/modules")
        .map(|contents| {
            contents
                .lines()
                .any(|line| line.split_whitespace().next() == Some(module))
        })
        .unwrap_or(false)
}

fn detect_rpm_fusion_packages() -> bool {
    Command::new("rpm")
        .arg("-qa")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|packages| {
            packages.lines().any(|package| {
                package.contains("akmod-nvidia")
                    || package.contains("xorg-x11-drv-nvidia")
                    || package.contains("kmod-nvidia")
            })
        })
        .unwrap_or(false)
}

fn read_trimmed(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::{
        NvidiaGpuInfo, PciAddress, collect_sysfs_gpus, gpu_supports_open_modules, merge_nvml_gpu,
        read_driver_version,
    };
    use std::collections::BTreeSet;
    use std::fs;

    fn sysfs_gpu(bus_id: &str) -> NvidiaGpuInfo {
        NvidiaGpuInfo {
            id: bus_id.into(),
            name: format!("NVIDIA GPU {bus_id}"),
            architecture: None,
            bus_id: Some(bus_id.into()),
        }
    }

    fn nvml_gpu(uuid: &str, bus_id: &str) -> NvidiaGpuInfo {
        NvidiaGpuInfo {
            id: uuid.into(),
            name: "NVIDIA GeForce RTX 4090".into(),
            architecture: Some("Ada".into()),
            bus_id: Some(bus_id.into()),
        }
    }

    #[test]
    fn nvml_fills_in_the_sysfs_gpu_at_the_same_address_however_its_written() {
        for (sysfs, nvml) in [
            ("0000:01:00.0", "00000000:01:00.0"),
            ("0000:0a:00.0", "00000000:0A:00.0"),
            ("0001:c1:00.0", "0001:C1:00.0"),
        ] {
            let mut gpus = vec![sysfs_gpu(sysfs)];
            merge_nvml_gpu(&mut gpus, &mut BTreeSet::new(), nvml_gpu("GPU-1", nvml));

            assert_eq!(gpus.len(), 1, "{sysfs} and {nvml}: {gpus:?}");
            assert_eq!(gpus[0].id, "GPU-1");
            assert_eq!(gpus[0].architecture.as_deref(), Some("Ada"));
            assert!(gpu_supports_open_modules(&gpus));
        }
    }

    #[test]
    fn nvml_adds_a_gpu_at_an_address_sysfs_didnt_list() {
        let mut gpus = vec![sysfs_gpu("0000:01:00.0")];
        merge_nvml_gpu(
            &mut gpus,
            &mut BTreeSet::new(),
            nvml_gpu("GPU-2", "00000000:02:00.0"),
        );

        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0].id, "0000:01:00.0");
        assert_eq!(gpus[1].id, "GPU-2");
    }

    #[test]
    fn pci_addresses_that_dont_parse_match_nothing() {
        assert_eq!(PciAddress::parse("01:00.0"), None);
        assert_eq!(PciAddress::parse("0000:01:00"), None);
        assert_eq!(PciAddress::parse("0000:01:zz.0"), None);
        assert_eq!(
            PciAddress::parse("00000000:0A:1F.3"),
            PciAddress::parse("0000:0a:1f.3")
        );
    }

    #[test]
    fn detects_nvidia_display_gpus_from_sysfs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let gpu_dir = dir.path().join("0000:01:00.0");
        fs::create_dir(&gpu_dir).expect("gpu dir");
        fs::write(gpu_dir.join("vendor"), "0x10de\n").expect("vendor");
        fs::write(gpu_dir.join("class"), "0x030000\n").expect("class");

        let gpus = collect_sysfs_gpus(dir.path());

        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].bus_id.as_deref(), Some("0000:01:00.0"));
    }

    #[test]
    fn ignores_non_nvidia_devices_from_sysfs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let gpu_dir = dir.path().join("0000:02:00.0");
        fs::create_dir(&gpu_dir).expect("gpu dir");
        fs::write(gpu_dir.join("vendor"), "0x1002\n").expect("vendor");
        fs::write(gpu_dir.join("class"), "0x030000\n").expect("class");

        assert!(collect_sysfs_gpus(dir.path()).is_empty());
    }

    #[test]
    fn parses_driver_version_from_proc_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("version");
        fs::write(
            &path,
            "NVRM version: NVIDIA UNIX x86_64 Kernel Module  595.58.03  Wed Jan 1\n",
        )
        .expect("write");

        assert_eq!(read_driver_version(&path).as_deref(), Some("595.58.03"));
    }

    #[test]
    fn open_modules_require_all_known_gpus_to_be_turing_or_newer() {
        assert!(gpu_supports_open_modules(&[NvidiaGpuInfo {
            id: "GPU-1".into(),
            name: "RTX".into(),
            architecture: Some("Blackwell".into()),
            bus_id: None,
        }]));
        assert!(!gpu_supports_open_modules(&[NvidiaGpuInfo {
            id: "GPU-2".into(),
            name: "GTX".into(),
            architecture: Some("Pascal".into()),
            bus_id: None,
        }]));
    }
}
