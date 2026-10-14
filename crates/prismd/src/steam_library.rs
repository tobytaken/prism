//! The caller's installed Steam apps, read from the Steam roots in their own home: package
//! installs (Valve and Debian layouts), Flatpak and Snap.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use super::session::SessionPeer;
use prism_gpu::{GpuCommandError, GpuCommandErrorCode};
use prism_protocol::SteamApp;

/// A Steam install's root relative to the caller's home, and the home Steam sees inside its
/// sandbox, if that differs from the caller's.
struct SteamRoot {
    path: &'static str,
    sandbox_home: Option<&'static str>,
}

const STEAM_ROOTS: [SteamRoot; 5] = [
    // Valve's package, and its legacy location.
    SteamRoot {
        path: ".local/share/Steam",
        sandbox_home: None,
    },
    SteamRoot {
        path: "Steam",
        sandbox_home: None,
    },
    // Debian's steam-installer.
    SteamRoot {
        path: ".steam/debian-installation",
        sandbox_home: None,
    },
    // Flatpak persists the sandbox's home here, and records library paths under the home it sees.
    SteamRoot {
        path: ".var/app/com.valvesoftware.Steam/.local/share/Steam",
        sandbox_home: Some(".var/app/com.valvesoftware.Steam"),
    },
    // Snap runs Steam with its home set to this directory's real path.
    SteamRoot {
        path: "snap/steam/common/.local/share/Steam",
        sandbox_home: None,
    },
];

/// Known non-game apps whose install folders don't carry the compatibility-tool marker, and the
/// Steam runtimes in case their install folder is missing.
const NON_GAME_APP_IDS: [(u32, &str); 10] = [
    (228980, "Steamworks Common Redistributables"),
    (1007, "Steamworks SDK Redist"),
    (1826330, "Proton EasyAntiCheat Runtime"),
    (1161040, "Proton BattlEye Runtime"),
    (1070560, "Steam Linux Runtime 1.0 (scout)"),
    (1391110, "Steam Linux Runtime 2.0 (soldier)"),
    (1628350, "Steam Linux Runtime 3.0 (sniper)"),
    (4183110, "Steam Linux Runtime 4.0"),
    (4185400, "Steam Linux Runtime 4.0 (arm64)"),
    (4690330, "Legacy Steam Runtime"),
];

/// Marks a compatibility tool's install folder: every Proton version and Steam Linux Runtime.
const COMPATIBILITY_TOOL_MARKER: &str = "toolmanifest.vdf";

/// Steam's flag for an app whose files are fully installed, even if an update is pending.
const FULLY_INSTALLED: u64 = 4;

/// Manifests and library lists are small; anything bigger isn't Steam's.
const MAX_FILE_BYTES: u64 = 1 << 20;

/// Fully installed apps from every library folder of the caller's Steam roots, without
/// compatibility tools or known non-game apps, each listed once and ordered by name.
///
/// The daemon runs as root, but a library list or link can point anywhere, so it reads with the
/// caller's file access and never shows them apps they couldn't list themselves.
pub fn installed_steam_apps(
    host_root: &Path,
    peer: &SessionPeer,
) -> Result<Vec<SteamApp>, GpuCommandError> {
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } != 0 {
        return Ok(scan(host_root, &peer.home));
    }
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                become_caller(peer.uid)?;
                Ok(scan(host_root, &peer.home))
            })
            .join()
            .unwrap_or_else(|_| Err(io::Error::other("the Steam library scan panicked")))
    })
    .map_err(|error: io::Error| {
        GpuCommandError::new(
            GpuCommandErrorCode::Io,
            format!("Prism couldn't read your Steam libraries as you: {error}"),
        )
    })
}

/// Gives the calling thread the user's file-system identity and groups, which also drops root's
/// file access overrides. It never switches back, so the thread must end after its reads.
fn become_caller(uid: u32) -> io::Result<()> {
    let user = users::get_user_by_uid(uid)
        .ok_or_else(|| io::Error::other(format!("no user has uid {uid}")))?;
    let gid = user.primary_group_id();
    let groups = crate::host::user_group_ids(user.name(), gid).unwrap_or_default();
    // Raw system calls change only this thread's credentials; glibc's wrappers would change every
    // thread's. setfsuid and setfsgid report the previous id rather than failure, so asking again
    // with an invalid id confirms the change.
    // SAFETY: the group list outlives the call, and the other calls take plain integers.
    unsafe {
        if libc::syscall(libc::SYS_setgroups, groups.len(), groups.as_ptr()) != 0 {
            return Err(io::Error::last_os_error());
        }
        libc::syscall(libc::SYS_setfsgid, gid);
        libc::syscall(libc::SYS_setfsuid, uid);
        if libc::syscall(libc::SYS_setfsgid, u32::MAX) as u32 != gid
            || libc::syscall(libc::SYS_setfsuid, u32::MAX) as u32 != uid
        {
            return Err(io::Error::other("the file-system identity didn't change"));
        }
    }
    Ok(())
}

