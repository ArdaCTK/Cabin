use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet, VecDeque},
    fs::{self, File},
    path::{Component, Path, PathBuf},
    process::Command,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    time::{Duration, Instant, SystemTime},
};

use anyhow::{anyhow, Context, Result};
use arboard::Clipboard;
use chrono::{DateTime, Local};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use directories_next::{BaseDirs, UserDirs};
use ratatui::layout::Rect;
use ratatui_image::picker::Picker;
use trash::delete;

use crate::config::{CabinConfig, SortField, ThemePreset};
use crate::preview::{
    build_audio_preview, build_image_preview, build_video_preview,
    build_pdf_preview, is_supported_audio, is_supported_image,
    is_supported_pdf, is_supported_text, is_supported_video,
    spawn_image_workers, spawn_text_worker,
    AudioPreview, AudioPreviewKey, ImagePreview, ImagePreviewKey,
    TextPreview, TextPreviewKey, VideoPreview, VideoPreviewKey,
    ImageJob,
};
use crate::system::SystemMonitor;

// ---------------------------------------------------------------------------
// Core enums
// ---------------------------------------------------------------------------

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardMode {
    Copy,
    Cut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictChoice {
    KeepBoth,
    Replace,
    Cancel,
}

impl ConflictChoice {
    const ALL: [Self; 3] = [Self::KeepBoth, Self::Replace, Self::Cancel];

    pub fn label(self) -> &'static str {
        match self {
            Self::KeepBoth => "Keep both",
            Self::Replace => "Replace",
            Self::Cancel => "Cancel",
        }
    }

    fn next(self) -> Self {
        let i = Self::ALL.iter().position(|x| *x == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    fn prev(self) -> Self {
        let i = Self::ALL.iter().position(|x| *x == self).unwrap_or(0);
        Self::ALL[(i + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorTarget {
    Accent,
    Foreground,
    Background,
    Muted,
}

// ---------------------------------------------------------------------------
// Data structs
// ---------------------------------------------------------------------------

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
    SetColor { target: ColorTarget },
    SetStartDir,
    JumpToPath,
    AddBookmark,
}

// ---------------------------------------------------------------------------
// Settings fields
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingField {
    Theme,
    AccentColor,
    ForegroundColor,
    BackgroundColor,
    MutedColor,
    BorderStyle,
    PanelLayout,
    SortField,
    StartDir,
    RememberLastFolder,
    FooterTips,
    ShowHidden,
}

impl SettingField {
    const ALL: [Self; 12] = [
        Self::Theme,
        Self::AccentColor,
        Self::ForegroundColor,
        Self::BackgroundColor,
        Self::MutedColor,
        Self::BorderStyle,
        Self::PanelLayout,
        Self::SortField,
        Self::StartDir,
        Self::RememberLastFolder,
        Self::FooterTips,
        Self::ShowHidden,
    ];

    fn from_index(i: usize) -> Self {
        Self::ALL[i.min(Self::ALL.len().saturating_sub(1))]
    }
}

// ---------------------------------------------------------------------------
// Dialogs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Dialog {
    Input {
        title: String,
        value: String,
        action: InputAction,
    },
    ConfirmDelete {
        paths: Vec<PathBuf>,
        label: String,
    },
    Conflict {
        destination: PathBuf,
        choice: ConflictChoice,
    },
}

// ---------------------------------------------------------------------------
// App struct
// ---------------------------------------------------------------------------

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

    // Preview caches
    pub text_cache: HashMap<TextPreviewKey, TextPreview>,
    pub text_cache_order: VecDeque<TextPreviewKey>,
    pub video_cache: HashMap<VideoPreviewKey, VideoPreview>,
    pub video_cache_order: VecDeque<VideoPreviewKey>,
    pub audio_cache: HashMap<AudioPreviewKey, AudioPreview>,
    pub audio_cache_order: VecDeque<AudioPreviewKey>,
    pub image_cache: HashMap<ImagePreviewKey, ImagePreview>,
    pub image_cache_order: VecDeque<ImagePreviewKey>,

    // Background workers
    image_jobs_tx: Sender<ImageJob>,
    image_jobs_rx: Receiver<ImagePreview>,
    image_pending: HashSet<ImagePreviewKey>,
    text_jobs_tx: Sender<PathBuf>,
    text_results_rx: Receiver<(PathBuf, TextPreview)>,
    text_pending: HashSet<PathBuf>,

    // Debounce: dispatch image/text jobs only after the cursor has been still
    // for at least DEBOUNCE_MS milliseconds.
    last_move_time: Instant,
    debounce_fired: bool,

    pub last_image_area: Option<Rect>,
    pub preview_scroll: u16,
    pub show_hidden: bool,
    pub status_message: Option<String>,
    pub help_visible: bool,
    pub settings_visible: bool,
    pub settings_selected: usize,
    pub search_restore_mode: Option<(ContentsMode, usize)>,
    pub dialog: Option<Dialog>,
    pub pending_operation: Option<PendingOperation>,
    pub hovered_place_entries: Vec<FileEntry>,
    pub hovered_place_error: Option<String>,
    pub system_monitor: SystemMonitor,

    // Persistent clipboard instance — fixes X11/Wayland where data is lost
    // when the Clipboard object is dropped between calls.
    clipboard: Option<Clipboard>,

    // Multi-select: indices into `entries` that are currently selected.
    pub selected_indices: HashSet<usize>,
}

const DEBOUNCE_MS: u64 = 80;
const TEXT_CACHE_LIMIT: usize = 32;
const IMAGE_CACHE_LIMIT: usize = 24;
const AUDIO_CACHE_LIMIT: usize = 24;
const VIDEO_CACHE_LIMIT: usize = 24;

impl App {
    pub fn new() -> Result<Self> {
        let (config, warning, should_init_config) = CabinConfig::load_or_default();
        let mut startup_msg =
            warning.unwrap_or_else(|| String::from("Cabin is ready."));
        if should_init_config {
            if let Err(e) = config.save() {
                startup_msg =
                    format!("Cabin is ready, but config.toml could not be created: {e}");
            }
        }

        let show_hidden = config.show_hidden;
        let current_dir = config.startup_dir();
        let places = build_places(&config);

        let picker = Picker::from_query_stdio()
            .unwrap_or_else(|_| Picker::from_fontsize((10, 20)));
        let (image_jobs_tx, image_jobs_rx) = spawn_image_workers(picker);
        let (text_jobs_tx, text_results_rx) = spawn_text_worker();

        let clipboard = Clipboard::new().ok();

        let mut app = Self {
            should_quit: false,
            active_panel: Panel::Contents,
            config,
            current_dir,
            places,
            directory_entries: Vec::new(),
            entries: Vec::new(),
            contents_mode: ContentsMode::Directory { path: PathBuf::new() },
            places_selected: 0,
            contents_selected: 0,
            preview: PreviewData { lines: Vec::new() },
            text_cache: HashMap::new(),
            text_cache_order: VecDeque::new(),
            video_cache: HashMap::new(),
            video_cache_order: VecDeque::new(),
            audio_cache: HashMap::new(),
            audio_cache_order: VecDeque::new(),
            image_cache: HashMap::new(),
            image_cache_order: VecDeque::new(),
            image_jobs_tx,
            image_jobs_rx,
            image_pending: HashSet::new(),
            text_jobs_tx,
            text_results_rx,
            text_pending: HashSet::new(),
            last_move_time: Instant::now() - Duration::from_secs(10),
            debounce_fired: true,
            last_image_area: None,
            preview_scroll: 0,
            show_hidden,
            status_message: Some(startup_msg),
            help_visible: false,
            settings_visible: false,
            settings_selected: 0,
            search_restore_mode: None,
            dialog: None,
            pending_operation: None,
            hovered_place_entries: Vec::new(),
            hovered_place_error: None,
            system_monitor: SystemMonitor::new(),
            clipboard,
            selected_indices: HashSet::new(),
        };

        app.sync_places_selection();
        app.contents_mode = ContentsMode::Directory { path: app.current_dir.clone() };
        app.refresh_entries()?;
        app.refresh_hovered_place_entries();
        Ok(app)
    }

    // -----------------------------------------------------------------------
    // Debounce tick — called every 50 ms from the event loop
    // -----------------------------------------------------------------------

    /// Fires image/text prefetch jobs once the cursor has been still for
    /// `DEBOUNCE_MS` milliseconds.  This prevents flooding the worker queues
    /// when the user scrolls quickly.
    pub fn tick_debounce(&mut self) {
        if !self.debounce_fired
            && self.last_move_time.elapsed() >= Duration::from_millis(DEBOUNCE_MS)
        {
            self.debounce_fired = true;
            self.dispatch_preview_jobs();
        }
    }

    /// Dispatches background jobs for the currently selected entry.
    fn dispatch_preview_jobs(&mut self) {
        self.schedule_text_preview();
        if let Some(area) = self.last_image_area {
            self.prefetch_visible_image_previews(area);
        }
    }

    // -----------------------------------------------------------------------
    // Input handling
    // -----------------------------------------------------------------------

    pub fn handle_key(&mut self, key: KeyEvent) {
        if self.handle_dialog_key(key) {
            return;
        }
        if self.settings_visible {
            self.handle_settings_key(key);
            return;
        }
        if self.help_visible {
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') => self.help_visible = false,
                _ => {}
            }
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
            // In search mode Backspace should NOT navigate to the parent dir.
            KeyCode::Backspace | KeyCode::Left => {
                if self.is_in_search_mode() {
                    self.exit_search_mode();
                } else {
                    self.go_parent();
                }
            }
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
            // Sort cycling
            KeyCode::Char('S') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.cycle_sort()
            }
            // Multi-select with Space
            KeyCode::Char(' ') => self.toggle_selection(),
            // Jump to path
            KeyCode::Char('g') => self.begin_jump_to_path(),
            // Add bookmark for current dir
            KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.begin_add_bookmark()
            }
            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // Search-mode helpers
    // -----------------------------------------------------------------------

    fn is_in_search_mode(&self) -> bool {
        matches!(
            &self.contents_mode,
            ContentsMode::SearchCurrent { .. } | ContentsMode::SearchRecursive { .. }
        )
    }

    fn exit_search_mode(&mut self) {
        if let Err(e) = self.restore_search_mode() {
            self.set_status(format!("Error: {e}"));
        } else {
            self.set_status("Search exited.");
        }
    }

    // -----------------------------------------------------------------------
    // Sort
    // -----------------------------------------------------------------------

    fn cycle_sort(&mut self) {
        self.config.sort_field = self.config.sort_field.next();
        match self.refresh_entries() {
            Ok(()) => {
                let _ = self.config.save();
                self.set_status(format!("Sorted by: {}", self.config.sort_field.label()));
            }
            Err(e) => self.set_status(format!("Error: {e}")),
        }
    }

    // -----------------------------------------------------------------------
    // Multi-select
    // -----------------------------------------------------------------------

    fn toggle_selection(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let i = self.contents_selected;
        if self.selected_indices.contains(&i) {
            self.selected_indices.remove(&i);
        } else {
            self.selected_indices.insert(i);
        }
        self.refresh_preview();
    }

    /// Returns the paths that are currently selected.  If nothing is explicitly
    /// selected, falls back to the cursor entry (single-item operations).
    fn effective_selection_paths(&self) -> Vec<PathBuf> {
        if !self.selected_indices.is_empty() {
            self.selected_indices
                .iter()
                .filter_map(|&i| self.entries.get(i).map(|e| e.path.clone()))
                .collect()
        } else if let Some(e) = self.current_selection() {
            vec![e.path.clone()]
        } else {
            vec![]
        }
    }

    // -----------------------------------------------------------------------
    // Bookmarks
    // -----------------------------------------------------------------------

    fn begin_add_bookmark(&mut self) {
        let name = self
            .current_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Bookmark")
            .to_string();
        self.dialog = Some(Dialog::Input {
            title: String::from("Add bookmark — enter a name"),
            value: name,
            action: InputAction::AddBookmark,
        });
        self.set_status("Type a bookmark name, then press Enter.");
    }

    fn add_bookmark(&mut self, name: String) {
        let path = self.current_dir.display().to_string();
        // Avoid duplicates by path.
        if !self.config.bookmarks.iter().any(|(_, p)| p == &path) {
            self.config.bookmarks.push((name.clone(), path));
            self.places = build_places(&self.config);
            self.persist_config(&format!("Bookmark '{name}' added."));
        } else {
            self.set_status("This directory is already bookmarked.");
        }
    }

    // -----------------------------------------------------------------------
    // Jump to path
    // -----------------------------------------------------------------------

    fn begin_jump_to_path(&mut self) {
        self.dialog = Some(Dialog::Input {
            title: String::from("Jump to path"),
            value: self.current_dir.display().to_string(),
            action: InputAction::JumpToPath,
        });
        self.set_status("Type a full path and press Enter.");
    }

    // -----------------------------------------------------------------------
    // Preview lines
    // -----------------------------------------------------------------------

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
        self.append_status_lines(lines)
    }

    pub fn dialog_preview_lines(&self) -> Option<Vec<String>> {
        match self.dialog.as_ref() {
            Some(Dialog::Input { title, value, .. }) => Some(vec![
                title.clone(),
                String::new(),
                format!("Value: {value}"),
                String::new(),
                String::from("Enter: confirm"),
                String::from("Esc: cancel"),
                String::from("Backspace: delete last character"),
            ]),
            Some(Dialog::ConfirmDelete { label, .. }) => Some(vec![
                String::from("Delete confirmation"),
                String::new(),
                format!("Move \"{label}\" to Recycle Bin?"),
                String::new(),
                String::from("Y / Enter: yes"),
                String::from("N / Esc: no"),
            ]),
            Some(Dialog::Conflict { destination, choice }) => Some(vec![
                String::from("Paste conflict"),
                String::new(),
                format!("Destination exists: {}", destination.display()),
                String::new(),
                format!("Choice: {}", choice.label()),
                String::new(),
                String::from("Left/Right: change"),
                String::from("Enter: confirm"),
                String::from("Esc: cancel"),
            ]),
            None => None,
        }
    }

    fn refresh_preview(&mut self) {
        self.refresh_video_preview();
        self.refresh_audio_preview();
        self.preview = PreviewData { lines: self.active_preview_lines() };
        self.preview_scroll = 0;
        // Text jobs are dispatched by the debounce tick, NOT here, to avoid
        // kicking off a disk read on every cursor movement.
    }

    // -----------------------------------------------------------------------
    // Text preview scheduling (background)
    // -----------------------------------------------------------------------

    fn schedule_text_preview(&mut self) {
        let Some(entry) = self.current_selection().cloned() else { return };
        if entry.kind != EntryKind::File {
            return;
        }
        if !is_supported_text(&entry.path) && !is_supported_pdf(&entry.path) {
            return;
        }

        let key = TextPreviewKey::new(entry.path.clone());
        if self.text_cache.contains_key(&key) || self.text_pending.contains(&entry.path) {
            return;
        }

        if self.text_jobs_tx.send(entry.path.clone()).is_ok() {
            self.text_pending.insert(entry.path);
        }
    }

    pub fn poll_text_previews(&mut self) {
        loop {
            match self.text_results_rx.try_recv() {
                Ok((path, preview)) => {
                    self.text_pending.remove(&path);
                    let key = TextPreviewKey::new(path);
                    self.text_cache.insert(key.clone(), preview);
                    self.text_cache_order.push_back(key);
                    while self.text_cache_order.len() > TEXT_CACHE_LIMIT {
                        if let Some(oldest) = self.text_cache_order.pop_front() {
                            self.text_cache.remove(&oldest);
                        }
                    }
                }
                Err(_) => break,
            }
        }
    }

    pub fn cached_text_preview(&self, path: &Path) -> Option<&TextPreview> {
        self.text_cache.get(&TextPreviewKey::new(path.to_path_buf()))
    }

    // -----------------------------------------------------------------------
    // Video / Audio preview (still synchronous — fast metadata reads)
    // -----------------------------------------------------------------------

    fn refresh_video_preview(&mut self) {
        let Some(entry) = self.current_selection().cloned() else { return };
        if entry.kind != EntryKind::File || !is_supported_video(&entry.path) { return; }
        let key = VideoPreviewKey::new(entry.path.clone());
        if self.video_cache.contains_key(&key) { return; }
        let preview = build_video_preview(&entry.path);
        self.video_cache.insert(key.clone(), preview);
        self.video_cache_order.push_back(key);
        while self.video_cache_order.len() > VIDEO_CACHE_LIMIT {
            if let Some(oldest) = self.video_cache_order.pop_front() {
                self.video_cache.remove(&oldest);
            }
        }
    }

    pub fn cached_video_preview(&self, path: &Path) -> Option<&VideoPreview> {
        self.video_cache.get(&VideoPreviewKey::new(path.to_path_buf()))
    }

    fn refresh_audio_preview(&mut self) {
        let Some(entry) = self.current_selection().cloned() else { return };
        if entry.kind != EntryKind::File || !is_supported_audio(&entry.path) { return; }
        let key = AudioPreviewKey::new(entry.path.clone());
        if self.audio_cache.contains_key(&key) { return; }
        let preview = build_audio_preview(&entry.path);
        self.audio_cache.insert(key.clone(), preview);
        self.audio_cache_order.push_back(key);
        while self.audio_cache_order.len() > AUDIO_CACHE_LIMIT {
            if let Some(oldest) = self.audio_cache_order.pop_front() {
                self.audio_cache.remove(&oldest);
            }
        }
    }

    pub fn cached_audio_preview(&self, path: &Path) -> Option<&AudioPreview> {
        self.audio_cache.get(&AudioPreviewKey::new(path.to_path_buf()))
    }

    // -----------------------------------------------------------------------
    // Image preview (multi-thread worker, debounced)
    // -----------------------------------------------------------------------

    pub fn poll_image_previews(&mut self) {
        loop {
            match self.image_jobs_rx.try_recv() {
                Ok(preview) => {
                    let key = preview.key.clone();
                    self.image_pending.remove(&key);
                    self.image_cache.insert(key.clone(), preview);
                    self.image_cache_order.push_back(key.clone());
                    while self.image_cache_order.len() > IMAGE_CACHE_LIMIT {
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
            .send(ImageJob { path: path.to_path_buf(), area })
            .is_ok()
        {
            self.image_pending.insert(key);
        }
    }

    pub fn cached_image_preview(&self, path: &Path, area: Rect) -> Option<&ImagePreview> {
        self.image_cache.get(&ImagePreviewKey::new(path.to_path_buf(), area))
    }

    pub fn prefetch_visible_image_previews(&mut self, area: Rect) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let start = self.contents_selected.saturating_sub(1);
        let end = self.contents_selected.saturating_add(3).min(self.entries.len());
        for i in start..end {
            if let Some(entry) = self.entries.get(i) {
                let path = entry.path.clone();
                if is_supported_image(&path) {
                    self.update_image_preview(&path, area);
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Movement
    // -----------------------------------------------------------------------

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

    fn move_selection(&mut self, delta: isize) {
        match self.active_panel {
            Panel::Places => {
                if self.places.is_empty() { return; }
                self.places_selected = move_index(self.places_selected, delta, self.places.len());
                self.refresh_hovered_place_entries();
                self.refresh_preview();
            }
            Panel::Contents | Panel::Preview => {
                if self.entries.is_empty() { return; }
                self.contents_selected =
                    move_index(self.contents_selected, delta, self.entries.len());
                // Refresh metadata-only preview immediately; image/text jobs
                // are dispatched by the debounce tick after DEBOUNCE_MS.
                self.refresh_preview();
            }
        }
        self.last_move_time = Instant::now();
        self.debounce_fired = false;
    }

    // -----------------------------------------------------------------------
    // Dialog key handling
    // -----------------------------------------------------------------------

    fn handle_dialog_key(&mut self, key: KeyEvent) -> bool {
        let Some(mut dialog) = self.dialog.take() else { return false };

        let mut keep = true;

        match &mut dialog {
            Dialog::Input { value, action, .. } => match key.code {
                KeyCode::Esc => {
                    if matches!(
                        action,
                        InputAction::SearchCurrent { .. } | InputAction::SearchRecursive { .. }
                    ) {
                        if let Err(e) = self.restore_search_mode() {
                            self.set_status(format!("Error: {e}"));
                        } else {
                            self.set_status("Search canceled.");
                        }
                    } else {
                        self.set_status("Canceled.");
                    }
                    keep = false;
                }
                KeyCode::Enter => {
                    let trimmed = value.trim().to_string();
                    let action = action.clone();
                    let is_search = matches!(
                        action,
                        InputAction::SearchCurrent { .. } | InputAction::SearchRecursive { .. }
                    );
                    if !is_search && trimmed.is_empty() {
                        self.set_status("Value cannot be empty.");
                        self.dialog = Some(dialog);
                        return true;
                    }
                    match self.commit_input_action(action, trimmed) {
                        Ok(()) => keep = false,
                        Err(e) => self.set_status(format!("Error: {e}")),
                    }
                }
                KeyCode::Backspace => {
                    value.pop();
                    let action = action.clone();
                    let val = value.clone();
                    self.update_live_search(action, val);
                }
                KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    value.push(ch);
                    let action = action.clone();
                    let val = value.clone();
                    self.update_live_search(action, val);
                }
                _ => {}
            },
            Dialog::ConfirmDelete { paths, .. } => match key.code {
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    self.set_status("Delete canceled.");
                    keep = false;
                }
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    let targets = paths.clone();
                    keep = false;
                    let mut errors = Vec::new();
                    for target in targets {
                        let label = target
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("item")
                            .to_string();
                        if let Err(e) = delete(&target) {
                            errors.push(format!("{label}: {e}"));
                        }
                    }
                    if errors.is_empty() {
                        match self.refresh_entries() {
                            Ok(()) => self.set_status("Moved to Recycle Bin."),
                            Err(e) => self.set_status(format!("Error: {e}")),
                        }
                        self.selected_indices.clear();
                    } else {
                        self.set_status(format!("Delete errors: {}", errors.join("; ")));
                    }
                }
                _ => {}
            },
            Dialog::Conflict { destination, choice } => match key.code {
                KeyCode::Esc => {
                    self.set_status("Paste canceled.");
                    keep = false;
                }
                KeyCode::Left | KeyCode::Up | KeyCode::Char('h') | KeyCode::Char('k') => {
                    *choice = (*choice).prev();
                }
                KeyCode::Right | KeyCode::Down | KeyCode::Char('l') | KeyCode::Char('j') => {
                    *choice = (*choice).next();
                }
                KeyCode::Enter => {
                    let choice = *choice;
                    let dest = destination.clone();
                    keep = false;
                    if let Err(e) = self.resolve_paste_conflict(choice, dest) {
                        self.set_status(format!("Error: {e}"));
                    }
                }
                _ => {}
            },
        }

        if keep {
            self.dialog = Some(dialog);
        }
        self.refresh_preview();
        true
    }

    // -----------------------------------------------------------------------
    // Settings
    // -----------------------------------------------------------------------

    fn handle_settings_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('s') => {
                self.settings_visible = false;
                self.set_status("Settings closed.");
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.reset_colors_to_defaults()
            }
            KeyCode::Enter => {
                if self.open_selected_setting_editor() { return; }
                self.settings_visible = false;
                self.set_status("Settings saved.");
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.settings_selected = self.settings_selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.settings_selected =
                    (self.settings_selected + 1).min(SettingField::ALL.len().saturating_sub(1));
            }
            KeyCode::Left | KeyCode::Char('h') => self.cycle_setting(-1),
            KeyCode::Right | KeyCode::Char('l') => self.cycle_setting(1),
            _ => {}
        }
    }

    fn open_selected_setting_editor(&mut self) -> bool {
        match SettingField::from_index(self.settings_selected) {
            SettingField::AccentColor => {
                self.begin_color_input("Accent color", self.config.accent_color.clone(), ColorTarget::Accent);
                true
            }
            SettingField::ForegroundColor => {
                self.begin_color_input("Foreground color", self.config.foreground_color.clone(), ColorTarget::Foreground);
                true
            }
            SettingField::BackgroundColor => {
                self.begin_color_input("Background color", self.config.background_color.clone(), ColorTarget::Background);
                true
            }
            SettingField::MutedColor => {
                self.begin_color_input("Muted color", self.config.muted_color.clone(), ColorTarget::Muted);
                true
            }
            SettingField::StartDir => {
                self.begin_start_dir_input();
                true
            }
            SettingField::RememberLastFolder => {
                self.config.remember_last_folder = !self.config.remember_last_folder;
                self.persist_config("Remember last folder toggled.");
                true
            }
            _ => false,
        }
    }

    fn toggle_settings(&mut self) {
        self.settings_visible = !self.settings_visible;
        if self.settings_visible { self.help_visible = false; }
        let msg = if self.settings_visible {
            "Settings opened. Use Left/Right to change, Enter to edit."
        } else {
            "Settings closed."
        };
        self.set_status(msg);
    }

    fn cycle_setting(&mut self, delta: isize) {
        match SettingField::from_index(self.settings_selected) {
            SettingField::Theme => {
                self.config.theme = if delta < 0 { self.config.theme.prev() } else { self.config.theme.next() };
                self.persist_config("Theme updated.");
            }
            SettingField::BorderStyle => {
                self.config.border_style = if delta < 0 { self.config.border_style.prev() } else { self.config.border_style.next() };
                self.persist_config("Border style updated.");
            }
            SettingField::PanelLayout => {
                self.config.panel_layout = if delta < 0 { self.config.panel_layout.prev() } else { self.config.panel_layout.next() };
                self.persist_config("Panel layout updated.");
            }
            SettingField::SortField => {
                self.config.sort_field = self.config.sort_field.next();
                match self.refresh_entries() {
                    Ok(()) => self.persist_config("Sort field updated."),
                    Err(e) => self.set_status(format!("Error: {e}")),
                }
            }
            SettingField::RememberLastFolder => {
                self.config.remember_last_folder = !self.config.remember_last_folder;
                self.persist_config("Remember last folder toggled.");
            }
            SettingField::FooterTips => {
                self.config.show_footer_tips = !self.config.show_footer_tips;
                self.persist_config("Footer tips toggled.");
            }
            SettingField::ShowHidden => {
                self.config.show_hidden = !self.config.show_hidden;
                self.show_hidden = self.config.show_hidden;
                match self.refresh_entries() {
                    Ok(()) => self.persist_config("Hidden files setting updated."),
                    Err(e) => self.set_status(format!("Error: {e}")),
                }
            }
            _ => {}
        }
    }

    fn persist_config(&mut self, msg: &str) {
        match self.config.save() {
            Ok(()) => self.set_status(msg),
            Err(e) => self.set_status(format!("Error saving config: {e}")),
        }
    }

    fn reset_colors_to_defaults(&mut self) {
        let d = CabinConfig::default();
        self.config.theme = ThemePreset::Dark;
        self.config.accent_color = d.accent_color;
        self.config.foreground_color = d.foreground_color;
        self.config.background_color = d.background_color;
        self.config.muted_color = d.muted_color;
        self.persist_config("Colors reset to defaults.");
        self.refresh_preview();
    }

    // -----------------------------------------------------------------------
    // Settings rows (for UI rendering)
    // -----------------------------------------------------------------------

    pub fn settings_rows(&self) -> Vec<String> {
        vec![
            format!("Theme: {}", self.config.theme_label()),
            format!("Accent color: {}", self.config.accent_color),
            format!("Foreground color: {}", self.config.foreground_color),
            format!("Background color: {}", self.config.background_color),
            format!("Muted color: {}", self.config.muted_color),
            format!("Border style: {}", self.config.border_style.label()),
            format!("Panel layout: {}", self.config.panel_layout.label()),
            format!("Sort by: {}", self.config.sort_field.label()),
            format!("Start dir: {}", self.config.start_dir),
            format!("Remember last folder: {}", bool_label(self.config.remember_last_folder)),
            format!("Footer tips: {}", bool_label(self.config.show_footer_tips)),
            format!("Show hidden files: {}", bool_label(self.config.show_hidden)),
        ]
    }

    // -----------------------------------------------------------------------
    // Live search
    // -----------------------------------------------------------------------

    fn update_live_search(&mut self, action: InputAction, query: String) {
        let trimmed = query.trim().to_string();

        if trimmed.is_empty() {
            if let Some((mode, selected)) = self.search_restore_mode.clone() {
                self.contents_mode = mode;
                self.contents_selected = selected;
                let _ = self.apply_contents_mode();
            }
            return;
        }

        let result = match action {
            InputAction::SearchCurrent { base } => {
                self.contents_mode = ContentsMode::SearchCurrent { base, query: trimmed };
                self.contents_selected = 0;
                self.apply_contents_mode()
            }
            InputAction::SearchRecursive { base } => {
                self.contents_mode = ContentsMode::SearchRecursive { base, query: trimmed };
                self.contents_selected = 0;
                self.apply_contents_mode()
            }
            _ => Ok(()),
        };

        if let Err(e) = result {
            self.set_status(format!("Error: {e}"));
        }
    }

    fn restore_search_mode(&mut self) -> Result<()> {
        if let Some((mode, selected)) = self.search_restore_mode.clone() {
            self.contents_mode = mode;
            self.contents_selected = selected;
            self.apply_contents_mode()?;
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Preview lines — places
    // -----------------------------------------------------------------------

    pub fn place_preview_lines(&self) -> Vec<String> {
        if self.places.is_empty() {
            return vec![String::from("No places available.")];
        }
        let i = self.places_selected.min(self.places.len().saturating_sub(1));
        let place = &self.places[i];
        vec![
            String::from("Type: Shortcut"),
            format!("Name: {}", place.name),
            format!("Path: {}", place.path.display()),
        ]
    }

    // -----------------------------------------------------------------------
    // Preview lines — entries
    // -----------------------------------------------------------------------

    pub fn entry_preview_lines(&self) -> Vec<String> {
        let Some(entry) = self.current_selection() else {
            return vec![
                String::from("Type: Folder"),
                format!("Path: {}", self.current_dir.display()),
                String::from("Items: 0"),
            ];
        };

        if entry.kind == EntryKind::Directory {
            let children = read_directory(&entry.path, self.show_hidden, &self.config).unwrap_or_default();
            let mut lines = vec![
                String::from("Type: Folder"),
                format!("Path: {}", entry.path.display()),
                format!("Items: {}", children.len()),
                format!("Created: {}", fmt_opt_time(entry.created)),
                format!("Modified: {}", fmt_opt_time(entry.modified)),
                String::from("Contents:"),
            ];
            if children.is_empty() {
                lines.push(String::from("(empty)"));
            } else {
                for child in children.iter().take(18) {
                    lines.push(self.entry_list_label(child));
                }
                if children.len() > 18 {
                    lines.push(format!("... and {} more", children.len() - 18));
                }
            }
            return lines;
        }

        // Build common file metadata lines
        let mut lines = vec![
            format!("Type: {}", file_type_label(entry)),
            format!("Extension: {}", entry.extension.as_deref().unwrap_or("(none)")),
            format!("Size: {}", entry.size.map(human_size).unwrap_or_else(|| String::from("Unknown"))),
            format!("Created: {}", fmt_opt_time(entry.created)),
            format!("Modified: {}", fmt_opt_time(entry.modified)),
            format!("Path: {}", entry.path.display()),
        ];

        if is_supported_video(&entry.path) {
            if let Some(preview) = self.cached_video_preview(&entry.path) {
                lines = preview.lines.clone();
                if let Some(e) = &preview.error { lines.push(format!("Note: {e}")); }
            }
        } else if is_supported_audio(&entry.path) {
            if let Some(preview) = self.cached_audio_preview(&entry.path) {
                lines = preview.lines.clone();
                if let Some(e) = &preview.error { lines.push(format!("Note: {e}")); }
            }
        } else if is_supported_text(&entry.path) || is_supported_pdf(&entry.path) {
            lines.push(String::from("Preview: see right panel"));
        }

        // Multi-select info
        if !self.selected_indices.is_empty() {
            lines.push(String::new());
            lines.push(format!("{} item(s) selected", self.selected_indices.len()));
        }

        lines
    }

    // -----------------------------------------------------------------------
    // List labels
    // -----------------------------------------------------------------------

    pub fn place_list_label(&self, place: &Place) -> String {
        format!("{} {}", place_icon(&place.name), place.name)
    }

    pub fn entry_list_label(&self, entry: &FileEntry) -> String {
        let name = if entry.kind == EntryKind::Directory {
            format!("{}/", entry.name)
        } else {
            entry.name.clone()
        };
        let prefix = if entry.is_hidden { ". " } else { "" };
        format!("{} {prefix}{name}", entry_icon(entry))
    }

    // -----------------------------------------------------------------------
    // System metrics
    // -----------------------------------------------------------------------

    pub fn refresh_system_metrics(&mut self) {
        self.system_monitor.refresh_if_due(&self.current_dir);
    }

    pub fn performance_summary(&mut self) -> String {
        self.refresh_system_metrics();
        let s = self.system_monitor.snapshot();
        format!(
            "CPU {} | GPU {} | RAM {} | SWAP {} | DISK {}",
            fmt_metric(s.cpu),
            fmt_metric(s.gpu),
            fmt_metric(s.ram),
            fmt_metric(s.swap),
            fmt_metric(s.disk),
        )
    }

    // -----------------------------------------------------------------------
    // Opening items
    // -----------------------------------------------------------------------

    fn open_selected(&mut self) {
        let result = match self.active_panel {
            Panel::Places => self.open_place(),
            Panel::Contents | Panel::Preview => self.open_current_entry(),
        };
        if let Err(e) = result {
            self.set_status(format!("Error: {e}"));
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
            EntryKind::Directory | EntryKind::Drive => {
                self.open_directory(entry.path)?;
                self.set_status(format!("Opened {}", entry.name));
            }
            _ => {
                open_with_system(&entry.path)
                    .with_context(|| format!("Could not open {}", entry.path.display()))?;
                self.set_status(format!("Opened externally: {}", entry.name));
            }
        }
        Ok(())
    }

    fn open_directory(&mut self, path: PathBuf) -> Result<()> {
        if !path.exists() {
            return Err(anyhow!("Path does not exist"));
        }
        self.current_dir = path;
        self.selected_indices.clear();
        if self.config.remember_last_folder {
            self.config.last_folder = Some(self.current_dir.display().to_string());
            let _ = self.config.save();
        }
        self.contents_mode = ContentsMode::Directory { path: self.current_dir.clone() };
        self.sync_places_selection();
        // Reset cursor to top when entering a new directory.
        self.contents_selected = 0;
        self.refresh_entries()?;
        Ok(())
    }

    fn go_parent(&mut self) {
        let Some(parent) = self.current_dir.parent().map(Path::to_path_buf) else {
            self.set_status("Already at the top level.");
            return;
        };
        if let Err(e) = self.open_directory(parent) {
            self.set_status(format!("Error: {e}"));
        }
    }

    fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.config.show_hidden = self.show_hidden;
        match self.refresh_entries() {
            Ok(()) => match self.config.save() {
                Ok(()) => self.set_status(if self.show_hidden { "Hidden files shown." } else { "Hidden files hidden." }),
                Err(e) => self.set_status(format!("Error: {e}")),
            },
            Err(e) => self.set_status(format!("Error: {e}")),
        }
    }

    // -----------------------------------------------------------------------
    // Panel navigation
    // -----------------------------------------------------------------------

    fn next_panel(&mut self) {
        self.active_panel = match self.active_panel {
            Panel::Places => Panel::Contents,
            Panel::Contents => Panel::Preview,
            Panel::Preview => Panel::Places,
        };
        self.refresh_hovered_place_entries();
        self.refresh_preview();
    }

    fn previous_panel(&mut self) {
        self.active_panel = match self.active_panel {
            Panel::Places => Panel::Preview,
            Panel::Contents => Panel::Places,
            Panel::Preview => Panel::Contents,
        };
        self.refresh_hovered_place_entries();
        self.refresh_preview();
    }

    // -----------------------------------------------------------------------
    // Selection helpers
    // -----------------------------------------------------------------------

    pub fn current_selection(&self) -> Option<&FileEntry> {
        self.entries.get(self.contents_selected)
    }

    fn selected_entry(&self) -> Option<&FileEntry> {
        if matches!(self.active_panel, Panel::Contents | Panel::Preview) {
            self.current_selection()
        } else {
            None
        }
    }

    fn current_path(&self) -> &Path {
        if self.active_panel == Panel::Places {
            self.places
                .get(self.places_selected)
                .map(|p| p.path.as_path())
                .unwrap_or(self.current_dir.as_path())
        } else {
            self.current_selection()
                .map(|e| e.path.as_path())
                .unwrap_or(self.current_dir.as_path())
        }
    }

    // -----------------------------------------------------------------------
    // File operations — new file / folder
    // -----------------------------------------------------------------------

    fn begin_new_file(&mut self) {
        self.dialog = Some(Dialog::Input {
            title: String::from("New file"),
            value: String::from("new_file.txt"),
            action: InputAction::CreateFile { dir: self.current_dir.clone() },
        });
        self.set_status("Type a file name, then press Enter.");
    }

    fn begin_new_folder(&mut self) {
        self.dialog = Some(Dialog::Input {
            title: String::from("New folder"),
            value: String::from("New Folder"),
            action: InputAction::CreateFolder { dir: self.current_dir.clone() },
        });
        self.set_status("Type a folder name, then press Enter.");
    }

    // -----------------------------------------------------------------------
    // File operations — search
    // -----------------------------------------------------------------------

    fn begin_search_current(&mut self) {
        self.search_restore_mode = Some((self.contents_mode.clone(), self.contents_selected));
        self.dialog = Some(Dialog::Input {
            title: String::from("Search current folder"),
            value: String::new(),
            action: InputAction::SearchCurrent { base: self.current_dir.clone() },
        });
        self.set_status("Type to filter live. Enter keeps it, Esc restores.");
    }

    fn begin_search_recursive(&mut self) {
        self.search_restore_mode = Some((self.contents_mode.clone(), self.contents_selected));
        self.dialog = Some(Dialog::Input {
            title: String::from("Recursive search"),
            value: String::new(),
            action: InputAction::SearchRecursive { base: self.current_dir.clone() },
        });
        self.set_status("Type to filter live. Enter keeps it, Esc restores.");
    }

    // -----------------------------------------------------------------------
    // File operations — settings dialogs
    // -----------------------------------------------------------------------

    fn begin_color_input(&mut self, title: &str, value: String, target: ColorTarget) {
        self.dialog = Some(Dialog::Input {
            title: String::from(title),
            value,
            action: InputAction::SetColor { target },
        });
        self.set_status("Type a color code (#RRGGBB), then press Enter.");
    }

    fn begin_start_dir_input(&mut self) {
        self.dialog = Some(Dialog::Input {
            title: String::from("Start directory"),
            value: self.config.start_dir.clone(),
            action: InputAction::SetStartDir,
        });
        self.set_status("Type 'home', 'last', or a full path, then press Enter.");
    }

    // -----------------------------------------------------------------------
    // File operations — rename / delete
    // -----------------------------------------------------------------------

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
        let paths = self.effective_selection_paths();
        if paths.is_empty() {
            self.set_status("Select a file or folder in Contents first.");
            return;
        }
        let label = if paths.len() == 1 {
            paths[0]
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("item")
                .to_string()
        } else {
            format!("{} items", paths.len())
        };
        self.dialog = Some(Dialog::ConfirmDelete { paths, label });
        self.set_status("Confirm delete with Y, cancel with N.");
    }

    // -----------------------------------------------------------------------
    // File operations — copy / cut / paste
    // -----------------------------------------------------------------------

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
        let Some(op) = self.pending_operation.clone() else {
            self.set_status("Nothing to paste.");
            return;
        };
        let Some(src_name) = op.source.file_name().map(|n| n.to_owned()) else {
            self.set_status("Source path has no file name.");
            return;
        };
        let dest = self.current_dir.join(&src_name);

        if same_path(&op.source, &dest) {
            self.set_status("Source and destination are the same.");
            return;
        }
        if dest.exists() {
            self.dialog = Some(Dialog::Conflict {
                destination: dest,
                choice: ConflictChoice::Cancel,
            });
            self.set_status("Destination exists. Choose Keep both, Replace, or Cancel.");
            return;
        }
        if let Err(e) = self.perform_paste(op, dest) {
            self.set_status(format!("Error: {e}"));
        }
    }

    fn refresh_current(&mut self) {
        match self.refresh_entries() {
            Ok(()) => self.set_status("Refreshed."),
            Err(e) => self.set_status(format!("Error: {e}")),
        }
    }

    fn copy_current_path(&mut self) {
        let path = self.current_path().display().to_string();
        // Use the persistent clipboard instance to avoid X11/Wayland data loss.
        let result = match &mut self.clipboard {
            Some(cb) => cb.set_text(path.clone()).map_err(|e| e.to_string()),
            None => {
                // Lazy-init in case init failed at startup.
                match Clipboard::new() {
                    Ok(mut cb) => {
                        let r = cb.set_text(path.clone()).map_err(|e| e.to_string());
                        self.clipboard = Some(cb);
                        r
                    }
                    Err(e) => Err(e.to_string()),
                }
            }
        };
        match result {
            Ok(()) => self.set_status(format!("Copied path: {path}")),
            Err(e) => self.set_status(format!("Clipboard error: {e}")),
        }
    }

    // -----------------------------------------------------------------------
    // Status lines appended to preview (renamed from with_clipboard_info)
    // -----------------------------------------------------------------------

    fn append_status_lines(&self, mut lines: Vec<String>) -> Vec<String> {
        match &self.contents_mode {
            ContentsMode::Directory { path } => {
                lines.push(String::new());
                lines.push(format!("Folder: {}", path.display()));
            }
            ContentsMode::SearchCurrent { base, query } => {
                lines.push(String::new());
                lines.push(String::from("Search: current folder"));
                lines.push(format!("Base: {}", base.display()));
                lines.push(format!("Query: {query}"));
            }
            ContentsMode::SearchRecursive { base, query } => {
                lines.push(String::new());
                lines.push(String::from("Search: recursive"));
                lines.push(format!("Base: {}", base.display()));
                lines.push(format!("Query: {query}"));
            }
        }
        if let Some(op) = &self.pending_operation {
            lines.push(String::new());
            let mode = match op.mode { ClipboardMode::Copy => "Copy", ClipboardMode::Cut => "Cut" };
            lines.push(format!("Clipboard: {mode} {}", op.name));
            lines.push(format!("Source: {}", op.source.display()));
            lines.push(String::from("Press p to paste here."));
        }
        lines
    }

    // -----------------------------------------------------------------------
    // Commit input action
    // -----------------------------------------------------------------------

    fn commit_input_action(&mut self, action: InputAction, raw: String) -> Result<()> {
        let name = raw.trim();

        match action {
            InputAction::CreateFile { dir } => {
                // Security: reject names that contain path separators or '..'
                validate_filename(name)?;
                let path = dir.join(name);
                if path.exists() { return Err(anyhow!("A file with this name already exists")); }
                File::create(&path)
                    .with_context(|| format!("Unable to create {}", path.display()))?;
                self.refresh_entries()?;
                self.select_entry_by_path(&path);
                self.set_status(format!("Created {name}"));
            }
            InputAction::CreateFolder { dir } => {
                validate_filename(name)?;
                let path = dir.join(name);
                if path.exists() { return Err(anyhow!("A folder with this name already exists")); }
                fs::create_dir(&path)
                    .with_context(|| format!("Unable to create {}", path.display()))?;
                self.refresh_entries()?;
                self.select_entry_by_path(&path);
                self.set_status(format!("Created {name}"));
            }
            InputAction::Rename { source } => {
                validate_filename(name)?;
                if name.is_empty() { return Err(anyhow!("Name cannot be empty")); }
                let parent = source.parent()
                    .ok_or_else(|| anyhow!("Cannot rename this item"))?;
                let dest = parent.join(name);
                if dest.exists() { return Err(anyhow!("A file with this name already exists")); }
                fs::rename(&source, &dest).with_context(|| {
                    format!("Unable to rename {} to {}", source.display(), dest.display())
                })?;
                self.refresh_entries()?;
                self.select_entry_by_path(&dest);
                self.set_status(format!("Renamed to {name}"));
            }
            InputAction::SearchCurrent { base } => {
                if name.is_empty() {
                    self.restore_search_mode()?;
                    self.search_restore_mode = None;
                    self.set_status("Search canceled.");
                } else {
                    self.contents_mode = ContentsMode::SearchCurrent { base, query: name.to_string() };
                    self.contents_selected = 0;
                    self.apply_contents_mode()?;
                    self.search_restore_mode = None;
                    self.set_status(format!("Filtered for \"{name}\"."));
                }
            }
            InputAction::SearchRecursive { base } => {
                if name.is_empty() {
                    self.restore_search_mode()?;
                    self.search_restore_mode = None;
                    self.set_status("Search canceled.");
                } else {
                    self.contents_mode = ContentsMode::SearchRecursive { base, query: name.to_string() };
                    self.contents_selected = 0;
                    self.apply_contents_mode()?;
                    self.search_restore_mode = None;
                    self.set_status(format!("Search results for \"{name}\"."));
                }
            }
            InputAction::SetColor { target } => {
                let _ = crate::config::parse_color_value(name)?;
                self.config.theme = ThemePreset::Custom;
                match target {
                    ColorTarget::Accent => self.config.accent_color = name.to_string(),
                    ColorTarget::Foreground => self.config.foreground_color = name.to_string(),
                    ColorTarget::Background => self.config.background_color = name.to_string(),
                    ColorTarget::Muted => self.config.muted_color = name.to_string(),
                }
                self.persist_config("Color updated.");
                self.refresh_preview();
            }
            InputAction::SetStartDir => {
                let normalized = normalize_start_dir(name)?;
                self.config.start_dir = normalized;
                self.persist_config("Start directory updated.");
            }
            InputAction::JumpToPath => {
                if name.is_empty() { return Ok(()); }
                let path = PathBuf::from(name);
                // Canonicalize to resolve '..' components safely.
                let canonical = path
                    .canonicalize()
                    .with_context(|| format!("Path does not exist: {name}"))?;
                if canonical.is_dir() {
                    self.open_directory(canonical)?;
                } else if canonical.is_file() {
                    if let Some(parent) = canonical.parent() {
                        self.open_directory(parent.to_path_buf())?;
                        self.select_entry_by_path(&canonical);
                    }
                }
                self.set_status(format!("Jumped to {name}"));
            }
            InputAction::AddBookmark => {
                if name.is_empty() { return Err(anyhow!("Bookmark name cannot be empty")); }
                self.add_bookmark(name.to_string());
            }
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Paste helpers
    // -----------------------------------------------------------------------

    fn resolve_paste_conflict(&mut self, choice: ConflictChoice, dest: PathBuf) -> Result<()> {
        let op = self.pending_operation.clone()
            .ok_or_else(|| anyhow!("Nothing to paste"))?;
        match choice {
            ConflictChoice::Cancel => { self.set_status("Paste canceled."); Ok(()) }
            ConflictChoice::KeepBoth => {
                let unique = unique_copy_destination(&dest);
                self.perform_paste(op, unique)
            }
            ConflictChoice::Replace => {
                if dest.exists() { remove_path_recursive(&dest)?; }
                self.perform_paste(op, dest)
            }
        }
    }

    fn perform_paste(&mut self, op: PendingOperation, dest: PathBuf) -> Result<()> {
        let src_name = op.source.file_name()
            .ok_or_else(|| anyhow!("Source path has no file name"))?
            .to_owned();

        if same_path(&op.source, &dest) {
            return Err(anyhow!("Source and destination are the same"));
        }

        match op.mode {
            ClipboardMode::Copy => copy_path_recursive(&op.source, &dest)?,
            ClipboardMode::Cut => move_path_recursive(&op.source, &dest)?,
        }
        if op.mode == ClipboardMode::Cut {
            self.pending_operation = None;
        }
        self.refresh_entries()?;
        self.select_entry_by_path(&dest);
        let verb = if op.mode == ClipboardMode::Cut { "Moved" } else { "Copied" };
        self.set_status(format!("{verb} {}.", src_name.to_string_lossy()));
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Entry selection / refresh
    // -----------------------------------------------------------------------

    fn select_entry_by_path(&mut self, path: &Path) {
        if let Some((i, _)) = self
            .entries
            .iter()
            .enumerate()
            .find(|(_, e)| same_path(&e.path, path))
        {
            self.contents_selected = i;
        }
    }

    fn refresh_entries(&mut self) -> Result<()> {
        self.directory_entries = read_directory(
            &self.current_dir,
            self.show_hidden,
            &self.config,
        )?;
        self.apply_contents_mode()
    }

    fn apply_contents_mode(&mut self) -> Result<()> {
        self.entries = match &self.contents_mode.clone() {
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

        // Remove stale selected_indices after list changes.
        self.selected_indices
            .retain(|&i| i < self.entries.len());

        self.refresh_hovered_place_entries();
        // Call refresh_preview exactly once here.
        self.refresh_preview();
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
        match read_directory(&place.path, self.show_hidden, &self.config) {
            Ok(entries) => {
                self.hovered_place_entries = entries;
                self.hovered_place_error = None;
            }
            Err(e) => {
                self.hovered_place_entries.clear();
                self.hovered_place_error = Some(format!("Unable to read {}: {e}", place.path.display()));
            }
        }
    }

    fn sync_places_selection(&mut self) {
        if let Some((i, _)) = self
            .places
            .iter()
            .enumerate()
            .find(|(_, p)| same_path(&p.path, &self.current_dir))
        {
            self.places_selected = i;
        }
    }

    fn set_status<S: Into<String>>(&mut self, msg: S) {
        self.status_message = Some(msg.into());
    }
}

// ---------------------------------------------------------------------------
// Security: filename validation
// ---------------------------------------------------------------------------

/// Rejects filenames that contain path separators or parent-directory
/// references to prevent path traversal during create / rename.
fn validate_filename(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(anyhow!("Name cannot be empty"));
    }
    let p = Path::new(name);
    for component in p.components() {
        match component {
            Component::Normal(_) => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(anyhow!(
                    "Name must not contain path separators or '..'"
                ));
            }
            Component::CurDir => {}
        }
    }
    // Also reject explicit separator characters.
    if name.contains('/') || name.contains('\\') {
        return Err(anyhow!("Name must not contain path separators"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Filesystem helpers
// ---------------------------------------------------------------------------

fn copy_path_recursive(src: &Path, dest: &Path) -> Result<()> {
    let meta = fs::metadata(src)
        .with_context(|| format!("Unable to read metadata for {}", src.display()))?;
    if meta.is_dir() {
        fs::create_dir(dest)
            .with_context(|| format!("Unable to create {}", dest.display()))?;
        for item in fs::read_dir(src)
            .with_context(|| format!("Unable to read {}", src.display()))?
        {
            let item = item?;
            copy_path_recursive(&item.path(), &dest.join(item.file_name()))?;
        }
    } else {
        fs::copy(src, dest).with_context(|| {
            format!("Unable to copy {} to {}", src.display(), dest.display())
        })?;
    }
    Ok(())
}

fn move_path_recursive(src: &Path, dest: &Path) -> Result<()> {
    match fs::rename(src, dest) {
        Ok(()) => Ok(()),
        Err(_) => {
            copy_path_recursive(src, dest)?;
            remove_path_recursive(src)
        }
    }
}

fn remove_path_recursive(path: &Path) -> Result<()> {
    let meta = fs::metadata(path)
        .with_context(|| format!("Unable to read metadata for {}", path.display()))?;
    if meta.is_dir() {
        fs::remove_dir_all(path).with_context(|| format!("Unable to remove {}", path.display()))?;
    } else {
        fs::remove_file(path).with_context(|| format!("Unable to remove {}", path.display()))?;
    }
    Ok(())
}

fn build_places(config: &CabinConfig) -> Vec<Place> {
    let mut places = Vec::new();

    if let Some(base) = BaseDirs::new() {
        places.push(Place { name: String::from("Home"), path: base.home_dir().to_path_buf() });
    }

    if let Some(ud) = UserDirs::new() {
        add_place_if_exists(&mut places, "Desktop", ud.desktop_dir().map(Path::to_path_buf));
        add_place_if_exists(&mut places, "Downloads", ud.download_dir().map(Path::to_path_buf));
        add_place_if_exists(&mut places, "Documents", ud.document_dir().map(Path::to_path_buf));
        add_place_if_exists(&mut places, "Pictures", ud.picture_dir().map(Path::to_path_buf));
        add_place_if_exists(&mut places, "Videos", ud.video_dir().map(Path::to_path_buf));
        add_place_if_exists(&mut places, "Music", ud.audio_dir().map(Path::to_path_buf));
    }

    #[cfg(windows)]
    for letter in b'A'..=b'Z' {
        let drive = PathBuf::from(format!("{}:\\", letter as char));
        if drive.exists() {
            places.push(Place { name: format!("{}:", letter as char), path: drive });
        }
    }

    #[cfg(not(windows))]
    places.push(Place { name: String::from("Root"), path: PathBuf::from("/") });

    // Append user bookmarks.
    for (name, path_str) in &config.bookmarks {
        let path = PathBuf::from(path_str);
        if path.exists() {
            places.push(Place { name: name.clone(), path });
        }
    }

    places.dedup_by(|a, b| same_path(&a.path, &b.path));
    places
}

fn add_place_if_exists(places: &mut Vec<Place>, name: &str, path: Option<PathBuf>) {
    if let Some(p) = path {
        if p.exists() {
            places.push(Place { name: String::from(name), path: p });
        }
    }
}

fn read_directory(dir: &Path, show_hidden: bool, config: &CabinConfig) -> Result<Vec<FileEntry>> {
    let mut entries = Vec::new();
    let rd = fs::read_dir(dir)
        .with_context(|| format!("Unable to read {}", dir.display()))?;

    for item in rd {
        let item = item?;
        let path = item.path();
        let name = item.file_name().to_string_lossy().to_string();
        let metadata = item.metadata().ok();
        let file_type = item.file_type().ok();
        let hidden = is_hidden(&path, &name, metadata.as_ref());

        if hidden && !show_hidden {
            continue;
        }

        let kind = match file_type.as_ref() {
            Some(ft) if ft.is_dir() => EntryKind::Directory,
            Some(ft) if ft.is_symlink() => EntryKind::Symlink,
            Some(ft) if ft.is_file() => EntryKind::File,
            _ => EntryKind::Unknown,
        };

        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{e}"));

        entries.push(FileEntry {
            name,
            path,
            kind,
            size: metadata.as_ref().map(|m| m.len()),
            created: metadata.as_ref().and_then(|m| m.created().ok()),
            modified: metadata.as_ref().and_then(|m| m.modified().ok()),
            extension,
            is_hidden: hidden,
        });
    }

    entries.sort_by(|a, b| compare_entries(a, b, config.sort_field, config.sort_descending));
    Ok(entries)
}

fn filter_entries(entries: &[FileEntry], query: &str) -> Vec<FileEntry> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return entries.to_vec();
    }
    entries
        .iter()
        .filter(|e| {
            e.name.to_lowercase().contains(&needle)
                || e.path.to_string_lossy().to_lowercase().contains(&needle)
        })
        .cloned()
        .collect()
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

    for entry in walker {
        // Permission-denied and other IO errors: skip silently instead of
        // aborting the whole search.
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path == base { continue; }

        let file_name = entry.file_name().to_string_lossy().to_string();
        if !show_hidden && is_hidden(path, &file_name, None) { continue; }
        if is_ignored_search_dir(&file_name) { continue; }

        let metadata = entry.metadata().ok();
        let ft = entry.file_type();
        let kind = match ft {
            Some(ft) if ft.is_dir() => EntryKind::Directory,
            Some(ft) if ft.is_symlink() => EntryKind::Symlink,
            Some(ft) if ft.is_file() => EntryKind::File,
            _ => EntryKind::Unknown,
        };

        let relative = path.strip_prefix(base).unwrap_or(path);
        let rel_str = relative.to_string_lossy().to_string();
        if !needle.is_empty()
            && !rel_str.to_lowercase().contains(&needle)
            && !file_name.to_lowercase().contains(&needle)
        {
            continue;
        }

        results.push(FileEntry {
            name: rel_str,
            path: path.to_path_buf(),
            kind,
            size: metadata.as_ref().map(|m| m.len()),
            created: metadata.as_ref().and_then(|m| m.created().ok()),
            modified: metadata.as_ref().and_then(|m| m.modified().ok()),
            extension: path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| format!(".{e}")),
            is_hidden: is_hidden(path, &file_name, metadata.as_ref()),
        });
    }

    results.sort_by(|a, b| compare_entries(a, b, SortField::Name, false));
    Ok(results)
}

fn is_ignored_search_dir(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        ".git" | "node_modules" | "target" | "dist" | "build" | ".cache"
    )
}

fn compare_entries(
    l: &FileEntry,
    r: &FileEntry,
    field: SortField,
    descending: bool,
) -> Ordering {
    // Directories always sort before files regardless of sort field.
    match (l.kind == EntryKind::Directory, r.kind == EntryKind::Directory) {
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        _ => {}
    }

    let base = match field {
        SortField::Name => l
            .name
            .to_lowercase()
            .cmp(&r.name.to_lowercase())
            .then_with(|| l.name.cmp(&r.name)),
        SortField::Size => l.size.unwrap_or(0).cmp(&r.size.unwrap_or(0)),
        SortField::Modified => l.modified.cmp(&r.modified),
        SortField::Extension => {
            l.extension
                .as_deref()
                .unwrap_or("")
                .cmp(r.extension.as_deref().unwrap_or(""))
                .then_with(|| l.name.to_lowercase().cmp(&r.name.to_lowercase()))
        }
    };

    if descending { base.reverse() } else { base }
}

fn is_hidden(_path: &Path, name: &str, meta: Option<&fs::Metadata>) -> bool {
    if name.starts_with('.') { return true; }

    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if let Some(m) = meta {
            return m.file_attributes() & 0x2 != 0;
        }
    }
    #[cfg(not(windows))]
    let _ = meta;

    false
}

fn move_index(cur: usize, delta: isize, len: usize) -> usize {
    if len == 0 { return 0; }
    (cur as isize + delta).clamp(0, len.saturating_sub(1) as isize) as usize
}

fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .eq_ignore_ascii_case(b.to_string_lossy().trim_end_matches(['\\', '/']))
    } else {
        a == b
    }
}

/// Generates a non-colliding destination path by appending " copy", " copy 2",
/// etc.  Capped at 1 000 iterations to guarantee termination.
fn unique_copy_destination(dest: &Path) -> PathBuf {
    if !dest.exists() { return dest.to_path_buf(); }

    let parent = dest.parent().unwrap_or_else(|| Path::new(""));
    let file_name = dest
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("copy");

    let (stem, ext) = if let Some(e) = dest.extension().and_then(|e| e.to_str()) {
        (dest.file_stem().and_then(|s| s.to_str()).unwrap_or(file_name), Some(e))
    } else {
        (file_name, None)
    };

    for i in 1..=1_000 {
        let candidate_name = match ext {
            Some(e) if i == 1 => format!("{stem} copy.{e}"),
            Some(e) => format!("{stem} copy {i}.{e}"),
            None if i == 1 => format!("{stem} copy"),
            None => format!("{stem} copy {i}"),
        };
        let candidate = parent.join(candidate_name);
        if !candidate.exists() { return candidate; }
    }

    // Fallback: append a timestamp to guarantee uniqueness.
    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    match ext {
        Some(e) => parent.join(format!("{stem} copy {ts}.{e}")),
        None => parent.join(format!("{stem} copy {ts}")),
    }
}

fn normalize_start_dir(v: &str) -> Result<String> {
    let t = v.trim();
    if t.is_empty() { return Err(anyhow!("Start directory cannot be empty")); }
    let low = t.to_ascii_lowercase();
    if matches!(low.as_str(), "home" | "last") { return Ok(low); }

    // Canonicalize resolves '..' components so a malicious path can't escape.
    let path = PathBuf::from(t)
        .canonicalize()
        .with_context(|| format!("Start directory does not exist: {t}"))?;
    Ok(path.display().to_string())
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

fn human_size(size: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = size as f64;
    let mut u = 0usize;
    while v >= 1024.0 && u < UNITS.len() - 1 { v /= 1024.0; u += 1; }
    if u == 0 { format!("{size} B") } else { format!("{v:.1} {}", UNITS[u]) }
}

fn fmt_metric(v: Option<f32>) -> String {
    v.map(|x| format!("{x:.0}%")).unwrap_or_else(|| String::from("n/a"))
}

fn fmt_opt_time(t: Option<SystemTime>) -> String {
    t.map(|st| {
        let dt: DateTime<Local> = st.into();
        dt.format("%Y-%m-%d %H:%M").to_string()
    })
    .unwrap_or_else(|| String::from("Unknown"))
}

fn file_type_label(entry: &FileEntry) -> &'static str {
    match entry.kind {
        EntryKind::Directory => "Folder",
        EntryKind::Symlink => "Symlink",
        EntryKind::Drive => "Drive",
        EntryKind::Unknown => "Unknown",
        EntryKind::File => {
            if is_supported_image(&entry.path) { "Image" }
            else if is_supported_video(&entry.path) { "Video" }
            else if is_supported_audio(&entry.path) { "Audio" }
            else if is_supported_pdf(&entry.path) { "PDF" }
            else if is_supported_text(&entry.path) { "Text" }
            else { "File" }
        }
    }
}

fn bool_label(v: bool) -> &'static str {
    if v { "On" } else { "Off" }
}

