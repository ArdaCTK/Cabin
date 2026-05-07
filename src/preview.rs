use std::path::{Path, PathBuf};

use image::ImageReader;
use ratatui::layout::Rect;
use ratatui_image::{picker::Picker, protocol::Protocol, FilterType, Resize};

pub struct ImagePreview {
    pub path: PathBuf,
    pub area: Rect,
    pub protocol: Option<Protocol>,
    pub error: Option<String>,
}

impl ImagePreview {
    pub fn matches(&self, path: &Path, area: Rect) -> bool {
        self.path == path && self.area == area
    }
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
    if area.width == 0 || area.height == 0 {
        return ImagePreview {
            path: path.to_path_buf(),
            area,
            protocol: None,
            error: Some(String::from("Preview area is too small.")),
        };
    }

    let reader = match ImageReader::open(path) {
        Ok(reader) => reader,
        Err(err) => {
            return ImagePreview {
                path: path.to_path_buf(),
                area,
                protocol: None,
                error: Some(format!("Unable to open image: {err}")),
            };
        }
    };

    let image = match reader.decode() {
        Ok(image) => image,
        Err(err) => {
            return ImagePreview {
                path: path.to_path_buf(),
                area,
                protocol: None,
                error: Some(format!("Unable to decode image: {err}")),
            };
        }
    };

    match picker.new_protocol(image, area, Resize::Fit(Some(FilterType::Lanczos3))) {
        Ok(protocol) => ImagePreview {
            path: path.to_path_buf(),
            area,
            protocol: Some(protocol),
            error: None,
        },
        Err(err) => ImagePreview {
            path: path.to_path_buf(),
            area,
            protocol: None,
            error: Some(format!("Image preview error: {err}")),
        },
    }
}
