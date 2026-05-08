use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};
use ratatui_image::Image as TerminalImage;

use crate::app::{App, Dialog, EntryKind, Panel};
use crate::config::PanelLayout;
use crate::preview::{is_supported_image, is_supported_pdf, is_supported_text};

// ---------------------------------------------------------------------------
// Top-level draw entry point
// ---------------------------------------------------------------------------

pub fn draw(frame: &mut Frame<'_>, app: &mut App) {
    // Poll background workers before rendering so any finished jobs are
    // reflected in the same frame they complete.
    app.poll_image_previews();
    app.poll_text_previews();

    let size = frame.area();

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header bar
            Constraint::Min(5),    // three-panel body
            Constraint::Length(2), // footer / status
        ])
        .split(size);

    draw_header(frame, outer[0], app);
    draw_body(frame, outer[1], app);
    draw_footer(frame, outer[2], app);

    // Overlays rendered last so they appear on top.
    if app.help_visible {
        draw_help(frame, centered_rect(60, 60, size), app);
    }
    if app.settings_visible {
        draw_settings(frame, centered_rect(64, 72, size), app);
    }
    if let Some(dialog) = app.dialog.as_ref() {
        draw_dialog(frame, centered_rect(58, 40, size), dialog, app);
    }
}

// ---------------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------------

fn draw_header(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let sort_label = format!(
        " [sort: {}{}]",
        app.config.sort_field.label(),
        if app.config.sort_descending { " ▼" } else { " ▲" }
    );

    let paragraph = Paragraph::new(Line::from(vec![
        Span::styled("Cabin", app.config.header_style()),
        Span::raw("  "),
        Span::styled(app.current_dir.display().to_string(), app.config.panel_style()),
        Span::styled(sort_label, app.config.muted_style()),
    ]))
    .style(app.config.panel_style());

    frame.render_widget(paragraph, area);
}

// ---------------------------------------------------------------------------
// Three-panel body
// ---------------------------------------------------------------------------

fn draw_body(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    let widths = panel_widths(app.config.panel_layout, area.width);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(widths)
        .split(area);

    draw_places(frame, columns[0], app);
    draw_contents(frame, columns[1], app);
    draw_preview(frame, columns[2], app);
}

