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
    /// The PCI device ID, such as 0x2B85, which sysfs reports whether or not a driver is loaded.
    #[serde(default)]
    pub pci_device_id: Option<u16>,
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
            pci_device_id: read_trimmed(path.join("device"))
                .and_then(|device| u16::from_str_radix(device.strip_prefix("0x")?, 16).ok()),
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

/// The NVIDIA kernel module a driver install builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelModule {
    Open,
    Proprietary,
}

/// The first PCI device ID of a Turing GPU. The open kernel module can't run GPUs before Turing,
/// and every GPU from this ID on supports it, as NVIDIA's list of supported GPUs for driver
/// 615.78.08 shows.
const FIRST_TURING_DEVICE_ID: u16 = 0x1E00;

/// A GPU's generation as far as it decides the kernel modules it runs on, from what NVML or its
/// device ID tells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModuleGeneration {
    /// Turing to Hopper, as NVML reports, which run on either module.
    TuringToHopper,
    /// Turing or newer by its device ID, which runs on the open module and may be a Blackwell
    /// that needs it.
    TuringOrNewer,
    /// Blackwell, as NVML reports, which needs the open module.
    Blackwell,
    /// Older than Turing, which needs the proprietary module.
    OlderThanTuring,
    /// Neither NVML nor the device ID tells its generation.
    Unknown,
}

impl ModuleGeneration {
    /// What NVML's architecture says, or failing that the device ID.
    fn of(gpu: &NvidiaGpuInfo) -> Self {
        match gpu.architecture.as_deref() {
            Some("Turing" | "Ampere" | "Ada" | "Hopper") => Self::TuringToHopper,
            Some("Blackwell") => Self::Blackwell,
            Some("Kepler" | "Maxwell" | "Pascal" | "Volta") => Self::OlderThanTuring,
            _ => match gpu.pci_device_id {
                Some(id) if id >= FIRST_TURING_DEVICE_ID => Self::TuringOrNewer,
                Some(_) => Self::OlderThanTuring,
                None => Self::Unknown,
            },
        }
    }
}

