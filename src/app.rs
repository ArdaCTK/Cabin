use std::{
    cmp::Ordering,
    env,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::SystemTime,
};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Local};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use directories_next::{BaseDirs, UserDirs};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Places,
    Contents,
    Preview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
    #[allow(dead_code)]
    Drive,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    pub kind: EntryKind,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub extension: Option<String>,
    pub is_hidden: bool,
}

#[derive(Debug, Clone)]
pub struct Place {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct PreviewData {
    pub lines: Vec<String>,
}

#[derive(Debug)]
pub struct App {
    pub should_quit: bool,
    pub active_panel: Panel,
    pub current_dir: PathBuf,
    pub places: Vec<Place>,
    pub entries: Vec<FileEntry>,
    pub places_selected: usize,
    pub contents_selected: usize,
    pub preview: PreviewData,
    pub show_hidden: bool,
    pub status_message: Option<String>,
    pub help_visible: bool,
}

impl App {
    pub fn new() -> Result<Self> {
        let current_dir = starting_dir();
        let places = build_places();
        let mut app = Self {
            should_quit: false,
            active_panel: Panel::Contents,
            current_dir,
            places,
            entries: Vec::new(),
            places_selected: 0,
            contents_selected: 0,
            preview: PreviewData { lines: Vec::new() },
            show_hidden: false,
            status_message: Some(String::from("Cabin is ready.")),
            help_visible: false,
        };

        app.sync_places_selection();
        app.refresh_entries()?;
        Ok(app)
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if self.help_visible {
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') => {
                    self.help_visible = false;
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.help_visible = true,
            KeyCode::Tab => self.next_panel(),
            KeyCode::BackTab => self.previous_panel(),
            KeyCode::Char('h') => self.toggle_hidden(),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Enter => self.open_selected(),
            KeyCode::Backspace | KeyCode::Left => self.go_parent(),
            KeyCode::Char('r') => self.set_status("Rename is coming in a later version."),
            KeyCode::Char('d') => self.set_status("Delete is coming in a later version."),
            KeyCode::Char('n') => self.set_status("New file creation is coming in a later version."),
            KeyCode::Char('N') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.set_status("New folder creation is coming in a later version.")
            }
            _ => {}
        }
    }

    pub fn active_preview_lines(&self) -> Vec<String> {
        if self.help_visible {
            return help_lines();
        }

        match self.active_panel {
            Panel::Places => self.place_preview_lines(),
            Panel::Contents | Panel::Preview => self.entry_preview_lines(),
        }
    }

    fn refresh_preview(&mut self) {
        self.preview = PreviewData {
            lines: self.active_preview_lines(),
        };
    }

    pub fn place_preview_lines(&self) -> Vec<String> {
        if self.places.is_empty() {
            return vec![String::from("No places available.")];
        }

        let index = self.places_selected.min(self.places.len().saturating_sub(1));
        let place = &self.places[index];
        vec![
            format!("Type: Shortcut"),
            format!("Name: {}", place.name),
            format!("Path: {}", place.path.display()),
        ]
    }

    pub fn entry_preview_lines(&self) -> Vec<String> {
        let Some(entry) = self.current_selection() else {
            return vec![
                String::from("Type: Folder"),
                format!("Path: {}", self.current_dir.display()),
                String::from("Items: 0"),
            ];
        };

        if entry.kind == EntryKind::Directory {
            let child_count = fs::read_dir(&entry.path).map(|it| it.count()).unwrap_or(0);
            vec![
                String::from("Type: Folder"),
                format!("Path: {}", entry.path.display()),
                format!("Items: {}", child_count),
            ]
        } else {
            vec![
                String::from("Type: File"),
                format!(
                    "Extension: {}",
                    entry
                        .extension
                        .clone()
                        .unwrap_or_else(|| String::from("(none)"))
                ),
                format!(
                    "Size: {}",
                    entry
                        .size
                        .map(human_size)
                        .unwrap_or_else(|| String::from("Unknown"))
                ),
                format!(
                    "Modified: {}",
                    entry
                        .modified
                        .map(format_system_time)
                        .unwrap_or_else(|| String::from("Unknown"))
                ),
                format!("Path: {}", entry.path.display()),
            ]
        }
    }

    fn open_selected(&mut self) {
        let result = match self.active_panel {
            Panel::Places => self.open_place(),
            Panel::Contents | Panel::Preview => self.open_current_entry(),
        };

        if let Err(err) = result {
            self.set_status(format!("Error: {err}"));
        }
    }

