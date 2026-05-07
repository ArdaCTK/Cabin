use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet, VecDeque},
    env,
    fs::{self, File},
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread,
    time::SystemTime,
};

use anyhow::{anyhow, Context, Result};
use arboard::Clipboard;
use chrono::{DateTime, Local};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use directories_next::{BaseDirs, UserDirs};
use ratatui::layout::Rect;
use ratatui_image::picker::Picker;
use trash::delete;

use crate::config::CabinConfig;
use crate::preview::{
    build_image_preview, build_text_preview, build_video_preview, is_supported_text,
    is_supported_video, ImagePreview, ImagePreviewKey, TextPreview, TextPreviewKey, VideoPreview,
    VideoPreviewKey,
};

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
    pub created: Option<SystemTime>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardMode {
    Copy,
    Cut,
}

#[derive(Debug, Clone)]
pub struct PendingOperation {
    pub mode: ClipboardMode,
    pub source: PathBuf,
    pub name: String,
}

#[derive(Debug, Clone)]
pub enum ContentsMode {
    Directory { path: PathBuf },
    SearchCurrent { base: PathBuf, query: String },
    SearchRecursive { base: PathBuf, query: String },
}

#[derive(Debug, Clone)]
pub enum InputAction {
    CreateFile { dir: PathBuf },
    CreateFolder { dir: PathBuf },
    Rename { source: PathBuf },
    SearchCurrent { base: PathBuf },
    SearchRecursive { base: PathBuf },
}

#[derive(Debug, Clone)]
pub enum Dialog {
    Input {
        title: String,
        value: String,
        action: InputAction,
    },
    ConfirmDelete {
        path: PathBuf,
        name: String,
    },
}

struct ImageJob {
    path: PathBuf,
    area: Rect,
}

pub struct App {
    pub should_quit: bool,
    pub active_panel: Panel,
    pub config: CabinConfig,
    pub current_dir: PathBuf,
    pub places: Vec<Place>,
    pub directory_entries: Vec<FileEntry>,
    pub entries: Vec<FileEntry>,
    pub contents_mode: ContentsMode,
    pub places_selected: usize,
    pub contents_selected: usize,
    pub preview: PreviewData,
    pub text_cache: HashMap<TextPreviewKey, TextPreview>,
    pub text_cache_order: VecDeque<TextPreviewKey>,
    pub video_cache: HashMap<VideoPreviewKey, VideoPreview>,
    pub video_cache_order: VecDeque<VideoPreviewKey>,
    pub image_cache: HashMap<ImagePreviewKey, ImagePreview>,
    pub image_cache_order: VecDeque<ImagePreviewKey>,
    pub hovered_place_entries: Vec<FileEntry>,
    pub hovered_place_error: Option<String>,
    image_jobs_tx: Sender<ImageJob>,
    image_jobs_rx: Receiver<ImagePreview>,
    image_pending: HashSet<ImagePreviewKey>,
    pub last_image_area: Option<Rect>,
    pub preview_scroll: u16,
    pub show_hidden: bool,
    pub status_message: Option<String>,
    pub help_visible: bool,
    pub settings_visible: bool,
    pub settings_selected: usize,
    pub dialog: Option<Dialog>,
    pub pending_operation: Option<PendingOperation>,
}

