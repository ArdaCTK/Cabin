use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use directories_next::BaseDirs;
use ratatui::{
    style::{Color, Modifier, Style},
    widgets::BorderType,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreset {
    Dark,
    Light,
    Minimal,
    Mono,
}

impl ThemePreset {
    pub const ALL: [Self; 4] = [Self::Dark, Self::Light, Self::Minimal, Self::Mono];

    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::Minimal => "Minimal",
            Self::Mono => "Mono",
        }
    }

    pub fn next(self) -> Self {
        let index = Self::ALL.iter().position(|item| *item == self).unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        let index = Self::ALL.iter().position(|item| *item == self).unwrap_or(0);
        Self::ALL[(index + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    pub fn text_color(self) -> Color {
        match self {
            Self::Dark => Color::Gray,
            Self::Light => Color::Black,
            Self::Minimal => Color::White,
            Self::Mono => Color::Gray,
        }
    }

    pub fn muted_color(self) -> Color {
        match self {
            Self::Dark => Color::DarkGray,
            Self::Light => Color::DarkGray,
            Self::Minimal => Color::DarkGray,
            Self::Mono => Color::DarkGray,
        }
    }

    pub fn background_color(self) -> Color {
        match self {
            Self::Dark => Color::Black,
            Self::Light => Color::White,
            Self::Minimal => Color::Reset,
            Self::Mono => Color::Black,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccentColor {
    Cyan,
    Blue,
    Green,
    Amber,
    Magenta,
    White,
}

impl AccentColor {
    pub const ALL: [Self; 6] = [
        Self::Cyan,
        Self::Blue,
        Self::Green,
        Self::Amber,
        Self::Magenta,
        Self::White,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Cyan => "Cyan",
            Self::Blue => "Blue",
            Self::Green => "Green",
            Self::Amber => "Amber",
            Self::Magenta => "Magenta",
            Self::White => "White",
        }
    }

    pub fn next(self) -> Self {
        let index = Self::ALL.iter().position(|item| *item == self).unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        let index = Self::ALL.iter().position(|item| *item == self).unwrap_or(0);
        Self::ALL[(index + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    pub fn color(self) -> Color {
        match self {
            Self::Cyan => Color::Cyan,
            Self::Blue => Color::Blue,
            Self::Green => Color::Green,
            Self::Amber => Color::Yellow,
            Self::Magenta => Color::Magenta,
            Self::White => Color::White,
        }
    }

    pub fn on_color(self) -> Color {
        match self {
            Self::Amber | Self::White => Color::Black,
            _ => Color::White,
        }
    }
}

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
        let index = Self::ALL.iter().position(|item| *item == self).unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        let index = Self::ALL.iter().position(|item| *item == self).unwrap_or(0);
        Self::ALL[(index + Self::ALL.len() - 1) % Self::ALL.len()]
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
        let index = Self::ALL.iter().position(|item| *item == self).unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        let index = Self::ALL.iter().position(|item| *item == self).unwrap_or(0);
        Self::ALL[(index + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CabinConfig {
    pub theme: ThemePreset,
    pub accent_color: AccentColor,
    pub border_style: BorderPreset,
    pub panel_layout: PanelLayout,
    pub show_footer_tips: bool,
    pub show_hidden: bool,
}

impl Default for CabinConfig {
    fn default() -> Self {
        Self {
            theme: ThemePreset::Dark,
            accent_color: AccentColor::Cyan,
            border_style: BorderPreset::Rounded,
            panel_layout: PanelLayout::Balanced,
            show_footer_tips: true,
            show_hidden: false,
        }
    }
}

impl CabinConfig {
    pub fn load_or_default() -> (Self, Option<String>, bool) {
        let path = config_path();
        match fs::read_to_string(&path) {
            Ok(contents) => match toml::from_str::<Self>(&contents) {
                Ok(config) => (config, None, false),
                Err(err) => (
                    Self::default(),
                    Some(format!(
                        "Could not parse config.toml, using defaults: {err}"
                    )),
                    false,
                ),
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => (Self::default(), None, true),
            Err(err) => (
                Self::default(),
                Some(format!("Could not read config.toml, using defaults: {err}")),
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

    pub fn panel_style(&self) -> Style {
        Style::default()
            .fg(self.theme.text_color())
            .bg(self.theme.background_color())
    }

    pub fn header_style(&self) -> Style {
        Style::default()
            .fg(self.accent_color.color())
            .bg(self.theme.background_color())
            .add_modifier(Modifier::BOLD)
    }

    pub fn muted_style(&self) -> Style {
        Style::default()
            .fg(self.theme.muted_color())
            .bg(self.theme.background_color())
    }

    pub fn active_style(&self) -> Style {
        Style::default()
            .fg(self.accent_color.on_color())
            .bg(self.accent_color.color())
            .add_modifier(Modifier::BOLD)
    }

    pub fn border_color_style(&self, active: bool) -> Style {
        Style::default().fg(if active {
            self.accent_color.color()
        } else {
            self.theme.muted_color()
        })
    }

    pub fn config_path() -> PathBuf {
        config_path()
    }
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn config_dir() -> PathBuf {
    if let Some(base) = BaseDirs::new() {
        return base.config_dir().join("Cabin");
    }
    Path::new(".").join("Cabin")
}