    fn open_place(&mut self) -> Result<()> {
        let place = self
            .places
            .get(self.places_selected)
            .ok_or_else(|| anyhow!("No place selected"))?
            .clone();
        self.open_directory(place.path)?;
        self.set_status(format!("Opened {}", place.name));
        Ok(())
    }

    fn open_current_entry(&mut self) -> Result<()> {
        let entry = self
            .current_selection()
            .cloned()
            .ok_or_else(|| anyhow!("No item selected"))?;

        match entry.kind {
            EntryKind::Directory => {
                self.open_directory(entry.path)?;
                self.set_status(format!("Opened {}", entry.name));
            }
            EntryKind::File | EntryKind::Symlink | EntryKind::Unknown => {
                open_with_system(&entry.path).with_context(|| {
                    format!("Could not open {}", entry.path.display())
                })?;
                self.set_status(format!("Opened externally: {}", entry.name));
            }
            EntryKind::Drive => {
                self.open_directory(entry.path)?;
                self.set_status(format!("Opened {}", entry.name));
            }
        }

        Ok(())
    }

    fn open_directory(&mut self, path: PathBuf) -> Result<()> {
        let canonical = path;
        if !canonical.exists() {
            return Err(anyhow!("Path does not exist"));
        }

        self.current_dir = canonical;
        self.sync_places_selection();
        self.refresh_entries()?;
        Ok(())
    }

    fn go_parent(&mut self) {
        let Some(parent) = self.current_dir.parent().map(Path::to_path_buf) else {
            self.set_status("Already at the top level.");
            return;
        };

        if let Err(err) = self.open_directory(parent) {
            self.set_status(format!("Error: {err}"));
        }
    }

    fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        match self.refresh_entries() {
            Ok(()) => {
                let state = if self.show_hidden { "shown" } else { "hidden" };
                self.set_status(format!("Hidden files are now {state}."));
            }
            Err(err) => self.set_status(format!("Error: {err}")),
        }
    }

    fn move_selection(&mut self, delta: isize) {
        match self.active_panel {
            Panel::Places => {
                if self.places.is_empty() {
                    return;
                }
                self.places_selected = move_index(self.places_selected, delta, self.places.len());
                self.refresh_preview();
            }
            Panel::Contents | Panel::Preview => {
                if self.entries.is_empty() {
                    return;
                }
                self.contents_selected =
                    move_index(self.contents_selected, delta, self.entries.len());
                self.refresh_preview();
            }
        }
    }

    fn next_panel(&mut self) {
        self.active_panel = match self.active_panel {
            Panel::Places => Panel::Contents,
            Panel::Contents => Panel::Preview,
            Panel::Preview => Panel::Places,
        };
        self.refresh_preview();
    }

    fn previous_panel(&mut self) {
        self.active_panel = match self.active_panel {
            Panel::Places => Panel::Preview,
            Panel::Contents => Panel::Places,
            Panel::Preview => Panel::Contents,
        };
        self.refresh_preview();
    }

    fn current_selection(&self) -> Option<&FileEntry> {
        self.entries.get(self.contents_selected)
    }

    fn refresh_entries(&mut self) -> Result<()> {
        self.entries = read_directory(&self.current_dir, self.show_hidden)?;
        self.contents_selected = self.contents_selected.min(self.entries.len().saturating_sub(1));
        self.refresh_preview();
        Ok(())
    }

    fn sync_places_selection(&mut self) {
        if let Some((index, _)) = self
            .places
            .iter()
            .enumerate()
            .find(|(_, place)| same_path(&place.path, &self.current_dir))
        {
            self.places_selected = index;
        }
    }

    fn set_status<S: Into<String>>(&mut self, message: S) {
        self.status_message = Some(message.into());
    }
}