/// The kernel module every one of these GPUs runs on, or why Prism can't choose one: no GPU, a
/// GPU whose generation it couldn't identify, or GPUs no single module supports.
pub fn kernel_module_for(gpus: &[NvidiaGpuInfo]) -> Result<KernelModule, String> {
    if gpus.is_empty() {
        return Err("No NVIDIA GPU was detected.".into());
    }
    let generations: Vec<_> = gpus.iter().map(ModuleGeneration::of).collect();

    if let Some(gpu) = gpus
        .iter()
        .zip(&generations)
        .find_map(|(gpu, generation)| (*generation == ModuleGeneration::Unknown).then_some(gpu))
    {
        return Err(format!(
            "Prism couldn't identify the generation of {}, so it can't choose an NVIDIA kernel module.",
            gpu.name
        ));
    }
    if !generations.contains(&ModuleGeneration::OlderThanTuring) {
        return Ok(KernelModule::Open);
    }
    if generations.iter().any(|generation| {
        matches!(
            generation,
            ModuleGeneration::Blackwell | ModuleGeneration::TuringOrNewer
        )
    }) {
        return Err(
            "No single NVIDIA kernel module supports this combination of GPUs: one needs the proprietary module, and another needs or may need the open module.".into(),
        );
    }
    Ok(KernelModule::Proprietary)
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
        let pci_info = device.pci_info().ok();
        // NVML combines the device ID with the vendor ID, which is in the low 16 bits.
        let pci_device_id = pci_info
            .as_ref()
            .map(|info| (info.pci_device_id >> 16) as u16);
        let bus_id = pci_info.map(|info| info.bus_id);

        merge_nvml_gpu(
            gpus,
            &mut seen,
            NvidiaGpuInfo {
                id: uuid,
                name,
                architecture,
                bus_id,
                pci_device_id,
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
        existing.pci_device_id = existing.pci_device_id.or(gpu.pci_device_id);
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
        KernelModule, NvidiaGpuInfo, PciAddress, collect_sysfs_gpus, kernel_module_for,
        merge_nvml_gpu, read_driver_version,
    };
    use std::collections::BTreeSet;
    use std::fs;

    fn sysfs_gpu(bus_id: &str) -> NvidiaGpuInfo {
        NvidiaGpuInfo {
            id: bus_id.into(),
            name: format!("NVIDIA GPU {bus_id}"),
            architecture: None,
            bus_id: Some(bus_id.into()),
            pci_device_id: Some(0x2684),
        }
    }

    fn nvml_gpu(uuid: &str, bus_id: &str) -> NvidiaGpuInfo {
        NvidiaGpuInfo {
            id: uuid.into(),
            name: "NVIDIA GeForce RTX 4090".into(),
            architecture: Some("Ada".into()),
            bus_id: Some(bus_id.into()),
            pci_device_id: None,
        }
    }

    /// A GPU NVML reports with `architecture`, or that sysfs found with `pci_device_id`.
    fn gpu(architecture: Option<&str>, pci_device_id: Option<u16>) -> NvidiaGpuInfo {
        NvidiaGpuInfo {
            id: "0000:01:00.0".into(),
            name: "NVIDIA GPU 0000:01:00.0".into(),
            architecture: architecture.map(str::to_string),
            bus_id: Some("0000:01:00.0".into()),
            pci_device_id,
        }
    }

    /// A sysfs PCI device directory for an NVIDIA display controller, with this `device` file.
    fn sysfs_device(devices: &std::path::Path, bus_id: &str, device: Option<&str>) {
        let gpu_dir = devices.join(bus_id);
        fs::create_dir(&gpu_dir).expect("gpu dir");
        fs::write(gpu_dir.join("vendor"), "0x10de\n").expect("vendor");
        fs::write(gpu_dir.join("class"), "0x030000\n").expect("class");
        if let Some(device) = device {
            fs::write(gpu_dir.join("device"), device).expect("device");
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
            assert_eq!(gpus[0].pci_device_id, Some(0x2684));
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
    fn sysfs_discovery_reads_each_gpus_pci_device_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        sysfs_device(dir.path(), "0000:01:00.0", Some("0x2b85\n"));
        sysfs_device(dir.path(), "0000:02:00.0", None);
        sysfs_device(dir.path(), "0000:03:00.0", Some("graphics\n"));

        let gpus = collect_sysfs_gpus(dir.path());

        let ids: Vec<_> = gpus.iter().map(|gpu| gpu.pci_device_id).collect();
        assert_eq!(ids, [Some(0x2B85), None, None]);
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
    fn a_fresh_install_chooses_the_module_from_the_gpus_device_id() {
        for (pci_device_id, module) in [
            // Blackwell, which needs the open module.
            (0x2B85, KernelModule::Open),
            // Ada, which runs on either.
            (0x2684, KernelModule::Open),
            // The first Turing ID.
            (0x1E02, KernelModule::Open),
            // Pascal, which needs the proprietary module.
            (0x1B80, KernelModule::Proprietary),
            // The last Volta ID.
            (0x1DF6, KernelModule::Proprietary),
        ] {
            assert_eq!(
                kernel_module_for(&[gpu(None, Some(pci_device_id))]),
                Ok(module),
                "{pci_device_id:#06x}"
            );
        }
    }

    #[test]
    fn the_architecture_nvml_reports_decides_over_the_device_id() {
        for (architecture, pci_device_id, module) in [
            ("Blackwell", Some(0x1B80), KernelModule::Open),
            ("Hopper", None, KernelModule::Open),
            ("Turing", None, KernelModule::Open),
            ("Pascal", Some(0x2B85), KernelModule::Proprietary),
            ("Maxwell", None, KernelModule::Proprietary),
            ("Kepler", None, KernelModule::Proprietary),
            ("Unknown", Some(0x2B85), KernelModule::Open),
            ("Unknown", Some(0x1B80), KernelModule::Proprietary),
        ] {
            assert_eq!(
                kernel_module_for(&[gpu(Some(architecture), pci_device_id)]),
                Ok(module),
                "{architecture} {pci_device_id:?}"
            );
        }
    }

    #[test]
    fn no_gpu_leaves_no_module_to_choose() {
        assert_eq!(
            kernel_module_for(&[]),
            Err("No NVIDIA GPU was detected.".into())
        );
    }

    #[test]
    fn a_gpu_of_unknown_generation_leaves_no_module_to_choose() {
        for unidentified in [gpu(None, None), gpu(Some("Unknown"), None)] {
            let reason = kernel_module_for(&[gpu(Some("Ada"), None), unidentified])
                .expect_err("no module for an unidentified GPU");

            assert!(reason.contains("couldn't identify"), "{reason}");
            assert!(reason.contains("0000:01:00.0"), "{reason}");
        }
    }

    #[test]
    fn gpus_known_not_to_need_the_open_module_share_the_proprietary_one() {
        assert_eq!(
            kernel_module_for(&[gpu(Some("Ada"), None), gpu(None, Some(0x1B80))]),
            Ok(KernelModule::Proprietary)
        );
        assert_eq!(
            kernel_module_for(&[gpu(Some("Turing"), None), gpu(Some("Pascal"), None)]),
            Ok(KernelModule::Proprietary)
        );
    }

    #[test]
    fn a_gpu_that_needs_the_proprietary_module_cant_share_with_one_that_may_need_the_open_one() {
        for newer in [
            gpu(None, Some(0x2684)),
            gpu(None, Some(0x2B85)),
            gpu(Some("Blackwell"), None),
        ] {
            let reason = kernel_module_for(&[gpu(None, Some(0x1B80)), newer.clone()])
                .expect_err("no module for both GPUs");

            assert!(
                reason.contains("No single NVIDIA kernel module"),
                "{newer:?}: {reason}"
            );
        }
    }
}
