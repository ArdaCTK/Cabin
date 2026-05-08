use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use directories_next::BaseDirs;
use ratatui::{
    style::{Color, Modifier, Style},
    widgets::BorderType,
};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreset {
    Dark,
    Light,
    Minimal,
    Mono,
    Custom,
}

impl ThemePreset {
    pub const ALL: [Self; 5] = [
        Self::Dark,
        Self::Light,
        Self::Minimal,
        Self::Mono,
        Self::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::Minimal => "Minimal",
            Self::Mono => "Mono",
            Self::Custom => "Custom",
        }
    }

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|x| *x == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        let i = Self::ALL.iter().position(|x| *x == self).unwrap_or(0);
        Self::ALL[(i + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

// ---------------------------------------------------------------------------
// Internal palette helper
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Palette {
    foreground: Color,
    background: Color,
    accent: Color,
    muted: Color,
}

// ---------------------------------------------------------------------------
// Border style
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BorderPreset {
    Plain,
    Rounded,
    Double,
    Thick,
}

impl BorderPreset {
    pub const ALL: [Self; 4] = [Self::Plain, Self::Rounded, Self::Double, Self::Thick];

    pub fn label(self) -> &'static str {
        match self {
            Self::Plain => "Plain",
            Self::Rounded => "Rounded",
            Self::Double => "Double",
            Self::Thick => "Thick",
        }
    }

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|x| *x == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        let i = Self::ALL.iter().position(|x| *x == self).unwrap_or(0);
        Self::ALL[(i + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    pub fn border_type(self) -> BorderType {
        match self {
            Self::Plain => BorderType::Plain,
            Self::Rounded => BorderType::Rounded,
            Self::Double => BorderType::Double,
            Self::Thick => BorderType::Thick,
        }
    }
}

// ---------------------------------------------------------------------------
// Panel layout
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelLayout {
    Classic,
    Balanced,
    PreviewFocus,
    ContentsFocus,
}

impl PanelLayout {
    pub const ALL: [Self; 4] = [
        Self::Classic,
        Self::Balanced,
        Self::PreviewFocus,
        Self::ContentsFocus,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Classic => "Classic",
            Self::Balanced => "Balanced",
            Self::PreviewFocus => "Preview focus",
            Self::ContentsFocus => "Contents focus",
        }
    }

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|x| *x == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        let i = Self::ALL.iter().position(|x| *x == self).unwrap_or(0);
        Self::ALL[(i + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

// ---------------------------------------------------------------------------
// Sort field
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortField {
    Name,
    Size,
    Modified,
    Extension,
}

impl SortField {
    pub const ALL: [Self; 4] = [Self::Name, Self::Size, Self::Modified, Self::Extension];

    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Size => "Size",
            Self::Modified => "Modified",
            Self::Extension => "Extension",
        }
    }

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|x| *x == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

// ---------------------------------------------------------------------------
// Config struct
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CabinConfig {
    pub theme: ThemePreset,
    pub accent_color: String,
    pub foreground_color: String,
    pub background_color: String,
    pub muted_color: String,
    pub border_style: BorderPreset,
    pub panel_layout: PanelLayout,
    pub sort_field: SortField,
    pub sort_descending: bool,
    pub start_dir: String,
    pub remember_last_folder: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_folder: Option<String>,
    pub show_footer_tips: bool,
    pub show_hidden: bool,
    /// User-defined bookmark paths (name, path pairs).
    #[serde(default)]
    pub bookmarks: Vec<(String, String)>,
}

impl Default for CabinConfig {
    fn default() -> Self {
        Self {
            theme: ThemePreset::Dark,
            accent_color: String::from("#00D7D7"),
            foreground_color: String::from("#D7D7D7"),
            background_color: String::from("#000000"),
            muted_color: String::from("#5F5F5F"),
            border_style: BorderPreset::Rounded,
            panel_layout: PanelLayout::Balanced,
            sort_field: SortField::Name,
            sort_descending: false,
            start_dir: String::from("home"),
            remember_last_folder: true,
            last_folder: None,
            show_footer_tips: true,
            show_hidden: false,
            bookmarks: Vec::new(),
        }
    }
}

impl CabinConfig {
    pub fn load_or_default() -> (Self, Option<String>, bool) {
        let path = config_path();
        match fs::read_to_string(&path) {
            Ok(contents) => match toml::from_str::<Self>(&contents) {
                Ok(cfg) => (cfg, None, false),
                Err(e) => (
                    Self::default(),
                    Some(format!("Could not parse config.toml, using defaults: {e}")),
                    false,
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Self::default(), None, true),
            Err(e) => (
                Self::default(),
                Some(format!("Could not read config.toml, using defaults: {e}")),
                false,
            ),
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Unable to create {}", parent.display()))?;
        }
        let contents = toml::to_string_pretty(self).context("Unable to serialize config.toml")?;
        fs::write(&path, contents)
            .with_context(|| format!("Unable to write {}", path.display()))?;
        Ok(())
    }

    pub fn startup_dir(&self) -> PathBuf {
        if self.remember_last_folder {
            if let Some(last) = self.last_folder.as_deref() {
                let p = PathBuf::from(last);
                if p.exists() {
                    return p;
                }
            }
        }
        match self.start_dir.trim() {
            "" | "home" => home_dir(),
            "last" => self
                .last_folder
                .as_deref()
                .map(PathBuf::from)
                .filter(|p| p.exists())
                .unwrap_or_else(home_dir),
            other => {
                let p = PathBuf::from(other);
                if p.exists() { p } else { home_dir() }
            }
        }
    }

    fn palette(&self) -> Palette {
        match self.theme {
            ThemePreset::Dark => Palette {
                foreground: rgb(215, 215, 215),
                background: Color::Black,
                accent: rgb(0, 215, 215),
                muted: rgb(95, 95, 95),
            },
            ThemePreset::Light => Palette {
                foreground: Color::Black,
                background: Color::White,
                accent: rgb(0, 102, 204),
                muted: rgb(120, 120, 120),
            },
            ThemePreset::Minimal => Palette {
                foreground: Color::White,
                background: Color::Reset,
                accent: Color::White,
                muted: Color::DarkGray,
            },
            ThemePreset::Mono => Palette {
                foreground: Color::Gray,
                background: Color::Black,
                accent: Color::White,
                muted: Color::DarkGray,
            },
            ThemePreset::Custom => Palette {
                foreground: parse_color_value(&self.foreground_color)
                    .unwrap_or_else(|_| rgb(215, 215, 215)),
                background: parse_color_value(&self.background_color).unwrap_or(Color::Black),
                accent: parse_color_value(&self.accent_color)
                    .unwrap_or_else(|_| rgb(0, 215, 215)),
                muted: parse_color_value(&self.muted_color)
                    .unwrap_or_else(|_| rgb(95, 95, 95)),
            },
        }
    }

    pub fn panel_style(&self) -> Style {
        let p = self.palette();
        Style::default().fg(p.foreground).bg(p.background)
    }

    pub fn header_style(&self) -> Style {
        let p = self.palette();
        Style::default()
            .fg(p.accent)
            .bg(p.background)
            .add_modifier(Modifier::BOLD)
    }

    pub fn muted_style(&self) -> Style {
        let p = self.palette();
        Style::default().fg(p.muted).bg(p.background)
    }

    pub fn active_style(&self) -> Style {
        let p = self.palette();
        Style::default()
            .fg(contrast_color(p.accent))
            .bg(p.accent)
            .add_modifier(Modifier::BOLD)
    }

    pub fn border_color_style(&self, active: bool) -> Style {
        let p = self.palette();
        Style::default().fg(if active { p.accent } else { p.muted })
    }

    /// Returns the path where the config file is stored.
    pub fn config_path() -> PathBuf {
        config_path()
    }

    pub fn theme_label(&self) -> &'static str {
        self.theme.label()
    }
}

// ---------------------------------------------------------------------------
// Color parsing
// ---------------------------------------------------------------------------

pub fn parse_color_value(input: &str) -> Result<Color> {
    let v = input.trim();
    if v.is_empty() {
        return Err(anyhow!("Color cannot be empty"));
    }

    if v.eq_ignore_ascii_case("reset") || v.eq_ignore_ascii_case("transparent") {
        return Ok(Color::Reset);
    }

    let named = match v.to_ascii_lowercase().as_str() {
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "gray" | "grey" => Some(Color::Gray),
        "darkgray" | "darkgrey" => Some(Color::DarkGray),
        "lightred" => Some(Color::LightRed),
        "lightgreen" => Some(Color::LightGreen),
        "lightyellow" => Some(Color::LightYellow),
        "lightblue" => Some(Color::LightBlue),
        "lightmagenta" => Some(Color::LightMagenta),
        "lightcyan" => Some(Color::LightCyan),
        "white" => Some(Color::White),
        _ => None,
    };
    if let Some(c) = named {
        return Ok(c);
    }

    let hex = v.strip_prefix('#').unwrap_or(v);
    let (r, g, b) = match hex.len() {
        3 => (
            expand_nibble(&hex[0..1])?,
            expand_nibble(&hex[1..2])?,
            expand_nibble(&hex[2..3])?,
        ),
        6 => (
            u8::from_str_radix(&hex[0..2], 16).context("Invalid red channel")?,
            u8::from_str_radix(&hex[2..4], 16).context("Invalid green channel")?,
            u8::from_str_radix(&hex[4..6], 16).context("Invalid blue channel")?,
        ),
        _ => return Err(anyhow!("Use #RGB or #RRGGBB color codes")),
    };
    Ok(Color::Rgb(r, g, b))
}

fn expand_nibble(v: &str) -> Result<u8> {
    let n = u8::from_str_radix(v, 16).context("Invalid color nibble")?;
    Ok(n * 17)
}

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

fn contrast_color(c: Color) -> Color {
    if color_luma(c) > 150 { Color::Black } else { Color::White }
}

fn color_luma(c: Color) -> u8 {
    let (r, g, b) = match c {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Reset | Color::Black => (0, 0, 0),
        Color::Red => (205, 49, 49),
        Color::Green => (13, 188, 121),
        Color::Yellow => (229, 229, 16),
        Color::Blue => (36, 114, 200),
        Color::Magenta => (188, 63, 188),
        Color::Cyan => (17, 168, 205),
        Color::Gray => (204, 204, 204),
        Color::DarkGray => (128, 128, 128),
        Color::LightRed => (255, 85, 85),
        Color::LightGreen => (35, 209, 139),
        Color::LightYellow => (245, 245, 67),
        Color::LightBlue => (59, 142, 234),
        Color::LightMagenta => (214, 112, 214),
        Color::LightCyan => (41, 184, 219),
        Color::White => (255, 255, 255),
        _ => (204, 204, 204),
    };
    ((r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000) as u8
}

// ---------------------------------------------------------------------------
// Path helpers (single source of truth — no duplicate free function)
// ---------------------------------------------------------------------------

/// Returns the path to the config file.
pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// Returns the directory that holds all Cabin config files.
pub fn config_dir() -> PathBuf {
    if let Some(base) = BaseDirs::new() {
        return base.config_dir().join("Cabin");
    }
    Path::new(".").join("Cabin")
}

fn home_dir() -> PathBuf {
    BaseDirs::new()
        .map(|b| b.home_dir().to_path_buf())
        .unwrap_or_else(|| Path::new(".").to_path_buf())
}