// ---------------------------------------------------------------------------
// System open
// ---------------------------------------------------------------------------

fn open_with_system(path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        Command::new("cmd")
            .args(["/C", "start", "", &path.to_string_lossy()])
            .status()
            .with_context(|| format!("Failed to launch {}", path.display()))?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("System failed to open the item"))?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(path)
            .status()
            .with_context(|| format!("Failed to launch {}", path.display()))?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("System failed to open the item"))?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(path)
            .status()
            .with_context(|| format!("Failed to launch {}", path.display()))?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("System failed to open the item"))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Help text
// ---------------------------------------------------------------------------

fn help_lines() -> Vec<String> {
    vec![
        String::from("Cabin — keyboard shortcuts"),
        String::new(),
        String::from("q            Quit"),
        String::from("?            Toggle help"),
        String::from("s            Settings"),
        String::from("Tab          Next panel"),
        String::from("Shift+Tab    Previous panel"),
        String::from("Up/Down j/k  Move selection"),
        String::from("Enter        Open item"),
        String::from("Backspace    Parent folder (or exit search)"),
        String::from("h            Toggle hidden files"),
        String::from("Space        Toggle multi-select"),
        String::from("c            Mark for copy"),
        String::from("x            Mark for cut"),
        String::from("p            Paste"),
        String::from("r            Rename"),
        String::from("d            Delete to Recycle Bin"),
        String::from("n            New file"),
        String::from("Shift+N      New folder"),
        String::from("y            Copy path to clipboard"),
        String::from("g            Jump to path"),
        String::from("Shift+S      Cycle sort field"),
        String::from("Ctrl+B       Add current dir as bookmark"),
        String::from("/            Search current folder"),
        String::from("Ctrl+F       Recursive search"),
        String::from("F5           Refresh"),
        String::from("Settings Ctrl+R  Reset colors to defaults"),
    ]
}