impl App {
    pub fn new() -> Result<Self> {
        let (config, warning, should_init_config) = CabinConfig::load_or_default();
        let mut startup_message = warning.unwrap_or_else(|| String::from("Cabin is ready."));
        if should_init_config {
            if let Err(err) = config.save() {
                startup_message =
                    format!("Cabin is ready, but config.toml could not be created: {err}");
            }
        }
        let show_hidden = config.show_hidden;

        let current_dir = starting_dir();
        let places = build_places();
        let (image_jobs_tx, image_jobs_rx) = spawn_image_worker(
            Picker::from_query_stdio().unwrap_or_else(|_| Picker::from_fontsize((10, 20))),
        );
        let mut app = Self {
            should_quit: false,
            active_panel: Panel::Contents,
            config,
            current_dir,
            places,
            directory_entries: Vec::new(),
            entries: Vec::new(),
            contents_mode: ContentsMode::Directory {
                path: PathBuf::new(),
            },
            places_selected: 0,
            contents_selected: 0,
            preview: PreviewData { lines: Vec::new() },
            text_cache: HashMap::new(),
            text_cache_order: VecDeque::new(),
            video_cache: HashMap::new(),
            video_cache_order: VecDeque::new(),
            image_cache: HashMap::new(),
            image_cache_order: VecDeque::new(),
            hovered_place_entries: Vec::new(),
            hovered_place_error: None,
            image_jobs_tx,
            image_jobs_rx,
            image_pending: HashSet::new(),
            last_image_area: None,
            preview_scroll: 0,
            show_hidden,
            status_message: Some(startup_message),
            help_visible: false,
            settings_visible: false,
            settings_selected: 0,
            dialog: None,
            pending_operation: None,
        };

        app.sync_places_selection();
        app.contents_mode = ContentsMode::Directory {
            path: app.current_dir.clone(),
        };
        app.refresh_entries()?;
        app.refresh_hovered_place_entries();
        Ok(app)
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if self.settings_visible {
            self.handle_settings_key(key);
            return;
        }

        if self.help_visible {
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') => {
                    self.help_visible = false;
                }
                _ => {}
            }
            return;
        }

        if self.handle_dialog_key(key) {
            return;
        }

        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.help_visible = true,
            KeyCode::Char('s') => self.toggle_settings(),
            KeyCode::Tab => self.next_panel(),
            KeyCode::BackTab => self.previous_panel(),
            KeyCode::Char('h') => self.toggle_hidden(),
            KeyCode::Up | KeyCode::Char('k') => self.move_up(),
            KeyCode::Down | KeyCode::Char('j') => self.move_down(),
            KeyCode::Enter => self.open_selected(),
            KeyCode::Backspace | KeyCode::Left => self.go_parent(),
            KeyCode::F(5) => self.refresh_current(),
            KeyCode::Char('/') => self.begin_search_current(),
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.begin_search_recursive()
            }
            KeyCode::Char('y') => self.copy_current_path(),
            KeyCode::Char('c') => self.mark_copy(),
            KeyCode::Char('x') => self.mark_cut(),
            KeyCode::Char('p') => self.paste_pending_operation(),
            KeyCode::Char('r') => self.begin_rename(),
            KeyCode::Char('d') => self.begin_delete(),
            KeyCode::Char('n') => self.begin_new_file(),
            KeyCode::Char('N') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.begin_new_folder()
            }
            _ => {}
        }
    }

    pub fn active_preview_lines(&self) -> Vec<String> {
        if let Some(lines) = self.dialog_preview_lines() {
            return lines;
        }

        if self.help_visible {
            return help_lines();
        }

        let lines = match self.active_panel {
            Panel::Places => self.place_preview_lines(),
            Panel::Contents | Panel::Preview => self.entry_preview_lines(),
        };

        self.with_clipboard_info(lines)
    }

    pub fn dialog_preview_lines(&self) -> Option<Vec<String>> {
        match self.dialog.as_ref() {
            Some(Dialog::Input { title, value, .. }) => Some(vec![
                title.clone(),
                String::new(),
                format!("Name: {value}"),
                String::new(),
                String::from("Enter: confirm"),
                String::from("Esc: cancel"),
                String::from("Backspace: delete last character"),
            ]),
            Some(Dialog::ConfirmDelete { name, .. }) => Some(vec![
                String::from("Delete confirmation"),
                String::new(),
                format!("Move \"{name}\" to Recycle Bin?"),
                String::new(),
                String::from("Y / Enter: yes"),
                String::from("N / Esc: no"),
            ]),
            None => None,
        }
    }

    fn refresh_preview(&mut self) {
        self.refresh_video_preview();
        self.preview = PreviewData {
            lines: self.active_preview_lines(),
        };
        self.preview_scroll = 0;
        self.refresh_text_preview();
    }

    fn refresh_text_preview(&mut self) {
        let Some(entry) = self.current_selection().cloned() else {
            return;
        };

        if entry.kind != EntryKind::File || !is_supported_text(&entry.path) {
            return;
        }

        let key = TextPreviewKey::new(entry.path.clone());
        if self.text_cache.contains_key(&key) {
            return;
        }

        let preview = build_text_preview(&entry.path, 400, 256 * 1024);
        self.text_cache.insert(key.clone(), preview);
        self.text_cache_order.push_back(key);
        while self.text_cache_order.len() > 32 {
            if let Some(oldest) = self.text_cache_order.pop_front() {
                self.text_cache.remove(&oldest);
            }
        }
    }

    pub fn cached_text_preview(&self, path: &Path) -> Option<&TextPreview> {
        let key = TextPreviewKey::new(path.to_path_buf());
        self.text_cache.get(&key)
    }

    fn refresh_video_preview(&mut self) {
        let Some(entry) = self.current_selection().cloned() else {
            return;
        };

        if entry.kind != EntryKind::File || !is_supported_video(&entry.path) {
            return;
        }

        let key = VideoPreviewKey::new(entry.path.clone());
        if self.video_cache.contains_key(&key) {
            return;
        }

        let preview = build_video_preview(&entry.path);
        self.video_cache.insert(key.clone(), preview);
        self.video_cache_order.push_back(key);
        while self.video_cache_order.len() > 24 {
            if let Some(oldest) = self.video_cache_order.pop_front() {
                self.video_cache.remove(&oldest);
            }
        }
    }

    pub fn cached_video_preview(&self, path: &Path) -> Option<&VideoPreview> {
        let key = VideoPreviewKey::new(path.to_path_buf());
        self.video_cache.get(&key)
    }

    fn directory_preview_entries(&self, path: &Path) -> Vec<FileEntry> {
        read_directory(path, self.show_hidden).unwrap_or_default()
    }

    pub fn poll_image_previews(&mut self) {
        loop {
            match self.image_jobs_rx.try_recv() {
                Ok(preview) => {
                    let key = preview.key.clone();
                    self.image_pending.remove(&key);
                    self.image_cache.insert(key.clone(), preview);
                    self.image_cache_order.push_back(key.clone());
                    while self.image_cache_order.len() > 24 {
                        if let Some(oldest) = self.image_cache_order.pop_front() {
                            self.image_cache.remove(&oldest);
                            self.image_pending.remove(&oldest);
                        }
                    }
                }
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
    }

    pub fn update_image_preview(&mut self, path: &Path, area: Rect) {
        let key = ImagePreviewKey::new(path.to_path_buf(), area);
        if self.image_cache.contains_key(&key) || self.image_pending.contains(&key) {
            return;
        }

        if area.width == 0 || area.height == 0 {
            return;
        }

        if self
            .image_jobs_tx
            .send(ImageJob {
                path: path.to_path_buf(),
                area,
            })
            .is_ok()
        {
            self.image_pending.insert(key);
        }
    }

    pub fn cached_image_preview(&self, path: &Path, area: Rect) -> Option<&ImagePreview> {
        let key = ImagePreviewKey::new(path.to_path_buf(), area);
        self.image_cache.get(&key)
    }

    pub fn prefetch_visible_image_previews(&mut self, area: Rect) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let start = self.contents_selected.saturating_sub(1);
        let end = self
            .contents_selected
            .saturating_add(3)
            .min(self.entries.len());

        for index in start..end {
            if let Some(entry) = self.entries.get(index) {
                let path = entry.path.clone();
                if crate::preview::is_supported_image(&path) {
                    self.update_image_preview(&path, area);
                }
            }
        }
    }

    fn prefetch_last_image_area(&mut self) {
        if let Some(area) = self.last_image_area {
            self.prefetch_visible_image_previews(area);
        }
    }

    fn move_up(&mut self) {
        match self.active_panel {
            Panel::Preview => self.scroll_preview(-1),
            _ => self.move_selection(-1),
        }
    }

    fn move_down(&mut self) {
        match self.active_panel {
            Panel::Preview => self.scroll_preview(1),
            _ => self.move_selection(1),
        }
    }

    fn scroll_preview(&mut self, delta: isize) {
        let next = self.preview_scroll as isize + delta;
        self.preview_scroll = next.max(0) as u16;
    }

    fn handle_dialog_key(&mut self, key: KeyEvent) -> bool {
        let Some(mut dialog) = self.dialog.take() else {
            return false;
        };

        let mut keep_dialog = true;

        match &mut dialog {
            Dialog::Input { value, action, .. } => match key.code {
                KeyCode::Esc => {
                    self.set_status("Canceled.");
                    keep_dialog = false;
                }
                KeyCode::Enter => {
                    let value = value.trim().to_string();
                    if value.is_empty() {
                        self.set_status("Name cannot be empty.");
                    } else {
                        let action = action.clone();
                        match self.commit_input_action(action, value) {
                            Ok(()) => {
                                keep_dialog = false;
                            }
                            Err(err) => {
                                self.set_status(format!("Error: {err}"));
                            }
                        }
                    }
                }
                KeyCode::Backspace => {
                    value.pop();
                }
                KeyCode::Char(ch) => {
                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                    } else {
                        value.push(ch);
                    }
                }
                _ => {}
            },
            Dialog::ConfirmDelete { path, name } => match key.code {
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    self.set_status("Delete canceled.");
                    keep_dialog = false;
                }
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    let target = path.clone();
                    let label = name.clone();
                    keep_dialog = false;
                    if let Err(err) = self.delete_entry(target, label) {
                        self.set_status(format!("Error: {err}"));
                    }
                }
                _ => {}
            },
        }

        if keep_dialog {
            self.dialog = Some(dialog);
            self.refresh_preview();
        }

        true
    }

    fn handle_settings_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('s') => {
                self.settings_visible = false;
                self.set_status("Closed settings.");
            }
            KeyCode::Enter => {
                self.settings_visible = false;
                self.set_status("Saved settings.");
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.settings_selected = self.settings_selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.settings_selected =
                    (self.settings_selected + 1).min(Self::settings_count().saturating_sub(1));
            }
            KeyCode::Left | KeyCode::Char('h') => self.cycle_setting(-1),
            KeyCode::Right | KeyCode::Char('l') => self.cycle_setting(1),
            _ => {}
        }
    }

    fn toggle_settings(&mut self) {
        self.settings_visible = !self.settings_visible;
        if self.settings_visible {
            self.help_visible = false;
        }
        self.settings_selected = self
            .settings_selected
            .min(Self::settings_count().saturating_sub(1));
        let message = if self.settings_visible {
            "Settings opened. Use Left/Right to change."
        } else {
            "Settings closed."
        };
        self.set_status(message);
    }

    fn settings_count() -> usize {
        6
    }

    fn cycle_setting(&mut self, delta: isize) {
        match self.settings_selected {
            0 => {
                self.config.theme = if delta < 0 {
                    self.config.theme.prev()
                } else {
                    self.config.theme.next()
                };
                self.persist_config("Theme updated.");
            }
            1 => {
                self.config.accent_color = if delta < 0 {
                    self.config.accent_color.prev()
                } else {
                    self.config.accent_color.next()
                };
                self.persist_config("Accent color updated.");
            }
            2 => {
                self.config.border_style = if delta < 0 {
                    self.config.border_style.prev()
                } else {
                    self.config.border_style.next()
                };
                self.persist_config("Border style updated.");
            }
            3 => {
                self.config.panel_layout = if delta < 0 {
                    self.config.panel_layout.prev()
                } else {
                    self.config.panel_layout.next()
                };
                self.persist_config("Panel layout updated.");
            }
            4 => {
                self.config.show_footer_tips = !self.config.show_footer_tips;
                self.persist_config("Footer tips toggled.");
            }
            5 => {
                self.config.show_hidden = !self.config.show_hidden;
                self.show_hidden = self.config.show_hidden;
                match self.refresh_entries() {
                    Ok(()) => self.persist_config("Hidden files setting updated."),
                    Err(err) => self.set_status(format!("Error: {err}")),
                }
            }
            _ => {}
        }
    }

    fn persist_config(&mut self, message: &str) {
        match self.config.save() {
            Ok(()) => self.set_status(message),
            Err(err) => self.set_status(format!("Error: {err}")),
        }
    }

    pub fn place_preview_lines(&self) -> Vec<String> {
        if self.places.is_empty() {
            return vec![String::from("No places available.")];
        }

        let index = self
            .places_selected
            .min(self.places.len().saturating_sub(1));
        let place = &self.places[index];
        vec![
            format!("Type: Shortcut"),
            format!("Name: {}", place.name),
            format!("Path: {}", place.path.display()),
        ]
    }

    pub fn settings_rows(&self) -> Vec<String> {
        vec![
            format!("Theme: {}", self.config.theme.label()),
            format!("Accent color: {}", self.config.accent_color.label()),
            format!("Border style: {}", self.config.border_style.label()),
            format!("Panel layout: {}", self.config.panel_layout.label()),
            format!(
                "Footer tips: {}",
                if self.config.show_footer_tips {
                    "On"
                } else {
                    "Off"
                }
            ),
            format!(
                "Show hidden files: {}",
                if self.config.show_hidden { "On" } else { "Off" }
            ),
        ]
    }

    pub fn entry_preview_lines(&self) -> Vec<String> {
        let Some(entry) = self.current_selection() else {
            return vec![
                String::from("Type: Folder"),
                format!("Path: {}", self.current_dir.display()),
                String::from("Items: 0"),
                String::from("Contents:"),
            ];
        };

        if entry.kind == EntryKind::Directory {
            let children = self.directory_preview_entries(&entry.path);
            let mut lines = vec![
                String::from("Type: Folder"),
                format!("Path: {}", entry.path.display()),
                format!("Items: {}", children.len()),
                format!(
                    "Created at: {}",
                    entry
                        .created
                        .map(format_system_time)
                        .unwrap_or_else(|| String::from("Unknown"))
                ),
                format!(
                    "Modified: {}",
                    entry
                        .modified
                        .map(format_system_time)
                        .unwrap_or_else(|| String::from("Unknown"))
                ),
                String::from("Contents:"),
            ];

            if children.is_empty() {
                lines.push(String::from("(empty)"));
            } else {
                for child in children.iter().take(18) {
                    let marker = if child.kind == EntryKind::Directory {
                        format!("{}/", child.name)
                    } else {
                        child.name.clone()
                    };
                    let marker = if child.is_hidden {
                        format!(". {marker}")
                    } else {
                        marker
                    };
                    lines.push(marker);
                }

                if children.len() > 18 {
                    lines.push(format!("... and {} more", children.len() - 18));
                }
            }

            lines
        } else if is_supported_video(&entry.path) {
            if let Some(preview) = self.cached_video_preview(&entry.path) {
                let mut lines = preview.lines.clone();
                if let Some(error) = preview.error.as_ref() {
                    lines.push(String::new());
                    lines.push(format!("Note: {error}"));
                }
                lines.push(format!(
                    "Size: {}",
                    entry
                        .size
                        .map(human_size)
                        .unwrap_or_else(|| String::from("Unknown"))
                ));
                lines.push(format!(
                    "Created at: {}",
                    entry
                        .created
                        .map(format_system_time)
                        .unwrap_or_else(|| String::from("Unknown"))
                ));
                lines.push(format!(
                    "Modified: {}",
                    entry
                        .modified
                        .map(format_system_time)
                        .unwrap_or_else(|| String::from("Unknown"))
                ));
                lines.push(format!("Path: {}", entry.path.display()));
                lines
            } else {
                vec![
                    String::from("Type: Video"),
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
                        "Created at: {}",
                        entry
                            .created
                            .map(format_system_time)
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
        } else if is_supported_text(&entry.path) {
            vec![
                String::from("Type: Text file"),
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
                    "Created at: {}",
                    entry
                        .created
                        .map(format_system_time)
                        .unwrap_or_else(|| String::from("Unknown"))
                ),
                format!(
                    "Modified: {}",
                    entry
                        .modified
                        .map(format_system_time)
                        .unwrap_or_else(|| String::from("Unknown"))
                ),
                String::from("Preview: first 400 lines"),
                format!("Path: {}", entry.path.display()),
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
                    "Created at: {}",
                    entry
                        .created
                        .map(format_system_time)
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
                open_with_system(&entry.path)
                    .with_context(|| format!("Could not open {}", entry.path.display()))?;
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
        self.contents_mode = ContentsMode::Directory {
            path: self.current_dir.clone(),
        };
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
        self.config.show_hidden = self.show_hidden;
        match self.refresh_entries() {
            Ok(()) => match self.config.save() {
                Ok(()) => {
                    let state = if self.show_hidden { "shown" } else { "hidden" };
                    self.set_status(format!("Hidden files are now {state}."));
                }
                Err(err) => self.set_status(format!("Error: {err}")),
            },
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
                self.refresh_hovered_place_entries();
                self.refresh_preview();
                self.prefetch_last_image_area();
            }
            Panel::Contents | Panel::Preview => {
                if self.entries.is_empty() {
                    return;
                }
                self.contents_selected =
                    move_index(self.contents_selected, delta, self.entries.len());
                self.refresh_preview();
                self.prefetch_last_image_area();
            }
        }
    }

    fn next_panel(&mut self) {
        self.active_panel = match self.active_panel {
            Panel::Places => Panel::Contents,
            Panel::Contents => Panel::Preview,
            Panel::Preview => Panel::Places,
        };
        self.refresh_hovered_place_entries();
        self.refresh_preview();
        self.prefetch_last_image_area();
    }

    fn previous_panel(&mut self) {
        self.active_panel = match self.active_panel {
            Panel::Places => Panel::Preview,
            Panel::Contents => Panel::Places,
            Panel::Preview => Panel::Contents,
        };
        self.refresh_hovered_place_entries();
        self.refresh_preview();
        self.prefetch_last_image_area();
    }

    fn current_selection(&self) -> Option<&FileEntry> {
        self.entries.get(self.contents_selected)
    }

    fn current_path(&self) -> &Path {
        if self.active_panel == Panel::Places {
            self.places
                .get(self.places_selected)
                .map(|place| place.path.as_path())
                .unwrap_or(self.current_dir.as_path())
        } else {
            self.current_selection()
                .map(|entry| entry.path.as_path())
                .unwrap_or(self.current_dir.as_path())
        }
    }

    fn selected_entry(&self) -> Option<&FileEntry> {
        if matches!(self.active_panel, Panel::Contents | Panel::Preview) {
            self.current_selection()
        } else {
            None
        }
    }

    fn begin_new_file(&mut self) {
        self.dialog = Some(Dialog::Input {
            title: String::from("New file"),
            value: String::from("new_file.txt"),
            action: InputAction::CreateFile {
                dir: self.current_dir.clone(),
            },
        });
        self.set_status("Type a file name, then press Enter.");
    }

    fn begin_new_folder(&mut self) {
        self.dialog = Some(Dialog::Input {
            title: String::from("New folder"),
            value: String::from("New Folder"),
            action: InputAction::CreateFolder {
                dir: self.current_dir.clone(),
            },
        });
        self.set_status("Type a folder name, then press Enter.");
    }

    fn begin_search_current(&mut self) {
        self.dialog = Some(Dialog::Input {
            title: String::from("Search current folder"),
            value: String::new(),
            action: InputAction::SearchCurrent {
                base: self.current_dir.clone(),
            },
        });
        self.set_status("Type a search term, then press Enter.");
    }

    fn begin_search_recursive(&mut self) {
        self.dialog = Some(Dialog::Input {
            title: String::from("Recursive search"),
            value: String::new(),
            action: InputAction::SearchRecursive {
                base: self.current_dir.clone(),
            },
        });
        self.set_status("Type a search term, then press Enter.");
    }

    fn begin_rename(&mut self) {
        let Some(entry) = self.selected_entry().cloned() else {
            self.set_status("Select a file or folder in Contents first.");
            return;
        };

        self.dialog = Some(Dialog::Input {
            title: String::from("Rename"),
            value: entry.name.clone(),
            action: InputAction::Rename { source: entry.path },
        });
        self.set_status("Type a new name, then press Enter.");
    }

    fn begin_delete(&mut self) {
        let Some(entry) = self.selected_entry().cloned() else {
            self.set_status("Select a file or folder in Contents first.");
            return;
        };

        self.dialog = Some(Dialog::ConfirmDelete {
            path: entry.path.clone(),
            name: entry.name.clone(),
        });
        self.set_status("Confirm delete with Y, cancel with N.");
    }

    fn mark_copy(&mut self) {
        let Some(entry) = self.selected_entry().cloned() else {
            self.set_status("Select a file or folder in Contents first.");
            return;
        };

        self.pending_operation = Some(PendingOperation {
            mode: ClipboardMode::Copy,
            source: entry.path.clone(),
            name: entry.name.clone(),
        });
        self.refresh_preview();
        self.set_status(format!("Marked {} for copy.", entry.name));
    }

    fn mark_cut(&mut self) {
        let Some(entry) = self.selected_entry().cloned() else {
            self.set_status("Select a file or folder in Contents first.");
            return;
        };

        self.pending_operation = Some(PendingOperation {
            mode: ClipboardMode::Cut,
            source: entry.path.clone(),
            name: entry.name.clone(),
        });
        self.refresh_preview();
        self.set_status(format!("Marked {} for move.", entry.name));
    }

    fn paste_pending_operation(&mut self) {
        let Some(operation) = self.pending_operation.clone() else {
            self.set_status("Nothing to paste.");
            return;
        };

        match self.perform_paste(operation) {
            Ok(()) => {}
            Err(err) => self.set_status(format!("Error: {err}")),
        }
    }

    fn refresh_current(&mut self) {
        match self.refresh_entries() {
            Ok(()) => self.set_status("Refreshed current folder."),
            Err(err) => self.set_status(format!("Error: {err}")),
        }
    }

    fn copy_current_path(&mut self) {
        let path = self.current_path().display().to_string();
        match Clipboard::new().and_then(|mut clipboard| clipboard.set_text(path.clone())) {
            Ok(()) => self.set_status(format!("Copied path: {path}")),
            Err(err) => self.set_status(format!("Error: {err}")),
        }
    }

    fn with_clipboard_info(&self, mut lines: Vec<String>) -> Vec<String> {
        match &self.contents_mode {
            ContentsMode::Directory { path } => {
                lines.push(String::new());
                lines.push(format!("Folder: {}", path.display()));
            }
            ContentsMode::SearchCurrent { base, query } => {
                lines.push(String::new());
                lines.push(format!("Search: current folder"));
                lines.push(format!("Base: {}", base.display()));
                lines.push(format!("Query: {}", query));
            }
            ContentsMode::SearchRecursive { base, query } => {
                lines.push(String::new());
                lines.push(format!("Search: recursive"));
                lines.push(format!("Base: {}", base.display()));
                lines.push(format!("Query: {}", query));
            }
        }

        if let Some(operation) = self.pending_operation.as_ref() {
            lines.push(String::new());
            let mode = match operation.mode {
                ClipboardMode::Copy => "Copy",
                ClipboardMode::Cut => "Cut",
            };
            lines.push(format!("Clipboard: {mode} {}", operation.name));
            lines.push(format!("Source: {}", operation.source.display()));
            lines.push(String::from("Press p to paste here."));
        }

        lines
    }

    fn commit_input_action(&mut self, action: InputAction, raw_name: String) -> Result<()> {
        let name = raw_name.trim();
        if name.is_empty() {
            return Err(anyhow!("Name cannot be empty"));
        }

        let target = match action {
            InputAction::CreateFile { dir } => {
                let path = dir.join(name);
                if path.exists() {
                    return Err(anyhow!("A file with this name already exists"));
                }
                File::create(&path)
                    .with_context(|| format!("Unable to create {}", path.display()))?;
                path
            }
            InputAction::CreateFolder { dir } => {
                let path = dir.join(name);
                if path.exists() {
                    return Err(anyhow!("A folder with this name already exists"));
                }
                fs::create_dir(&path)
                    .with_context(|| format!("Unable to create {}", path.display()))?;
                path
            }
            InputAction::Rename { source } => {
                let parent = source
                    .parent()
                    .ok_or_else(|| anyhow!("Cannot rename this item"))?;
                let path = parent.join(name);
                if path.exists() {
                    return Err(anyhow!("A file with this name already exists"));
                }
                fs::rename(&source, &path).with_context(|| {
                    format!(
                        "Unable to rename {} to {}",
                        source.display(),
                        path.display()
                    )
                })?;
                path
            }
            InputAction::SearchCurrent { base } => {
                self.contents_mode = ContentsMode::SearchCurrent {
                    base: base.clone(),
                    query: name.to_string(),
                };
                self.contents_selected = 0;
                self.apply_contents_mode()?;
                self.set_status(format!("Filtered current folder for \"{}\".", name));
                self.dialog = None;
                return Ok(());
            }
            InputAction::SearchRecursive { base } => {
                self.contents_mode = ContentsMode::SearchRecursive {
                    base: base.clone(),
                    query: name.to_string(),
                };
                self.contents_selected = 0;
                self.apply_contents_mode()?;
                self.set_status(format!("Search results for \"{}\".", name));
                self.dialog = None;
                return Ok(());
            }
        };

        self.dialog = None;
        self.refresh_entries()?;
        self.select_entry_by_path(&target);
        self.set_status(format!("Updated {}", target.display()));
        Ok(())
    }

    fn perform_paste(&mut self, operation: PendingOperation) -> Result<()> {
        let source_name = operation
            .source
            .file_name()
            .ok_or_else(|| anyhow!("Source path has no file name"))?
            .to_owned();
        let destination = self.current_dir.join(&source_name);

        if same_path(&operation.source, &destination) {
            return Err(anyhow!("Source and destination are the same"));
        }

        if destination.exists() {
            return Err(anyhow!("Destination already exists"));
        }

        match operation.mode {
            ClipboardMode::Copy => copy_path_recursive(&operation.source, &destination)?,
            ClipboardMode::Cut => move_path_recursive(&operation.source, &destination)?,
        }

        if operation.mode == ClipboardMode::Cut {
            self.pending_operation = None;
        }
        self.refresh_entries()?;
        self.select_entry_by_path(&destination);
        if operation.mode == ClipboardMode::Cut {
            self.set_status(format!("Moved {}.", source_name.to_string_lossy()));
        } else {
            self.set_status(format!("Copied {}.", source_name.to_string_lossy()));
        }

        Ok(())
    }

    fn delete_entry(&mut self, path: PathBuf, name: String) -> Result<()> {
        delete(&path).with_context(|| format!("Unable to move {} to trash", path.display()))?;
        self.refresh_entries()?;
        self.set_status(format!("Moved {name} to Recycle Bin."));
        Ok(())
    }

    fn select_entry_by_path(&mut self, path: &Path) {
        if let Some((index, _)) = self
            .entries
            .iter()
            .enumerate()
            .find(|(_, entry)| same_path(&entry.path, path))
        {
            self.contents_selected = index;
        }
    }

    fn refresh_entries(&mut self) -> Result<()> {
        self.directory_entries = read_directory(&self.current_dir, self.show_hidden)?;
        let result = self.apply_contents_mode();
        if result.is_ok() {
            self.prefetch_last_image_area();
        }
        result
    }

    fn apply_contents_mode(&mut self) -> Result<()> {
        self.entries = match &self.contents_mode {
            ContentsMode::Directory { .. } => self.directory_entries.clone(),
            ContentsMode::SearchCurrent { query, .. } => {
                filter_entries(&self.directory_entries, query)
            }
            ContentsMode::SearchRecursive { base, query } => {
                search_recursive(base, query, self.show_hidden)?
            }
        };

        self.contents_selected = self
            .contents_selected
            .min(self.entries.len().saturating_sub(1));
        self.refresh_hovered_place_entries();
        self.refresh_preview();
        self.prefetch_last_image_area();
        Ok(())
    }

    fn refresh_hovered_place_entries(&mut self) {
        if self.active_panel != Panel::Places {
            self.hovered_place_entries = self.directory_entries.clone();
            self.hovered_place_error = None;
            return;
        }

        let Some(place) = self.places.get(self.places_selected).cloned() else {
            self.hovered_place_entries.clear();
            self.hovered_place_error = None;
            return;
        };

        match read_directory(&place.path, self.show_hidden) {
            Ok(entries) => {
                self.hovered_place_entries = entries;
                self.hovered_place_error = None;
            }
            Err(err) => {
                self.hovered_place_entries.clear();
                self.hovered_place_error = Some(format!(
                    "Unable to read {}: {err}",
                    place.path.display()
                ));
            }
        }
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

fn copy_path_recursive(source: &Path, destination: &Path) -> Result<()> {
    let metadata = fs::metadata(source)
        .with_context(|| format!("Unable to read metadata for {}", source.display()))?;

    if metadata.is_dir() {
        fs::create_dir(destination)
            .with_context(|| format!("Unable to create {}", destination.display()))?;
        for entry in
            fs::read_dir(source).with_context(|| format!("Unable to read {}", source.display()))?
        {
            let entry = entry?;
            let child_source = entry.path();
            let child_destination = destination.join(entry.file_name());
            copy_path_recursive(&child_source, &child_destination)?;
        }
    } else {
        fs::copy(source, destination).with_context(|| {
            format!(
                "Unable to copy {} to {}",
                source.display(),
                destination.display()
            )
        })?;
    }

    Ok(())
}

fn move_path_recursive(source: &Path, destination: &Path) -> Result<()> {
    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(_) => {
            copy_path_recursive(source, destination)?;
            remove_path_recursive(source)?;
            Ok(())
        }
    }
}

fn remove_path_recursive(path: &Path) -> Result<()> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("Unable to read metadata for {}", path.display()))?;

    if metadata.is_dir() {
        fs::remove_dir_all(path).with_context(|| format!("Unable to remove {}", path.display()))?;
    } else {
        fs::remove_file(path).with_context(|| format!("Unable to remove {}", path.display()))?;
    }

    Ok(())
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
        add_if_exists(
            &mut places,
            "Desktop",
            user_dirs.desktop_dir().map(Path::to_path_buf),
        );
        add_if_exists(
            &mut places,
            "Downloads",
            user_dirs.download_dir().map(Path::to_path_buf),
        );
        add_if_exists(
            &mut places,
            "Documents",
            user_dirs.document_dir().map(Path::to_path_buf),
        );
        add_if_exists(
            &mut places,
            "Pictures",
            user_dirs.picture_dir().map(Path::to_path_buf),
        );
        add_if_exists(
            &mut places,
            "Videos",
            user_dirs.video_dir().map(Path::to_path_buf),
        );
        add_if_exists(
            &mut places,
            "Music",
            user_dirs.audio_dir().map(Path::to_path_buf),
        );
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
    let read_dir =
        fs::read_dir(dir).with_context(|| format!("Unable to read {}", dir.display()))?;

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
            created: metadata.as_ref().and_then(|meta| meta.created().ok()),
            modified: metadata.as_ref().and_then(|meta| meta.modified().ok()),
            extension,
            is_hidden,
        });
    }

    entries.sort_by(compare_entries);
    Ok(entries)
}

