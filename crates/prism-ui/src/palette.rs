//! The command palette: its command catalog with each command's availability, fuzzy matching and
//! recent-first ordering. Also the keyboard shortcuts, which run the same commands.

use crate::core::{Core, Section, Selection};
use crate::view::gpu_label;

/// Something the palette or a shortcut runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Command {
    /// Show this GPU, by id, in the workspace.
    GoToGpu(String),
    GoTo(Section),
    Quit,
}

/// How many recently run commands the palette remembers, to list first.
const RECENT: usize = 8;

/// The open palette's search.
#[derive(Debug, Default)]
pub(crate) struct PaletteState {
    query: String,
    /// The highlighted entry, as an index into the matches.
    highlight: usize,
}

/// The open command palette, as shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    pub query: String,
    /// The commands that match the query, recently run ones first.
    pub entries: Vec<PaletteEntry>,
}

/// A command in the palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteEntry {
    pub title: String,
    /// The keys of the command's shortcut, such as `["Ctrl", "Shift", "G"]`; empty when it has
    /// none.
    pub keys: Vec<String>,
    /// Why the command can't run now, when it can't. It stays listed, dimmed.
    pub unavailable: Option<String>,
    pub highlighted: bool,
}

impl PaletteEntry {
    /// The shortcut as one string, such as "Ctrl+Shift+G".
    pub fn shortcut(&self) -> String {
        self.keys.join("+")
    }

    /// Why the command can't run and its shortcut, as a screen reader adds them to the title.
    pub fn description(&self) -> String {
        [self.unavailable.clone(), Some(self.shortcut())]
            .into_iter()
            .flatten()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

/// A key press, as the window reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Key {
    /// The key's text, or a control character for a special key such as Escape.
    pub text: String,
    pub control: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}

/// A global keyboard shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    /// Ctrl+K.
    OpenPalette,
    /// Ctrl+1 … Ctrl+9: the GPU at this index in sidebar order.
    GoToGpu(usize),
    /// Ctrl+Shift+G, Ctrl+Shift+D and Ctrl+,.
    GoTo(Section),
    /// Ctrl+Q.
    Quit,
    /// Escape: close the topmost overlay.
    Dismiss,
}

const ESCAPE: &str = "\u{1b}";

impl Shortcut {
    /// The shortcut `key` presses, if any.
    pub fn for_key(key: &Key) -> Option<Self> {
        if key.alt || key.meta {
            return None;
        }
        let text = key.text.to_lowercase();
        Some(match (key.control, key.shift, text.as_str()) {
            (false, false, ESCAPE) => Self::Dismiss,
            (true, false, "k") => Self::OpenPalette,
            (true, false, "q") => Self::Quit,
            (true, false, ",") => Self::GoTo(Section::Settings),
            (true, true, "g") => Self::GoTo(Section::Games),
            (true, true, "d") => Self::GoTo(Section::Drivers),
            (true, false, digit) => match digit.parse::<usize>() {
                Ok(number @ 1..=9) => Self::GoToGpu(number - 1),
                _ => return None,
            },
            _ => return None,
        })
    }

    /// Whether a focused text field keeps this key for editing, so the shortcut doesn't fire
    /// there.
    pub fn clashes_with_text_editing(self) -> bool {
        matches!(self, Self::GoToGpu(_))
    }

    /// The keys to press, as shown next to a command.
    pub fn keys(self) -> Vec<String> {
        let keys: &[&str] = match self {
            Self::OpenPalette => &["Ctrl", "K"],
            Self::GoToGpu(index) => return vec!["Ctrl".into(), (index + 1).to_string()],
            Self::GoTo(Section::Games) => &["Ctrl", "Shift", "G"],
            Self::GoTo(Section::Drivers) => &["Ctrl", "Shift", "D"],
            Self::GoTo(Section::Settings) => &["Ctrl", ","],
            Self::Quit => &["Ctrl", "Q"],
            Self::Dismiss => &["Esc"],
        };
        keys.iter().map(|&key| key.to_owned()).collect()
    }
}

/// A command as the catalog lists it.
struct CatalogEntry {
    command: Command,
    title: String,
    shortcut: Option<Shortcut>,
    unavailable: Option<String>,
}

impl Core {
    /// The open palette, if it is open.
    pub fn palette(&self) -> Option<Palette> {
        let state = self.palette.as_ref()?;
        Some(Palette {
            query: state.query.clone(),
            entries: self
                .matches()
                .into_iter()
                .enumerate()
                .map(|(index, entry)| PaletteEntry {
                    title: entry.title,
                    keys: entry.shortcut.map(Shortcut::keys).unwrap_or_default(),
                    unavailable: entry.unavailable,
                    highlighted: index == state.highlight,
                })
                .collect(),
        })
    }

    pub fn palette_open(&self) -> bool {
        self.palette.is_some()
    }

    pub(crate) fn open_palette(&mut self) {
        if self.palette.is_none() {
            self.palette = Some(PaletteState::default());
            self.highlight_first_available();
        }
    }

    /// Closes the topmost overlay, which so far can only be the palette.
    pub(crate) fn dismiss(&mut self) {
        self.palette = None;
    }