// ---------------------------------------------------------------------------
// Icons
// ---------------------------------------------------------------------------

fn place_icon(name: &str) -> &'static str {
    match name.to_ascii_lowercase().as_str() {
        "home" => "󰋜",
        "desktop" => "󰍹",
        "downloads" => "󰇚",
        "documents" => "󰈙",
        "pictures" => "󰋩",
        "videos" => "󰕧",
        "music" => "󰝚",
        _ => "󰉋",
    }
}

fn entry_icon(entry: &FileEntry) -> &'static str {
    match entry.kind {
        EntryKind::Directory => "󰉋",
        EntryKind::Symlink => "󰌹",
        EntryKind::Drive => "󰋊",
        EntryKind::Unknown => "󰈔",
        EntryKind::File => {
            if is_supported_image(&entry.path) { "󰋩" }
            else if is_supported_video(&entry.path) { "󰕧" }
            else if is_supported_audio(&entry.path) { "󰝚" }
            else if is_supported_pdf(&entry.path) { "󰈦" }
            else if is_supported_text(&entry.path) { "󰈙" }
            else if let Some(ext) = entry.extension.as_deref() {
                match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
                    "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" => "󰗄",
                    "exe" | "msi" | "appimage" => "󰣇",
                    _ => "󰈔",
                }
            } else {
                "󰈔"
            }
        }
    }
}