fn scan(host_root: &Path, home: &Path) -> Vec<SteamApp> {
    let on_host = |path: &Path| host_root.join(path.strip_prefix("/").unwrap_or(path));
    let mut seen_libraries = HashSet::new();
    let mut apps = BTreeMap::new();
    for root in STEAM_ROOTS {
        for library in libraries(&root, home, on_host)
            .iter()
            .map(|path| on_host(path))
        {
            // A library listed by several roots, or reached through a link, is read once.
            let Ok(metadata) = fs::metadata(library.join("steamapps")) else {
                continue;
            };
            if !seen_libraries.insert((metadata.dev(), metadata.ino())) {
                continue;
            }
            for app in library_apps(&library) {
                apps.entry(app.app_id).or_insert(app);
            }
        }
    }
    let mut apps: Vec<_> = apps.into_values().collect();
    apps.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.app_id.cmp(&b.app_id))
    });
    apps
}

/// The root itself and every library folder it records, as paths on the host. A sandboxed Steam's
/// path under the caller's home may lie in the sandbox's home instead, so both are tried.
fn libraries(root: &SteamRoot, home: &Path, on_host: impl Fn(&Path) -> PathBuf) -> Vec<PathBuf> {
    let root_path = home.join(root.path);
    let mut libraries = vec![root_path.clone()];
    for list in ["steamapps/libraryfolders.vdf", "config/libraryfolders.vdf"] {
        let Some(folders) = read_vdf(&on_host(&root_path.join(list))) else {
            continue;
        };
        let Some(Vdf::Map(folders)) = folders.get("libraryfolders") else {
            continue;
        };
        for (_, folder) in folders {
            let Some(Vdf::Str(path)) = folder.get("path") else {
                continue;
            };
            let path = Path::new(path);
            if !path.is_absolute() {
                continue;
            }
            if let (Some(sandbox_home), Ok(relative)) = (root.sandbox_home, path.strip_prefix(home))
            {
                libraries.push(home.join(sandbox_home).join(relative));
            }
            libraries.push(path.to_path_buf());
        }
    }
    libraries
}

fn library_apps(library: &Path) -> Vec<SteamApp> {
    let Ok(entries) = fs::read_dir(library.join("steamapps")) else {
        return vec![];
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let file_name = entry.file_name();
            let app_id = file_name
                .to_str()?
                .strip_prefix("appmanifest_")?
                .strip_suffix(".acf")?
                .parse::<u32>()
                .ok()?;
            if NON_GAME_APP_IDS.iter().any(|(id, _)| *id == app_id) {
                return None;
            }
            let manifest = read_vdf(&entry.path())?;
            let state = manifest.get("AppState")?;
            let flags: u64 = state.get("StateFlags")?.as_str()?.parse().ok()?;
            if flags & FULLY_INSTALLED == 0 {
                return None;
            }
            if state.get("appid")?.as_str()?.parse::<u32>().ok()? != app_id {
                return None;
            }
            let install_dir = state.get("installdir")?.as_str()?;
            if install_dir.is_empty() || install_dir.contains('/') || install_dir == ".." {
                return None;
            }
            let installed = library.join("steamapps/common").join(install_dir);
            if installed.join(COMPATIBILITY_TOOL_MARKER).exists() {
                return None;
            }
            let name = state.get("name")?.as_str()?.trim();
            Some(SteamApp {
                app_id,
                name: if name.is_empty() {
                    format!("Steam app {app_id}")
                } else {
                    name.to_string()
                },
            })
        })
        .collect()
}

/// Reads a regular file of Steam's text KeyValues format, if it is one.
fn read_vdf(path: &Path) -> Option<Vdf> {
    if !fs::metadata(path).ok()?.is_file() {
        return None;
    }
    // Never block on a file swapped for a FIFO since the check.
    let file: File = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let mut contents = String::new();
    file.take(MAX_FILE_BYTES)
        .read_to_string(&mut contents)
        .ok()?;
    parse_vdf(&contents)
}

/// A KeyValues value: a string, or keys with values in file order.
#[derive(Debug, PartialEq)]
enum Vdf {
    Str(String),
    Map(Vec<(String, Vdf)>),
}

impl Vdf {
    /// The first value under the key, compared regardless of letter case as Steam does.
    fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Map(entries) => entries
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(key))
                .map(|(_, value)| value),
            Vdf::Str(_) => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Vdf::Str(value) => Some(value),
            Vdf::Map(_) => None,
        }
    }
}

#[derive(Debug, PartialEq)]
enum Token {
    Text(String),
    Open,
    Close,
}