fn panel_widths(layout: PanelLayout, total_width: u16) -> [Constraint; 3] {
    let wide = total_width >= 90;
    match layout {
        PanelLayout::Classic => {
            if wide {
                [Constraint::Length(24), Constraint::Min(30), Constraint::Length(34)]
            } else {
                [Constraint::Length(18), Constraint::Min(24), Constraint::Length(28)]
            }
        }
        PanelLayout::Balanced => {
            if wide {
                [Constraint::Length(22), Constraint::Min(30), Constraint::Length(36)]
            } else {
                [Constraint::Length(19), Constraint::Min(24), Constraint::Length(29)]
            }
        }
        PanelLayout::PreviewFocus => [
            Constraint::Percentage(15),
            Constraint::Percentage(25),
            Constraint::Percentage(60),
        ],
        PanelLayout::ContentsFocus => {
            if wide {
                [Constraint::Length(26), Constraint::Min(36), Constraint::Length(28)]
            } else {
                [Constraint::Length(18), Constraint::Min(28), Constraint::Length(26)]
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Places panel
// ---------------------------------------------------------------------------

fn draw_places(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let active = app.active_panel == Panel::Places;
    let block = panel_block("Places", active, app);
    let items: Vec<ListItem<'_>> = app
        .places
        .iter()
        .map(|p| ListItem::new(Line::from(app.place_list_label(p))))
        .collect();
    let list = List::new(items)
        .style(app.config.panel_style())
        .block(block)
        .highlight_style(app.config.active_style())
        .highlight_symbol("> ");

    let mut state = list_state(Some(app.places_selected), app.places.len());
    frame.render_stateful_widget(list, area, &mut state);
}

// ---------------------------------------------------------------------------
// Contents panel
// ---------------------------------------------------------------------------

fn draw_contents(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let active = app.active_panel == Panel::Contents;

    // When the cursor is in Places, show that place's children in the
    // Contents panel instead of the current directory entries.
    let entries = if app.active_panel == Panel::Places {
        &app.hovered_place_entries
    } else {
        &app.entries
    };

    let title = if app.active_panel == Panel::Places {
        "Contents"
    } else {
        match &app.contents_mode {
            crate::app::ContentsMode::SearchCurrent { .. } => "Search (current)",
            crate::app::ContentsMode::SearchRecursive { .. } => "Search (recursive)",
            _ => "Contents",
        }
    };

    let block = panel_block(title, active, app);

    if app.active_panel == Panel::Places {
        if let Some(err) = app.hovered_place_error.as_ref() {
            let p = Paragraph::new(Line::from(err.clone()))
                .style(app.config.panel_style())
                .block(block)
                .wrap(Wrap { trim: false });
            frame.render_widget(p, area);
            return;
        }
    }

    let items: Vec<ListItem<'_>> = if entries.is_empty() {
        vec![ListItem::new(Line::from("(empty)"))]
    } else {
        entries
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                let label = app.entry_list_label(entry);
                // Highlight multi-selected items with a distinct marker.
                let is_multi = app.selected_indices.contains(&i);
                let style = if is_multi {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(Line::from(Span::styled(
                    if is_multi { format!("* {label}") } else { label },
                    style,
                )))
            })
            .collect()
    };

    let selected = if app.active_panel == Panel::Places {
        None
    } else {
        Some(app.contents_selected)
    };

    let list = List::new(items)
        .style(app.config.panel_style())
        .block(block)
        .highlight_style(app.config.active_style())
        .highlight_symbol("> ");

    let mut state = list_state(selected, entries.len());
    frame.render_stateful_widget(list, area, &mut state);
}

// ---------------------------------------------------------------------------
// Preview panel
// ---------------------------------------------------------------------------

fn draw_preview(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    let active = app.active_panel == Panel::Preview;
    let block = panel_block("Preview", active, app);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    // Try to render a rich preview for the selected entry.
    if let Some(entry) = app.entries.get(app.contents_selected) {
        let path = entry.path.clone();

        if is_supported_image(&path) {
            draw_image_preview(frame, inner, app, &path);
            return;
        }
        if is_supported_text(&path) || is_supported_pdf(&path) {
            draw_text_or_pdf_preview(frame, inner, app, &path);
            return;
        }
    }

    // Fallback: plain metadata / status lines.
    let lines = app.preview.lines.clone();
    let text = Text::from(lines.into_iter().map(Line::from).collect::<Vec<_>>());
    let paragraph = Paragraph::new(text)
        .style(app.config.panel_style())
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}

/// Renders an image with a short metadata caption above it.
fn draw_image_preview(frame: &mut Frame<'_>, inner: Rect, app: &mut App, path: &std::path::Path) {
    let caption_lines = app.preview.lines.clone();
    let caption_height = (caption_lines.len().min(5)) as u16;

    if caption_height > 0 {
        let caption_area = Rect { height: caption_height, ..inner };
        let caption = Paragraph::new(
            caption_lines
                .iter()
                .take(caption_height as usize)
                .cloned()
                .map(Line::from)
                .collect::<Vec<_>>(),
        )
        .style(app.config.panel_style())
        .wrap(Wrap { trim: false });
        frame.render_widget(caption, caption_area);
    }

    let image_area = Rect {
        y: inner.y + caption_height,
        height: inner.height.saturating_sub(caption_height),
        ..inner
    };

    if image_area.height == 0 {
        return;
    }

    app.last_image_area = Some(image_area);

    if let Some(preview) = app.cached_image_preview(path, image_area) {
        if let Some(err) = preview.error.as_ref() {
            let p = Paragraph::new(Line::from(err.clone()))
                .style(app.config.panel_style())
                .wrap(Wrap { trim: false });
            frame.render_widget(p, image_area);
        } else if let Some(proto) = preview.protocol.as_ref() {
            frame.render_widget(TerminalImage::new(proto), image_area);
        }
    } else {
        let p = Paragraph::new(Line::from("Preparing image preview…"))
            .style(app.config.panel_style())
            .wrap(Wrap { trim: false });
        frame.render_widget(p, image_area);
    }
}

/// Renders text or PDF content with a short caption header.
fn draw_text_or_pdf_preview(
    frame: &mut Frame<'_>,
    inner: Rect,
    app: &mut App,
    path: &std::path::Path,
) {
    let caption_lines = app.preview.lines.clone();
    let caption_height = (caption_lines.len().min(5)) as u16;

    if caption_height > 0 {
        let caption_area = Rect { height: caption_height, ..inner };
        let caption = Paragraph::new(
            caption_lines
                .iter()
                .take(caption_height as usize)
                .cloned()
                .map(Line::from)
                .collect::<Vec<_>>(),
        )
        .style(app.config.panel_style())
        .wrap(Wrap { trim: false });
        frame.render_widget(caption, caption_area);
    }

    let text_area = Rect {
        y: inner.y + caption_height,
        height: inner.height.saturating_sub(caption_height),
        ..inner
    };

    if text_area.height == 0 {
        return;
    }

    if let Some(preview) = app.cached_text_preview(path) {
        if let Some(err) = preview.error.as_ref() {
            let p = Paragraph::new(Line::from(err.clone()))
                .style(app.config.panel_style())
                .wrap(Wrap { trim: false });
            frame.render_widget(p, text_area);
        } else {
            let max_scroll = preview
                .lines
                .len()
                .saturating_sub(text_area.height as usize) as u16;
            let scroll = app.preview_scroll.min(max_scroll);
            let content = Text::from(
                preview
                    .lines
                    .iter()
                    .cloned()
                    .map(Line::from)
                    .collect::<Vec<_>>(),
            );
            let p = Paragraph::new(content)
                .style(app.config.panel_style())
                .scroll((scroll, 0))
                .wrap(Wrap { trim: false });
            frame.render_widget(p, text_area);
        }
    } else {
        // Job has been dispatched but not yet complete — show a placeholder.
        let p = Paragraph::new(Line::from("Preparing preview…"))
            .style(app.config.panel_style())
            .wrap(Wrap { trim: false });
        frame.render_widget(p, text_area);
    }
}

// ---------------------------------------------------------------------------
// Footer
// ---------------------------------------------------------------------------

fn draw_footer(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    let message = app.status_message.clone().unwrap_or_else(|| String::from("Ready"));
    let perf = Line::from(app.performance_summary());

    let hint_spans: Vec<Span<'static>> = if app.config.show_footer_tips {
        vec![
            bold("q"), plain(" quit  "),
            bold("?"), plain(" help  "),
            bold("s"), plain(" settings  "),
            bold("Tab"), plain(" panel  "),
            bold("Enter"), plain(" open  "),
            bold("BS"), plain(" parent  "),
            bold("h"), plain(" hidden  "),
            bold("Space"), plain(" select  "),
            bold("c"), plain(" copy  "),
            bold("x"), plain(" cut  "),
            bold("p"), plain(" paste  "),
            bold("r"), plain(" rename  "),
            bold("d"), plain(" delete  "),
            bold("n"), plain(" new  "),
            bold("y"), plain(" path  "),
            bold("g"), plain(" jump  "),
            bold("Shift+S"), plain(" sort  "),
            bold("Ctrl+B"), plain(" bookmark  "),
            bold("/"), plain(" search  "),
            bold("Ctrl+F"), plain(" recursive  "),
            bold("F5"), plain(" refresh  |  "),
            Span::raw(message),
        ]
    } else {
        vec![Span::raw(message)]
    };

    let footer = Paragraph::new(vec![perf, Line::from(hint_spans)])
        .style(app.config.muted_style())
        .wrap(Wrap { trim: false });
    frame.render_widget(footer, area);
}

fn bold(s: &'static str) -> Span<'static> {
    Span::styled(s, Style::default().add_modifier(Modifier::BOLD))
}

fn plain(s: &'static str) -> Span<'static> {
    Span::raw(s)
}

// ---------------------------------------------------------------------------
// Help overlay
// ---------------------------------------------------------------------------

fn draw_help(frame: &mut Frame<'_>, area: Rect, app: &App) {
    frame.render_widget(Clear, area);
    let lines = vec![
        Line::from("q            Quit"),
        Line::from("?            Toggle help"),
        Line::from("s            Settings"),
        Line::from("Tab          Next panel"),
        Line::from("Shift+Tab    Previous panel"),
        Line::from("Up/Down j/k  Move selection"),
        Line::from("Enter        Open item"),
        Line::from("Backspace    Parent folder (exits search mode)"),
        Line::from("h            Toggle hidden files"),
        Line::from("Space        Toggle multi-select on item"),
        Line::from("c            Copy selected"),
        Line::from("x            Cut selected"),
        Line::from("p            Paste into current folder"),
        Line::from("r            Rename"),
        Line::from("d            Delete to Recycle Bin"),
        Line::from("n            New file"),
        Line::from("Shift+N      New folder"),
        Line::from("y            Copy path to clipboard"),
        Line::from("g            Jump to path"),
        Line::from("Shift+S      Cycle sort field"),
        Line::from("Ctrl+B       Add bookmark for current dir"),
        Line::from("/            Search current folder"),
        Line::from("Ctrl+F       Recursive search"),
        Line::from("F5           Refresh"),
        Line::from("Settings:    Left/Right cycle, Enter edit, Ctrl+R reset colors"),
    ];
    let p = Paragraph::new(lines)
        .style(app.config.panel_style())
        .block(panel_block("Help — press ? or Esc to close", true, app))
        .wrap(Wrap { trim: false });
    frame.render_widget(p, area);
}

// ---------------------------------------------------------------------------
// Settings overlay
// ---------------------------------------------------------------------------

fn draw_settings(frame: &mut Frame<'_>, area: Rect, app: &App) {
    frame.render_widget(Clear, area);

    let rows = app.settings_rows();
    let items: Vec<ListItem<'_>> = rows
        .iter()
        .map(|r| ListItem::new(Line::from(r.clone())))
        .collect();

    let list_area = Rect {
        height: area.height.saturating_sub(3),
        ..area
    };
    let list = List::new(items)
        .style(app.config.panel_style())
        .block(panel_block("Settings", true, app))
        .highlight_style(app.config.active_style())
        .highlight_symbol("> ");

    let mut state = list_state(Some(app.settings_selected), rows.len());
    frame.render_stateful_widget(list, list_area, &mut state);

    let tip_area = Rect {
        x: area.x.saturating_add(1),
        y: area.y + area.height.saturating_sub(3),
        width: area.width.saturating_sub(2),
        height: 3,
    };
    let tip = Paragraph::new(vec![
        Line::from("Up/Down: navigate   Left/Right: cycle value   Enter: open editor   Esc/S: close"),
        Line::from("Ctrl+R: reset colors to defaults"),
        Line::from(format!(
            "Config: {}",
            crate::config::CabinConfig::config_path().display()
        )),
    ])
    .style(app.config.muted_style())
    .wrap(Wrap { trim: false });
    frame.render_widget(tip, tip_area);
}

// ---------------------------------------------------------------------------
// Dialog overlay
// ---------------------------------------------------------------------------

fn draw_dialog(frame: &mut Frame<'_>, area: Rect, dialog: &Dialog, app: &App) {
    frame.render_widget(Clear, area);

    let (title, lines): (&str, Vec<Line<'_>>) = match dialog {
        Dialog::Input { title, value, .. } => (
            title.as_str(),
            vec![
                Line::from(vec![
                    Span::styled("Value: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw(value.clone()),
                    // Show a blinking cursor hint.
                    Span::styled("█", Style::default().add_modifier(Modifier::SLOW_BLINK)),
                ]),
                Line::from(""),
                Line::from("Enter: confirm   Esc: cancel   Backspace: delete"),
            ],
        ),
        Dialog::ConfirmDelete { label, .. } => (
            "Confirm delete",
            vec![
                Line::from(format!("Move \"{label}\" to Recycle Bin?")),
                Line::from(""),
                Line::from("Y / Enter: yes   N / Esc: no"),
            ],
        ),
        Dialog::Conflict { destination, choice } => (
            "Paste conflict",
            vec![
                Line::from(format!("Destination: {}", destination.display())),
                Line::from(""),
                Line::from(vec![
                    Span::raw("Choice: "),
                    Span::styled(choice.label(), Style::default().add_modifier(Modifier::BOLD)),
                ]),
                Line::from(""),
                Line::from("Left/Right: change   Enter: confirm   Esc: cancel"),
            ],
        ),
    };

    let p = Paragraph::new(lines)
        .style(app.config.panel_style())
        .block(panel_block(title, true, app))
        .wrap(Wrap { trim: false });
    frame.render_widget(p, area);
}

// ---------------------------------------------------------------------------
// Widget helpers
// ---------------------------------------------------------------------------

fn panel_block<'a>(title: &'a str, active: bool, app: &App) -> Block<'a> {
    Block::default()
        .border_type(app.config.border_style.border_type())
        .borders(Borders::ALL)
        .title(title)
        .style(app.config.panel_style())
        .border_style(app.config.border_color_style(active))
}

fn list_state(selected: Option<usize>, len: usize) -> ratatui::widgets::ListState {
    let mut state = ratatui::widgets::ListState::default();
    if len > 0 {
        state.select(selected.map(|i| i.min(len.saturating_sub(1))));
    }
    state
}

/// Returns a centred rectangle that is `px`% wide and `py`% tall of `r`.
fn centered_rect(px: u16, py: u16, r: Rect) -> Rect {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - py) / 2),
            Constraint::Percentage(py),
            Constraint::Percentage((100 - py) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - px) / 2),
            Constraint::Percentage(px),
            Constraint::Percentage((100 - px) / 2),
        ])
        .split(vert[1])[1]
}
