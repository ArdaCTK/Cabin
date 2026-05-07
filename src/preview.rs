use std::path::{Path, PathBuf};

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
