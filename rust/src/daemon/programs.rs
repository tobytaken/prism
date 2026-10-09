//! File identity, shared by manual program recognition and the running-program picker.
use std::ffi::CString;
use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
use std::{fs, io, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FileIdentity {
    device: u64,
    inode: u64,
}

impl FileIdentity {
    pub fn at(path: &Path) -> io::Result<Self> {
        let metadata = fs::metadata(path)?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
}

/// Device-only metadata can rule out an unrelated executable even when its filesystem
/// denies inode metadata (for example another app on a user-owned FUSE mount).
/// Never turn an unreadable possible match into a negative observation.
pub(super) fn candidate_executable(
    path: &Path,
    candidates: impl Iterator<Item = FileIdentity>,
) -> io::Result<Option<FileIdentity>> {
    let name = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid executable path"))?;
    let mut stat = std::mem::MaybeUninit::<libc::statx>::zeroed();
    // SAFETY: name is NUL-terminated and stat points to an entire writable statx struct.
    // A zero mask still returns the containing device. FUSE explicitly permits that
    // query without granting access to its owner-controlled inode attributes.
    let result = unsafe { libc::statx(libc::AT_FDCWD, name.as_ptr(), 0, 0, stat.as_mut_ptr()) };
    if result == 0 {
        // SAFETY: a successful statx initialized the output buffer.
        let stat = unsafe { stat.assume_init() };
        let device = libc::makedev(stat.stx_dev_major, stat.stx_dev_minor);
        if !candidates
            .into_iter()
            .any(|identity| identity.device == device)
        {
            return Ok(None);
        }
    }
    // Unsupported device-only queries use the full observation; errors remain errors.
    FileIdentity::at(path).map(Some)
}

pub(super) struct MappedImage {
    pub identity: FileIdentity,
    pub path: String,
}

/// Resolve each file-backed mapping through its mapping-specific proc link. The
/// textual maps device can differ from stat on btrfs; the displayed pathname is
/// namespace-relative and may now refer to an entirely different file.
pub(super) fn mapped_images(process: &Path) -> io::Result<Vec<MappedImage>> {
    let mut images = Vec::new();
    for line in fs::read_to_string(process.join("maps"))?.lines() {
        let invalid = || {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Incomplete mapped image details",
            )
        };
        let mut remainder = line;
        let mut fields = Vec::new();
        for _ in 0..5 {
            remainder = remainder.trim_start();
            let end = remainder
                .find(char::is_whitespace)
                .unwrap_or(remainder.len());
            fields.push(&remainder[..end]);
            remainder = &remainder[end..];
        }
        let (start, end) = fields[0].split_once('-').ok_or_else(invalid)?;
        let start = u64::from_str_radix(start, 16).map_err(|_| invalid())?;
        let end = u64::from_str_radix(end, 16).map_err(|_| invalid())?;
        if start >= end {
            return Err(invalid());
        }
        let inode = fields[4].parse::<u64>().map_err(|_| invalid())?;
        // Anonymous mappings have no file identity and no map_files entry.
        if inode == 0 {
            continue;
        }
        images.push(MappedImage {
            // maps pads low addresses with zeroes; map_files names do not.
            identity: FileIdentity::at(
                &process.join("map_files").join(format!("{start:x}-{end:x}")),
            )?,
            path: unescape_proc_path(remainder.trim_start()),
        });
    }
    Ok(images)
}

use super::session::SessionPeer;
use crate::protocol::RunningProgram;
use std::collections::{BTreeMap, HashSet};

struct Process {
    parent: u32,
    launcher: Option<String>,
    name: String,
    path: Option<String>,
    environment: Vec<String>,
    helper: bool,
    warning: Option<String>,
}

/// A best-effort picker snapshot, scoped before reading sensitive process details. A process
/// that exits while the picker opens is omitted; recognition itself uses complete scans.
pub(super) fn list(root: &Path, peer: &SessionPeer) -> io::Result<Vec<RunningProgram>> {
    let mut processes = BTreeMap::new();
    for entry in fs::read_dir(root.join("proc"))? {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        if let Ok(Some(process)) = read_process(root, &entry.path(), peer.uid, pid) {
            processes.insert(pid, process);
        }
    }
    let mut result: Vec<_> = processes
        .iter()
        .map(|(pid, process)| {
            let mut seen = HashSet::from([*pid]);
            let mut parent = process.parent;
            let mut launcher = None;
            while seen.insert(parent) {
                let Some(ancestor) = processes.get(&parent) else {
                    break;
                };
                launcher = ancestor.launcher.clone();
                if launcher.is_some() {
                    break;
                }
                parent = ancestor.parent;
            }
            RunningProgram {
                pid: *pid,
                name: process.name.clone(),
                path: process.path.clone(),
                launcher,
                environment: process.environment.clone(),
                helper: process.helper,
                recognition_warning: process.warning.clone(),
            }
        })
        .collect();
    result.sort_by(|a, b| {
        (a.helper, a.name.to_lowercase(), a.pid).cmp(&(b.helper, b.name.to_lowercase(), b.pid))
    });
    Ok(result)
}

fn read_process(root: &Path, process: &Path, uid: u32, pid: u32) -> io::Result<Option<Process>> {
    let status = fs::read_to_string(process.join("status"))?;
    let owner = real_uid(&status);
    if owner != Some(uid) {
        return Ok(None);
    }
    let stat = fs::read_to_string(process.join("stat"))?;
    let started = super::recognition::start_time(&stat, pid)?;
    let parent = stat
        .rsplit_once(')')
        .and_then(|(_, fields)| fields.split_whitespace().nth(1))
        .and_then(|pid| pid.parse().ok())
        .unwrap_or(0);
    let env = fs::read(process.join("environ"))?;
    let environment: Vec<_> = env
        .split(|byte| *byte == 0)
        .filter_map(|entry| {
            let entry = std::str::from_utf8(entry).ok()?;
            let (key, _) = entry.split_once('=')?;
            [
                "SteamAppId",
                "STEAM_COMPAT_APP_ID",
                "WINEPREFIX",
                "STEAM_COMPAT_DATA_PATH",
                "FLATPAK_ID",
                "APPIMAGE",
                "LUTRIS_GAME_UUID",
                "HEROIC_APP_NAME",
            ]
            .contains(&key)
            .then(|| entry.to_string())
        })
        .collect();
    let executable = fs::read_link(process.join("exe"))?;
    let exe_name = basename(&executable.to_string_lossy()).to_string();
    let cmdline = fs::read(process.join("cmdline"))?;
    let args: Vec<_> = cmdline
        .split(|byte| *byte == 0)
        .filter_map(|arg| std::str::from_utf8(arg).ok())
        .collect();
    let launcher = launcher_name(&exe_name)
        .or_else(|| {
            if exe_name.starts_with("python") {
                args.iter()
                    .take(2)
                    .find_map(|arg| launcher_name(basename(arg)))
            } else {
                None
            }
        })
        .map(str::to_string);
    let mut name = exe_name.clone();
    let mut helper = is_helper(&executable.to_string_lossy());
    let mut location = host_location(
        root,
        process,
        &executable.to_string_lossy(),
        FileIdentity::at(&process.join("exe"))?,
    );
    if is_wine_loader(&executable) {
        let mut images: Vec<_> = mapped_images(process)?
            .into_iter()
            .filter(|image| image.path.to_lowercase().ends_with(".exe"))
            .collect();
        images.sort_by_key(|image| is_helper(&image.path));
        if let Some(image) = images.first() {
            name = basename(&image.path).to_string();
            helper = is_helper(&image.path);
            location = host_location(root, process, &image.path, image.identity);
        }
    }
    let lower = exe_name.to_lowercase();
    let warning = if lower.ends_with(".appimage") || environment.iter().any(|tag| tag.starts_with("APPIMAGE=")) || executable.to_string_lossy().contains("/.mount_") {
        Some("AppImages may run from a temporary mount; recognizing this location is not reliable.")
    } else if environment.iter().any(|tag| tag.starts_with("FLATPAK_ID=")) || executable.starts_with("/app/") {
        Some("Flatpak-packaged programs may not be recognizable by their host location.")
    } else if ["python", "bash", "sh", "dash", "node", "java", "perl", "ruby", "qemu", "virtualbox", "vmware", "retroarch", "dolphin-emu", "pcsx2", "rpcs3", "yuzu", "ryujinx", "dosbox", "snes9x"].iter().any(|prefix| lower.starts_with(prefix)) {
        Some("Scripts, virtual machines and emulators identify the runtime, not the individual game; recognition may be unreliable.")
    } else if location.is_none() {
        Some("The host program location could not be recovered; browse for the program file instead.")
    } else { None }.map(str::to_string);
    if super::recognition::start_time(&fs::read_to_string(process.join("stat"))?, pid)? != started
        || real_uid(&fs::read_to_string(process.join("status"))?) != owner
    {
        return Ok(None);
    }
    Ok(Some(Process {
        parent,
        launcher,
        name,
        path: location,
        environment,
        helper,
        warning,
    }))
}

fn real_uid(status: &str) -> Option<u32> {
    status.lines().find_map(|line| {
        line.strip_prefix("Uid:")?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    })
}

pub(super) fn is_wine_loader(executable: &Path) -> bool {
    matches!(
        executable.file_name().and_then(|name| name.to_str()),
        Some("wine" | "wine64" | "wine-preloader" | "wine64-preloader")
    )
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn launcher_name(name: &str) -> Option<&'static str> {
    match name.to_lowercase().as_str() {
        "steam" | "steamwebhelper" => Some("Steam"),
        "lutris" => Some("Lutris"),
        "heroic" | "heroicgameslauncher" => Some("Heroic"),
        "bottles" => Some("Bottles"),
        _ => None,
    }
}

fn is_helper(path: &str) -> bool {
    let path = path.to_lowercase();
    let name = basename(&path);
    name.starts_with("python")
        || name.starts_with("pressure-vessel")
        || [
            "wineserver",
            "wine",
            "wine64",
            "wine-preloader",
            "wine64-preloader",
            "bwrap",
            "srt-bwrap",
            "pv-adverb",
            "reaper",
            "steam.exe",
        ]
        .contains(&name)
        || path.contains("/windows/system32/")
        || path.contains("/windows/syswow64/")
        // Wine may map the runtime file behind a prefix's system-directory symlink.
        || path.contains("/wine/x86_64-windows/")
        || path.contains("/wine/i386-windows/")
}

/// A displayed path is usable only if it resolves on the host to the observed identity.
fn host_location(
    root: &Path,
    process: &Path,
    displayed: &str,
    identity: FileIdentity,
) -> Option<String> {
    let path = Path::new(displayed);
    // Fixture symlinks carry the absolute fixture root; real host paths have root '/'.
    let relative = path
        .strip_prefix(root)
        .or_else(|_| path.strip_prefix("/"))
        .ok()?;
    if FileIdentity::at(&root.join(relative)).ok() == Some(identity) {
        return Some(format!("/{}", relative.to_string_lossy()));
    }
    // Translate through the filesystem roots of matching mounts. The final identity check
    // is required even when mount paths look right (overmounts and races can change them).
    let process_mounts = mounts(&process.join("mountinfo"));
    let host_mounts = mounts(&root.join("proc/self/mountinfo"));
    for (device, source, mount) in process_mounts {
        let Ok(suffix) = path.strip_prefix(&mount) else {
            continue;
        };
        let filesystem_path = Path::new(&source).join(suffix);
        for (host_device, host_source, host_mount) in &host_mounts {
            if &device != host_device {
                continue;
            }
            let Ok(suffix) = filesystem_path.strip_prefix(host_source) else {
                continue;
            };
            let host_path = Path::new(host_mount).join(suffix);
            let Ok(relative) = host_path.strip_prefix("/") else {
                continue;
            };
            if FileIdentity::at(&root.join(relative)).ok() == Some(identity) {
                return Some(host_path.to_string_lossy().into_owned());
            }
        }
    }
    None
}

fn mounts(path: &Path) -> Vec<(String, String, String)> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            Some((
                fields.get(2)?.to_string(),
                unescape_proc_path(fields.get(3)?),
                unescape_proc_path(fields.get(4)?),
            ))
        })
        .collect()
}

fn unescape_proc_path(path: &str) -> String {
    path.replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}
