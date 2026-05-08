use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use image::ImageReader;
use lofty::{
    file::{AudioFile, TaggedFileExt},
    read_from_path,
    tag::Accessor,
};
use ratatui::layout::Rect;
use ratatui_image::{picker::Picker, protocol::Protocol, FilterType, Resize};
use remeta::VideoMetadata;

// ---------------------------------------------------------------------------
// Cache keys
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImagePreviewKey {
    pub path: PathBuf,
    pub width: u16,
    pub height: u16,
}

impl ImagePreviewKey {
    pub fn new(path: PathBuf, area: Rect) -> Self {
        Self { path, width: area.width, height: area.height }
    }
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VideoPreviewKey {
    pub path: PathBuf,
}

impl VideoPreviewKey {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AudioPreviewKey {
    pub path: PathBuf,
}

impl AudioPreviewKey {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

// ---------------------------------------------------------------------------
// Preview result types
// ---------------------------------------------------------------------------

pub struct ImagePreview {
    pub key: ImagePreviewKey,
    pub protocol: Option<Protocol>,
    pub error: Option<String>,
}

pub struct TextPreview {
    pub lines: Vec<String>,
    pub error: Option<String>,
}

pub struct VideoPreview {
    pub lines: Vec<String>,
    pub error: Option<String>,
}

pub struct AudioPreview {
    pub lines: Vec<String>,
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// File-type detection
// ---------------------------------------------------------------------------

pub fn is_supported_image(path: &Path) -> bool {
    matches_ext(path, &["png", "jpg", "jpeg", "webp", "bmp", "gif"])
}

pub fn is_supported_text(path: &Path) -> bool {
    matches_ext(
        path,
        &[
            "txt", "md", "rs", "toml", "json", "yaml", "yml", "html", "css", "js", "ts", "py",
            "rpy", "xml", "csv", "log", "ini", "sh", "bat", "c", "cpp", "h", "hpp", "go", "rb",
            "java", "kt", "swift", "lua", "php", "sql",
        ],
    )
}

pub fn is_supported_video(path: &Path) -> bool {
    matches_ext(path, &["mp4", "mkv", "webm", "mov", "avi"])
}

pub fn is_supported_audio(path: &Path) -> bool {
    matches_ext(path, &["mp3", "flac", "wav", "ogg", "m4a", "aac"])
}

pub fn is_supported_pdf(path: &Path) -> bool {
    matches_ext(path, &["pdf"])
}

fn matches_ext(path: &Path, exts: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let lower = e.to_ascii_lowercase();
            exts.contains(&lower.as_str())
        })
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Image preview — multi-thread worker
// ---------------------------------------------------------------------------

/// Job sent from the main thread to an image worker.
pub struct ImageJob {
    pub path: PathBuf,
    pub area: Rect,
}

/// Spawns a pool of image-decode workers.  The number of threads is half the
/// available CPU count, clamped to [2, 4].  Using Triangle instead of Lanczos3
/// and pre-downscaling to terminal resolution makes decodes ~3–10x faster with
/// no visible quality difference at terminal pixel density.
pub fn spawn_image_workers(
    picker: Picker,
) -> (
    std::sync::mpsc::Sender<ImageJob>,
    std::sync::mpsc::Receiver<ImagePreview>,
) {
    use std::sync::mpsc;

    let (job_tx, job_rx) = mpsc::channel::<ImageJob>();
    let (result_tx, result_rx) = mpsc::channel::<ImagePreview>();

    let job_rx = Arc::new(Mutex::new(job_rx));
    let picker = Arc::new(picker);

    let worker_count = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .div_ceil(2)
        .clamp(2, 4);

    for _ in 0..worker_count {
        let job_rx = Arc::clone(&job_rx);
        let result_tx = result_tx.clone();
        let picker = Arc::clone(&picker);

        thread::spawn(move || loop {
            let job = {
                let Ok(rx) = job_rx.lock() else { break };
                match rx.recv() {
                    Ok(j) => j,
                    Err(_) => break,
                }
            };
            let preview = build_image_preview(&picker, &job.path, job.area);
            if result_tx.send(preview).is_err() {
                break;
            }
        });
    }

    (job_tx, result_rx)
}

/// Decodes the image at `path` and produces a terminal-compatible protocol
/// object sized to `area`.  Pre-downscales to terminal resolution before
/// calling ratatui-image so we never push multi-megapixel bitmaps through the
/// colour-quantisation step.
pub fn build_image_preview(picker: &Picker, path: &Path, area: Rect) -> ImagePreview {
    let key = ImagePreviewKey::new(path.to_path_buf(), area);

    if area.width == 0 || area.height == 0 {
        return ImagePreview {
            key,
            protocol: None,
            error: Some(String::from("Preview area is too small.")),
        };
    }

    let reader = match ImageReader::open(path).and_then(|r| r.with_guessed_format()) {
        Ok(r) => r,
        Err(e) => {
            return ImagePreview { key, protocol: None, error: Some(e.to_string()) };
        }
    };

    let image = match reader.decode() {
        Ok(img) => img,
        Err(e) => {
            return ImagePreview { key, protocol: None, error: Some(e.to_string()) };
        }
    };

    // Pre-downscale to the maximum pixels the terminal cell grid can display.
    // Typical terminal cell is ~8×16 px.  Pushing a full 4K image through
    // ratatui-image's quantiser is the main cause of slow previews.
    let target_w = (area.width as u32).saturating_mul(10).max(64);
    let target_h = (area.height as u32).saturating_mul(20).max(64);
    let image = image.thumbnail(target_w, target_h);

    // Triangle resampler: ~3x faster than Lanczos3 at terminal pixel density.
    match picker.new_protocol(image, area, Resize::Fit(Some(FilterType::Triangle))) {
        Ok(proto) => ImagePreview { key, protocol: Some(proto), error: None },
        Err(e) => ImagePreview { key, protocol: None, error: Some(e.to_string()) },
    }
}

// ---------------------------------------------------------------------------
// Text preview — background thread worker
// ---------------------------------------------------------------------------

/// Spawns a single worker that reads text and PDF files off the main thread.
/// Automatically routes to `build_pdf_preview` for `.pdf` files and
/// `build_text_preview` for everything else.
pub fn spawn_text_worker() -> (
    std::sync::mpsc::Sender<PathBuf>,
    std::sync::mpsc::Receiver<(PathBuf, TextPreview)>,
) {
    use std::sync::mpsc;

    let (job_tx, job_rx) = mpsc::channel::<PathBuf>();
    let (result_tx, result_rx) = mpsc::channel::<(PathBuf, TextPreview)>();

    thread::spawn(move || {
        while let Ok(path) = job_rx.recv() {
            let preview = if is_supported_pdf(&path) {
                // Extract text from the first 10 pages of the PDF.
                build_pdf_preview(&path, 10)
            } else {
                build_text_preview(&path, 500, 256 * 1024)
            };
            if result_tx.send((path, preview)).is_err() {
                break;
            }
        }
    });

    (job_tx, result_rx)
}

/// Reads up to `max_lines` lines / `max_bytes` bytes from a text file.
/// Never called on the main thread — dispatched via `spawn_text_worker`.
pub fn build_text_preview(path: &Path, max_lines: usize, max_bytes: usize) -> TextPreview {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            return TextPreview {
                lines: Vec::new(),
                error: Some(format!("Unable to open file: {e}")),
            }
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
        let n = match reader.read_until(b'\n', &mut buf) {
            Ok(n) => n,
            Err(e) => {
                return TextPreview {
                    lines: Vec::new(),
                    error: Some(format!("Unable to read file: {e}")),
                }
            }
        };
        if n == 0 {
            break;
        }
        total_bytes = total_bytes.saturating_add(n);
        if buf.ends_with(b"\n") { buf.pop(); }
        if buf.ends_with(b"\r") { buf.pop(); }
        lines.push(String::from_utf8_lossy(&buf).to_string());
    }

