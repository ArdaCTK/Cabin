use std::path::Path;

use anyhow::{Context, Result};
use image::imageops::FilterType;
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
};

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

pub fn build_image_lines(path: &Path, area: Rect) -> Result<Vec<Line<'static>>> {
    if area.width == 0 || area.height == 0 {
        return Ok(Vec::new());
    }

    let image = image::open(path)
        .with_context(|| format!("Unable to open image {}", path.display()))?
        .to_rgba8();
    let (src_w, src_h) = (image.width(), image.height());

    if src_w == 0 || src_h == 0 {
        return Ok(Vec::new());
    }

    let target_w = area.width.max(1) as u32;
    let target_h = area.height.max(1) as u32 * 2;
    let scale_w = target_w as f32 / src_w as f32;
    let scale_h = target_h as f32 / src_h as f32;
    let scale = scale_w.min(scale_h);

    let mut resized_w = ((src_w as f32 * scale).round() as u32).max(1);
    let mut resized_h = ((src_h as f32 * scale).round() as u32).max(2);
    if resized_h % 2 == 1 {
        resized_h += 1;
    }

    let resized = image::imageops::resize(&image, resized_w, resized_h, FilterType::Triangle);
    resized_w = resized.width();
    resized_h = resized.height();

    let cells_w = resized_w as u16;
    let cells_h = (resized_h / 2) as u16;

    let pad_x = area.width.saturating_sub(cells_w) / 2;
    let pad_y = area.height.saturating_sub(cells_h) / 2;
    let right_pad = area.width.saturating_sub(pad_x + cells_w);

    let mut lines = Vec::new();

    for _ in 0..pad_y {
        lines.push(blank_line(area.width));
    }

    for row in 0..cells_h {
        let mut spans = Vec::new();
        if pad_x > 0 {
            spans.push(Span::raw(" ".repeat(pad_x as usize)));
        }

        for col in 0..cells_w {
            let top = resized.get_pixel(col as u32, (row as u32) * 2);
            let bottom = resized.get_pixel(col as u32, (row as u32) * 2 + 1);
            spans.push(Span::styled(
                "▀",
                Style::default()
                    .fg(rgb_to_color(top.0))
                    .bg(rgb_to_color(bottom.0)),
            ));
        }

        if right_pad > 0 {
            spans.push(Span::raw(" ".repeat(right_pad as usize)));
        }

        lines.push(Line::from(spans));
    }

    let remaining = area.height.saturating_sub(pad_y + cells_h);
    for _ in 0..remaining {
        lines.push(blank_line(area.width));
    }

    Ok(lines)
}

fn blank_line(width: u16) -> Line<'static> {
    Line::from(" ".repeat(width as usize))
}

fn rgb_to_color(rgba: [u8; 4]) -> Color {
    let alpha = rgba[3] as u16;
    let blend = |channel: u8| -> u8 {
        ((channel as u16 * alpha) / 255) as u8
    };

    Color::Rgb(blend(rgba[0]), blend(rgba[1]), blend(rgba[2]))
}
