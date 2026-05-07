use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

use image::ImageReader;
use ratatui::layout::Rect;
use ratatui_image::{picker::Picker, protocol::Protocol, FilterType, Resize};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImagePreviewKey {
    pub path: PathBuf,
    pub width: u16,
    pub height: u16,
}

impl ImagePreviewKey {
    pub fn new(path: PathBuf, area: Rect) -> Self {
        Self {
            path,
            width: area.width,
            height: area.height,
        }
    }
}

pub struct ImagePreview {
    pub key: ImagePreviewKey,
    pub protocol: Option<Protocol>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextPreviewKey {
    pub path: PathBuf,
}

impl TextPreviewKey {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

pub struct TextPreview {
    pub lines: Vec<String>,
    pub error: Option<String>,
}

pub fn is_supported_image(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase()),
        Some(ext)
            if matches!(
                ext.as_str(),
                "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif"
            )
    )
}

pub fn is_supported_text(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase()),
        Some(ext)
            if matches!(
                ext.as_str(),
                "txt" | "md" | "rs" | "toml" | "json" | "yaml" | "yml" | "html" | "css" | "js" | "ts" | "py" | "rpy" | "xml" | "csv" | "log" | "ini"
            )
    )
}

pub fn build_text_preview(path: &Path, max_lines: usize, max_bytes: usize) -> TextPreview {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) => {
            return TextPreview {
                lines: Vec::new(),
                error: Some(format!("Unable to open file: {err}")),
            };
        }
    };

    let mut reader = BufReader::new(file);
    let mut lines = Vec::new();
    let mut buf = Vec::new();
    let mut total_bytes = 0usize;
    let mut truncated = false;

    loop {
        if lines.len() >= max_lines || total_bytes >= max_bytes {
            truncated = true;
            break;
        }

        buf.clear();
        let read = match reader.read_until(b'\n', &mut buf) {
            Ok(read) => read,
            Err(err) => {
                return TextPreview {
                    lines: Vec::new(),
                    error: Some(format!("Unable to read file: {err}")),
                };
            }
        };

        if read == 0 {
            break;
        }

        total_bytes = total_bytes.saturating_add(read);

        if buf.ends_with(b"\n") {
            buf.pop();
        }
        if buf.ends_with(b"\r") {
            buf.pop();
        }

        lines.push(String::from_utf8_lossy(&buf).to_string());
    }

    if truncated {
        lines.push(format!("... truncated after {} lines ...", lines.len()));
    }

    TextPreview {
        lines,
        error: None,
    }
}

pub fn build_image_preview(picker: &Picker, path: &Path, area: Rect) -> ImagePreview {
    let key = ImagePreviewKey::new(path.to_path_buf(), area);

    if area.width == 0 || area.height == 0 {
        return ImagePreview {
            key,
            protocol: None,
            error: Some(String::from("Preview area is too small.")),
        };
    }

    let reader = match ImageReader::open(path) {
        Ok(reader) => reader,
        Err(err) => {
            return ImagePreview {
                key,
                protocol: None,
                error: Some(format!("Unable to open image: {err}")),
            };
        }
    };

    let image = match reader.decode() {
        Ok(image) => image,
        Err(err) => {
            return ImagePreview {
                key,
                protocol: None,
                error: Some(format!("Unable to decode image: {err}")),
            };
        }
    };

    match picker.new_protocol(image, area, Resize::Fit(Some(FilterType::Lanczos3))) {
        Ok(protocol) => ImagePreview {
            key,
            protocol: Some(protocol),
            error: None,
        },
        Err(err) => ImagePreview {
            key,
            protocol: None,
            error: Some(format!("Image preview error: {err}")),
        },
    }
}