fn starting_dir() -> PathBuf {
    if let Some(base) = BaseDirs::new() {
        return base.home_dir().to_path_buf();
    }
    env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn build_places() -> Vec<Place> {
    let mut places = Vec::new();

    if let Some(base) = BaseDirs::new() {
        places.push(Place {
            name: String::from("Home"),
            path: base.home_dir().to_path_buf(),
        });
    }

    if let Some(user_dirs) = UserDirs::new() {
        add_if_exists(&mut places, "Desktop", user_dirs.desktop_dir().map(Path::to_path_buf));
        add_if_exists(&mut places, "Downloads", user_dirs.download_dir().map(Path::to_path_buf));
        add_if_exists(&mut places, "Documents", user_dirs.document_dir().map(Path::to_path_buf));
        add_if_exists(&mut places, "Pictures", user_dirs.picture_dir().map(Path::to_path_buf));
        add_if_exists(&mut places, "Videos", user_dirs.video_dir().map(Path::to_path_buf));
        add_if_exists(&mut places, "Music", user_dirs.audio_dir().map(Path::to_path_buf));
    }

    #[cfg(windows)]
    {
        for letter in b'A'..=b'Z' {
            let drive = format!("{}:\\", letter as char);
            let path = PathBuf::from(&drive);
            if path.exists() {
                places.push(Place {
                    name: format!("{}:", letter as char),
                    path,
                });
            }
        }
    }

    #[cfg(not(windows))]
    {
        places.push(Place {
            name: String::from("Root"),
            path: PathBuf::from("/"),
        });
    }

    places.dedup_by(|a, b| same_path(&a.path, &b.path));
    places
}

fn add_if_exists(places: &mut Vec<Place>, name: &str, path: Option<PathBuf>) {
    if let Some(path) = path {
        if path.exists() {
            places.push(Place {
                name: String::from(name),
                path,
            });
        }
    }
}

fn read_directory(dir: &Path, show_hidden: bool) -> Result<Vec<FileEntry>> {
    let mut entries = Vec::new();
    let read_dir = fs::read_dir(dir).with_context(|| format!("Unable to read {}", dir.display()))?;

    for item in read_dir {
        let item = item?;
        let path = item.path();
        let name = item.file_name().to_string_lossy().to_string();
        let metadata = item.metadata().ok();
        let file_type = item.file_type().ok();
        let is_hidden = is_hidden(&path, &name, metadata.as_ref());

        if is_hidden && !show_hidden {
            continue;
        }

        let kind = if file_type.map(|ft| ft.is_dir()).unwrap_or(false) {
            EntryKind::Directory
        } else if file_type.map(|ft| ft.is_symlink()).unwrap_or(false) {
            EntryKind::Symlink
        } else if file_type.map(|ft| ft.is_file()).unwrap_or(false) {
            EntryKind::File
        } else {
            EntryKind::Unknown
        };

        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| format!(".{ext}"));

        entries.push(FileEntry {
            name,
            path,
            kind,
            size: metadata.as_ref().map(|meta| meta.len()),
            modified: metadata.as_ref().and_then(|meta| meta.modified().ok()),
            extension,
            is_hidden,
        });
    }

    entries.sort_by(compare_entries);
    Ok(entries)
}

fn compare_entries(left: &FileEntry, right: &FileEntry) -> Ordering {
    match (left.kind == EntryKind::Directory, right.kind == EntryKind::Directory) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => left
            .name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.name.cmp(&right.name)),
    }
}

fn is_hidden(_path: &Path, name: &str, metadata: Option<&fs::Metadata>) -> bool {
    if name.starts_with('.') {
        return true;
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if let Some(metadata) = metadata {
            return metadata.file_attributes() & 0x2 != 0;
        }
    }

    #[cfg(not(windows))]
    let _ = metadata;

    false
}

fn move_index(current: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }

    let next = current as isize + delta;
    next.clamp(0, len.saturating_sub(1) as isize) as usize
}

fn same_path(left: &Path, right: &Path) -> bool {
    if cfg!(windows) {
        left.to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .eq_ignore_ascii_case(right.to_string_lossy().trim_end_matches(['\\', '/']))
    } else {
        left == right
    }
}

fn human_size(size: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = size as f64;
    let mut unit = 0usize;

    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{size} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn format_system_time(time: SystemTime) -> String {
    let datetime: DateTime<Local> = time.into();
    datetime.format("%Y-%m-%d %H:%M").to_string()
}

fn open_with_system(path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        Command::new("cmd")
            .args(["/C", "start", "", &path.to_string_lossy()])
            .status()
            .with_context(|| format!("Failed to launch {}", path.display()))?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("The system failed to open the item"))?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(path)
            .status()
            .with_context(|| format!("Failed to launch {}", path.display()))?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("The system failed to open the item"))?;
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(path)
            .status()
            .with_context(|| format!("Failed to launch {}", path.display()))?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("The system failed to open the item"))?;
    }

    Ok(())
}

fn help_lines() -> Vec<String> {
    vec![
        String::from("Cabin help"),
        String::new(),
        String::from("q          Quit"),
        String::from("?          Toggle help"),
        String::from("Tab        Next panel"),
        String::from("Shift+Tab  Previous panel"),
        String::from("Up/Down    Move selection"),
        String::from("j/k        Move selection"),
        String::from("Enter      Open selected item"),
        String::from("Backspace  Parent folder"),
        String::from("Left       Parent folder"),
        String::from("h          Toggle hidden files"),
    ]
}