fn tokens(contents: &str) -> Option<Vec<Token>> {
    let mut tokens = vec![];
    let mut chars = contents.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {}
            '{' => tokens.push(Token::Open),
            '}' => tokens.push(Token::Close),
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '"' => {
                let mut text = String::new();
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => match chars.next()? {
                            'n' => text.push('\n'),
                            't' => text.push('\t'),
                            other => text.push(other),
                        },
                        other => text.push(other),
                    }
                }
                tokens.push(Token::Text(text));
            }
            c => {
                let mut text = c.to_string();
                while let Some(&next) = chars.peek() {
                    if next.is_whitespace() || matches!(next, '{' | '}' | '"') {
                        break;
                    }
                    text.push(next);
                    chars.next();
                }
                tokens.push(Token::Text(text));
            }
        }
    }
    Some(tokens)
}

fn parse_vdf(contents: &str) -> Option<Vdf> {
    let tokens = tokens(contents)?;
    let mut tokens = tokens.into_iter();
    let root = parse_entries(&mut tokens, 0)?;
    Some(root)
}

/// Bounds nesting, which Steam's own files keep shallow.
const MAX_DEPTH: usize = 32;

fn parse_entries(tokens: &mut impl Iterator<Item = Token>, depth: usize) -> Option<Vdf> {
    if depth > MAX_DEPTH {
        return None;
    }
    let mut entries = vec![];
    loop {
        let key = match tokens.next() {
            Some(Token::Text(key)) => key,
            Some(Token::Close) if depth > 0 => return Some(Vdf::Map(entries)),
            None if depth == 0 => return Some(Vdf::Map(entries)),
            _ => return None,
        };
        let value = match tokens.next()? {
            Token::Text(value) => Vdf::Str(value),
            Token::Open => parse_entries(tokens, depth + 1)?,
            Token::Close => return None,
        };
        entries.push((key, value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs root, or root in a user namespace with other ids mapped, such as
    /// `unshare --map-root-user --map-auto cargo test`.
    #[test]
    fn as_root_a_scan_lists_only_libraries_the_caller_can_read() {
        // SAFETY: geteuid has no preconditions.
        if unsafe { libc::geteuid() } != 0 {
            eprintln!("skipped: switching to the caller's identity needs root");
            return;
        }
        use std::os::unix::fs::PermissionsExt;
        let host = tempfile::tempdir().unwrap();
        fs::set_permissions(host.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let write = |path: &str, contents: &str| {
            let path = host.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        };
        let manifest = |app_id: u32, name: &str| {
            format!(
                "\"AppState\" {{ \"appid\" \"{app_id}\" \"name\" \"{name}\" \"StateFlags\" \"4\" \"installdir\" \"{name}\" }}"
            )
        };
        let steam = "home/nobody/.local/share/Steam";
        write(
            &format!("{steam}/steamapps/libraryfolders.vdf"),
            "\"libraryfolders\" { \"1\" { \"path\" \"/home/private/SteamLibrary\" } }",
        );
        write(
            &format!("{steam}/steamapps/appmanifest_570.acf"),
            &manifest(570, "Dota 2"),
        );
        write(
            "home/private/SteamLibrary/steamapps/appmanifest_620.acf",
            &manifest(620, "Portal 2"),
        );
        let private = host.path().join("home/private");
        fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).unwrap();
        let nobody = users::get_user_by_name("nobody").expect("a nobody user");
        let peer = SessionPeer {
            uid: nobody.uid(),
            username: "nobody".into(),
            groups: vec![],
            primary_group: "nogroup".into(),
            home: "/home/nobody".into(),
        };

        assert_eq!(
            installed_steam_apps(host.path(), &peer).unwrap(),
            vec![SteamApp {
                app_id: 570,
                name: "Dota 2".into()
            }]
        );
        // Only the scan's own thread took the caller's identity.
        assert_eq!(scan(host.path(), Path::new("/home/nobody")).len(), 2);
    }

    #[test]
    fn keyvalues_keep_nesting_escapes_and_comments() {
        let parsed = parse_vdf(
            "// comment\n\"AppState\"\n{\n\t\"name\"\t\"Say \\\"hi\\\"\"\n\tbare value\n\t\"apps\" { \"570\" \"1\" }\n}\n",
        )
        .unwrap();
        let state = parsed.get("appstate").unwrap();
        assert_eq!(state.get("NAME").unwrap().as_str(), Some("Say \"hi\""));
        assert_eq!(state.get("bare").unwrap().as_str(), Some("value"));
        assert_eq!(
            state.get("apps").unwrap().get("570").unwrap().as_str(),
            Some("1")
        );
        for malformed in ["\"a\" {", "\"a\"", "}", "\"a\" \"b"] {
            assert_eq!(parse_vdf(malformed), None, "{malformed}");
        }
    }
}