    pub(crate) fn filter_palette(&mut self, query: String) {
        if let Some(state) = &mut self.palette {
            state.query = query;
            self.highlight_first_available();
        }
    }

    /// Moves the highlight `by` entries, wrapping around at either end.
    pub(crate) fn move_highlight(&mut self, by: isize) {
        let count = self.matches().len();
        if let Some(state) = &mut self.palette
            && count > 0
        {
            state.highlight = (state.highlight as isize + by).rem_euclid(count as isize) as usize;
        }
    }

    /// The matching command at `index`, or the highlighted one.
    pub(crate) fn palette_command(&self, index: Option<usize>) -> Option<Command> {
        let state = self.palette.as_ref()?;
        let index = index.unwrap_or(state.highlight);
        self.matches()
            .into_iter()
            .nth(index)
            .map(|entry| entry.command)
    }

    /// The command a shortcut runs, if it runs one here.
    pub(crate) fn shortcut_command(&self, shortcut: Shortcut) -> Option<Command> {
        match shortcut {
            Shortcut::GoToGpu(index) => self
                .gpus()
                .get(index)
                .map(|gpu| Command::GoToGpu(gpu.id.clone())),
            Shortcut::GoTo(section) => Some(Command::GoTo(section)),
            Shortcut::Quit => Some(Command::Quit),
            Shortcut::OpenPalette | Shortcut::Dismiss => None,
        }
    }

    /// Why `command` can't run now, when it can't.
    pub(crate) fn unavailable(&self, command: &Command) -> Option<String> {
        let already_open = match command {
            Command::GoToGpu(id) => self.selection.as_ref() == Some(&Selection::Gpu(id.clone())),
            Command::GoTo(section) => self.selection == Some(Selection::Section(*section)),
            Command::Quit => false,
        };
        already_open.then(|| "Already open".into())
    }

    /// Notes that `command` ran, so the palette lists it first, and closes the palette.
    pub(crate) fn ran(&mut self, command: &Command) {
        self.recent.retain(|recent| recent != command);
        self.recent.insert(0, command.clone());
        self.recent.truncate(RECENT);
        self.palette = None;
    }

    /// Every command that applies now, in catalog order. Commands that don't apply at all, such
    /// as GPU commands on an empty inventory, are left out.
    fn catalog(&self) -> Vec<CatalogEntry> {
        let navigation = self.gpus().iter().enumerate().map(|(index, gpu)| {
            (
                Command::GoToGpu(gpu.id.clone()),
                format!("Go to {}", gpu_label(index, gpu)),
                (index < 9).then_some(Shortcut::GoToGpu(index)),
            )
        });
        let sections = [Section::Games, Section::Drivers, Section::Settings].map(|section| {
            let name = match section {
                Section::Games => "Games",
                Section::Drivers => "Drivers",
                Section::Settings => "Settings",
            };
            (
                Command::GoTo(section),
                format!("Go to {name}"),
                Some(Shortcut::GoTo(section)),
            )
        });
        let app = [(Command::Quit, "Quit Prism".to_owned(), Some(Shortcut::Quit))];
        navigation
            .chain(sections)
            .chain(app)
            .map(|(command, title, shortcut)| CatalogEntry {
                unavailable: self.unavailable(&command),
                command,
                title,
                shortcut,
            })
            .collect()
    }

    /// The catalog's commands that match the query, recently run ones first.
    fn matches(&self) -> Vec<CatalogEntry> {
        let query = self
            .palette
            .as_ref()
            .map(|state| state.query.as_str())
            .unwrap_or_default();
        let mut matches: Vec<_> = self
            .catalog()
            .into_iter()
            .filter(|entry| fuzzy_match(query, &entry.title))
            .collect();
        matches.sort_by_key(|entry| {
            self.recent
                .iter()
                .position(|recent| *recent == entry.command)
                .unwrap_or(usize::MAX)
        });
        matches
    }

    fn highlight_first_available(&mut self) {
        let first = self
            .matches()
            .iter()
            .position(|entry| entry.unavailable.is_none())
            .unwrap_or(0);
        if let Some(state) = &mut self.palette {
            state.highlight = first;
        }
    }
}

/// Whether the query's characters, ignoring case and whitespace, appear in order in `title`.
fn fuzzy_match(query: &str, title: &str) -> bool {
    let mut title = title.chars().flat_map(char::to_lowercase);
    query
        .chars()
        .filter(|letter| !letter.is_whitespace())
        .flat_map(char::to_lowercase)
        .all(|letter| title.any(|candidate| candidate == letter))
}

#[cfg(test)]
mod tests {
    use super::fuzzy_match;

    #[test]
    fn a_query_matches_its_letters_in_order() {
        assert!(fuzzy_match("", "Go to Games"));
        assert!(fuzzy_match("gtg", "Go to Games"));
        assert!(fuzzy_match("GO  GAMES", "Go to Games"));
        assert!(!fuzzy_match("ggt", "Go to Games"));
        assert!(!fuzzy_match("gamess", "Go to Games"));
    }
}