fn filter_entries(entries: &[FileEntry], query: &str) -> Vec<FileEntry> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return entries.to_vec();
    }

    let mut filtered = entries
        .iter()
        .filter(|entry| {
            entry.name.to_lowercase().contains(&needle)
                || entry
                    .path
                    .to_string_lossy()
                    .to_lowercase()
                    .contains(&needle)
        })
        .cloned()
        .collect::<Vec<_>>();
    filtered.sort_by(compare_entries);
    filtered
}

fn search_recursive(base: &Path, query: &str, show_hidden: bool) -> Result<Vec<FileEntry>> {
    let needle = query.trim().to_lowercase();
    let mut results = Vec::new();
    let walker = ignore::WalkBuilder::new(base)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .ignore(true)
        .parents(true)
        .build();

    for result in walker {
        let entry = result?;
        let path = entry.path();

        if path == base {
            continue;
        }

        let file_name = entry.file_name().to_string_lossy().to_string();
        if !show_hidden && is_hidden(path, &file_name, None) {
            continue;
        }

        if is_ignored_search_dir(&file_name) {
            continue;
        }

        let metadata = entry.metadata().ok();
        let file_type = entry.file_type();
        let kind = if file_type.map(|ft| ft.is_dir()).unwrap_or(false) {
            EntryKind::Directory
        } else if file_type.map(|ft| ft.is_symlink()).unwrap_or(false) {
            EntryKind::Symlink
        } else if file_type.map(|ft| ft.is_file()).unwrap_or(false) {
            EntryKind::File
        } else {
            EntryKind::Unknown
        };

        let relative = path.strip_prefix(base).unwrap_or(path);
        let relative_name = relative.to_string_lossy().to_string();
        if !needle.is_empty()
            && !relative_name.to_lowercase().contains(&needle)
            && !file_name.to_lowercase().contains(&needle)
        {
            continue;
        }

        results.push(FileEntry {
            name: relative_name,
            path: path.to_path_buf(),
            kind,
            size: metadata.as_ref().map(|meta| meta.len()),
            created: metadata.as_ref().and_then(|meta| meta.created().ok()),
            modified: metadata.as_ref().and_then(|meta| meta.modified().ok()),
            extension: path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| format!(".{ext}")),
            is_hidden: is_hidden(path, &file_name, metadata.as_ref()),
        });
    }

    results.sort_by(compare_entries);
    Ok(results)
}

