//! The caller's installed Steam apps, read from the Steam roots in their own home: package
//! installs (Valve and Debian layouts), Flatpak and Snap.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use crate::protocol::SteamApp;

/// Steam roots relative to the caller's home.
const STEAM_ROOTS: [&str; 5] = [
    // Valve's package, and its legacy location.
    ".local/share/Steam",
    "Steam",
    // Debian's steam-installer.
    ".steam/debian-installation",
    ".var/app/com.valvesoftware.Steam/.local/share/Steam",
    "snap/steam/common/.local/share/Steam",
];

/// Known non-game apps whose install folders don't carry the compatibility-tool marker, and the
/// Steam runtimes in case their install folder is missing.
const NON_GAME_APP_IDS: [u32; 10] = [
    // Steamworks Common Redistributables.
    228980, // Proton EasyAntiCheat Runtime and Proton BattlEye Runtime.
    1826330, 1161040,
    // Steam Linux Runtime 1.0 (scout), 2.0 (soldier), 3.0 (sniper), 4.0, and their variants.
    1070560, 1391110, 1628350, 4183110, 4185400, // The Legacy Steam Runtime.
    4690330, // Steamworks SDK Redist.
    1007,
];

/// Marks a compatibility tool's install folder: every Proton version and Steam Linux Runtime.
const COMPATIBILITY_TOOL_MARKER: &str = "toolmanifest.vdf";

/// Steam's flag for an app whose files are fully installed, even if an update is pending.
const FULLY_INSTALLED: u64 = 4;

/// Manifests and library lists are small; anything bigger isn't Steam's.
const MAX_FILE_BYTES: u64 = 1 << 20;

/// Fully installed apps from every library folder of the caller's Steam roots, without
/// compatibility tools or known non-game apps, each listed once and ordered by name.
pub fn installed_steam_apps(host_root: &Path, home: &Path) -> Vec<SteamApp> {
    let on_host = |path: &Path| host_root.join(path.strip_prefix("/").unwrap_or(path));
    let mut seen_libraries = HashSet::new();
    let mut apps = BTreeMap::new();
    for root in STEAM_ROOTS {
        let root = on_host(&home.join(root));
        for library in libraries(&root, &on_host) {
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

/// The root itself and every library folder it records.
fn libraries(root: &Path, on_host: &impl Fn(&Path) -> PathBuf) -> Vec<PathBuf> {
    let mut libraries = vec![root.to_path_buf()];
    for list in ["steamapps/libraryfolders.vdf", "config/libraryfolders.vdf"] {
        let Some(folders) = read_vdf(&root.join(list)) else {
            continue;
        };
        let Some(Vdf::Map(folders)) = folders.get("libraryfolders") else {
            continue;
        };
        for (_, folder) in folders {
            if let Some(Vdf::Str(path)) = folder.get("path") {
                if Path::new(path).is_absolute() {
                    libraries.push(on_host(Path::new(path)));
                }
            }
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
            if NON_GAME_APP_IDS.contains(&app_id) {
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