    if truncated {
        lines.push(format!("... truncated after {} lines ...", lines.len()));
    }

    TextPreview { lines, error: None }
}

// ---------------------------------------------------------------------------
// PDF preview
// ---------------------------------------------------------------------------

/// Extracts plain text from the first `max_pages` pages of a PDF.
pub fn build_pdf_preview(path: &Path, max_pages: usize) -> TextPreview {
    let doc = match lopdf::Document::load(path) {
        Ok(d) => d,
        Err(e) => {
            return TextPreview {
                lines: vec![
                    String::from("Type: PDF"),
                    format!("Could not parse PDF: {e}"),
                    format!("Path: {}", path.display()),
                ],
                error: Some(e.to_string()),
            }
        }
    };

    let total_pages = doc.get_pages().len();
    let mut lines = vec![
        String::from("Type: PDF"),
        format!("Pages: {total_pages}"),
        format!("Path: {}", path.display()),
        String::new(),
    ];

    let page_nums: Vec<u32> = doc.get_pages().keys().cloned().collect();
    for &page_num in page_nums.iter().take(max_pages) {
        match doc.extract_text(&[page_num]) {
            Ok(text) => {
                lines.push(format!("── Page {page_num} ──"));
                for line in text.lines().take(40) {
                    if !line.trim().is_empty() {
                        lines.push(line.to_string());
                    }
                }
                lines.push(String::new());
            }
            Err(e) => {
                lines.push(format!("── Page {page_num}: text extraction failed ({e}) ──"));
            }
        }
    }

    if total_pages > max_pages {
        lines.push(format!("... {} more page(s) not shown ...", total_pages - max_pages));
    }

    TextPreview { lines, error: None }
}

// ---------------------------------------------------------------------------
// Video preview
// ---------------------------------------------------------------------------

pub fn build_video_preview(path: &Path) -> VideoPreview {
    let file_name = file_name_str(path);

    let metadata = match VideoMetadata::from_file(path) {
        Ok(m) => m,
        Err(e) => {
            return VideoPreview {
                lines: vec![
                    String::from("Type: Video"),
                    format!("Filename: {file_name}"),
                    format!("Path: {}", path.display()),
                    format!("Metadata unavailable: {e}"),
                ],
                error: Some(e.to_string()),
            }
        }
    };

    let mut lines = vec![
        String::from("Type: Video"),
        format!("Filename: {file_name}"),
        format!(
            "Duration: {}",
            metadata.duration_ms.map(fmt_duration_ms).unwrap_or_else(|| String::from("Unknown"))
        ),
        format!(
            "Resolution: {}",
            metadata
                .resolution
                .map(|(w, h)| format!("{w} x {h}"))
                .unwrap_or_else(|| String::from("Unknown"))
        ),
        format!(
            "Codec: {}",
            metadata.codec.unwrap_or_else(|| String::from("Unknown"))
        ),
    ];

    if let Some(title) = metadata.title {
        lines.push(format!("Title: {title}"));
    }
    if let Some(director) = metadata.director {
        lines.push(format!("Director: {director}"));
    }

    VideoPreview { lines, error: None }
}

// ---------------------------------------------------------------------------
// Audio preview
// ---------------------------------------------------------------------------

pub fn build_audio_preview(path: &Path) -> AudioPreview {
    let file_name = file_name_str(path);

    let tagged = match read_from_path(path) {
        Ok(f) => f,
        Err(e) => {
            return AudioPreview {
                lines: vec![
                    String::from("Type: Audio"),
                    format!("Filename: {file_name}"),
                    format!("Path: {}", path.display()),
                    format!("Metadata unavailable: {e}"),
                ],
                error: Some(e.to_string()),
            }
        }
    };

    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let props = tagged.properties();

    let bitrate = props
        .audio_bitrate()
        .or_else(|| props.overall_bitrate())
        .map(|v| format!("{v} kbps"))
        .unwrap_or_else(|| String::from("Unknown"));

    let lines = vec![
        String::from("Type: Audio"),
        format!("Filename: {file_name}"),
        format!(
            "Title: {}",
            tag.and_then(|t| t.title()).map(|v| v.to_string()).unwrap_or_else(|| String::from("Unknown"))
        ),
        format!(
            "Artist: {}",
            tag.and_then(|t| t.artist()).map(|v| v.to_string()).unwrap_or_else(|| String::from("Unknown"))
        ),
        format!(
            "Album: {}",
            tag.and_then(|t| t.album()).map(|v| v.to_string()).unwrap_or_else(|| String::from("Unknown"))
        ),
        format!("Duration: {}", fmt_duration(props.duration())),
        format!("Bitrate: {bitrate}"),
        format!("Path: {}", path.display()),
    ];

    AudioPreview { lines, error: None }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn file_name_str(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("(unknown)")
        .to_string()
}

fn fmt_duration_ms(ms: u64) -> String {
    let s = ms / 1000;
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let s = s % 60;
    if h > 0 { format!("{h:02}:{m:02}:{s:02}") } else { format!("{m:02}:{s:02}") }
}

fn fmt_duration(d: Duration) -> String {
    let s = d.as_secs();
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let s = s % 60;
    if h > 0 { format!("{h:02}:{m:02}:{s:02}") } else { format!("{m:02}:{s:02}") }
}