fn is_ignored_search_dir(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        ".git" | "node_modules" | "target" | "dist" | "build" | ".cache"
    )
}

fn compare_entries(left: &FileEntry, right: &FileEntry) -> Ordering {
    match (
        left.kind == EntryKind::Directory,
        right.kind == EntryKind::Directory,
    ) {
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
        String::from("s          Open settings"),
        String::from("Tab        Next panel"),
        String::from("Shift+Tab  Previous panel"),
        String::from("Up/Down    Move selection or scroll preview"),
        String::from("j/k        Move selection or scroll preview"),
        String::from("Enter      Open selected item"),
        String::from("Backspace  Parent folder"),
        String::from("Left       Parent folder"),
        String::from("h          Toggle hidden files"),
        String::from("c          Mark selected item for copy"),
        String::from("x          Mark selected item for move"),
        String::from("p          Paste into current folder"),
        String::from("y          Copy selected path"),
        String::from("F5         Refresh folder"),
        String::from("/          Search current folder"),
        String::from("Ctrl+f     Recursive search"),
    ]
}

fn spawn_image_worker(picker: Picker) -> (Sender<ImageJob>, Receiver<ImagePreview>) {
    let (job_tx, job_rx) = mpsc::channel::<ImageJob>();
    let (preview_tx, preview_rx) = mpsc::channel::<ImagePreview>();

    thread::spawn(move || {
        while let Ok(job) = job_rx.recv() {
            let preview = build_image_preview(&picker, &job.path, job.area);
            if preview_tx.send(preview).is_err() {
                break;
            }
        }
    });

    (job_tx, preview_rx)
}
