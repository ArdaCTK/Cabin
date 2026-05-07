use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};
use ratatui_image::Image as TerminalImage;

use crate::app::{App, Dialog, Panel};
use crate::config::PanelLayout;
use crate::preview::{is_supported_image, is_supported_text};

pub fn draw(frame: &mut Frame<'_>, app: &mut App) {
    app.poll_image_previews();
    let size = frame.area();

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(size);

    draw_header(frame, outer[0], app);
    draw_body(frame, outer[1], app);
    draw_footer(frame, outer[2], app);

    if app.help_visible {
        draw_help(frame, centered_rect(60, 55, size), app);
    }

    if app.settings_visible {
        draw_settings(frame, centered_rect(64, 68, size), app);
    }

    if let Some(dialog) = app.dialog.as_ref() {
        draw_dialog(frame, centered_rect(58, 38, size), dialog, app);
    }
}

fn draw_header(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let title = Paragraph::new(Line::from(vec![
        Span::styled("Cabin", app.config.header_style()),
        Span::raw("  "),
        Span::raw("terminal file manager"),
    ]))
    .style(app.config.panel_style());
    frame.render_widget(title, area);
}

fn draw_body(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    let widths = match app.config.panel_layout {
        PanelLayout::Classic => {
            if area.width < 90 {
                [
                    Constraint::Length(18),
                    Constraint::Min(24),
                    Constraint::Length(28),
                ]
            } else {
                [
                    Constraint::Length(24),
                    Constraint::Min(30),
                    Constraint::Length(34),
                ]
            }
        }
        PanelLayout::Balanced => {
            if area.width < 90 {
                [
                    Constraint::Length(19),
                    Constraint::Min(24),
                    Constraint::Length(29),
                ]
            } else {
                [
                    Constraint::Length(22),
                    Constraint::Min(30),
                    Constraint::Length(36),
                ]
            }
        }
        PanelLayout::PreviewFocus => {
            [
                Constraint::Percentage(15),
                Constraint::Percentage(25),
                Constraint::Percentage(60),
            ]
        }
        PanelLayout::ContentsFocus => {
            if area.width < 90 {
                [
                    Constraint::Length(18),
                    Constraint::Min(28),
                    Constraint::Length(26),
                ]
            } else {
                [
                    Constraint::Length(26),
                    Constraint::Min(36),
                    Constraint::Length(28),
                ]
            }
        }
    };

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(widths)
        .split(area);

    draw_places(frame, columns[0], &*app);
    draw_contents(frame, columns[1], &*app);
    draw_preview(frame, columns[2], app);
}

fn draw_places(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let active = app.active_panel == Panel::Places;
    let block = panel_block("Places", active, app);
    let items = app
        .places
        .iter()
        .map(|place| ListItem::new(Line::from(place.name.clone())))
        .collect::<Vec<_>>();
    let list = List::new(items)
        .style(app.config.panel_style())
        .block(block)
        .highlight_style(active_style(app))
        .highlight_symbol("> ");

    let mut state = list_state(Some(app.places_selected), app.places.len());
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_contents(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let active = app.active_panel == Panel::Contents;
    let block = panel_block("Contents", active, app);
    let entries = if app.active_panel == Panel::Places {
        &app.hovered_place_entries
    } else {
        &app.entries
    };

    if app.active_panel == Panel::Places {
        if let Some(error) = app.hovered_place_error.as_ref() {
            let paragraph = Paragraph::new(Line::from(error.clone()))
                .style(app.config.panel_style())
                .block(block)
                .wrap(Wrap { trim: false });
            frame.render_widget(paragraph, area);
            return;
        }
    }

    let items = if entries.is_empty() {
        vec![ListItem::new(Line::from("Empty folder"))]
    } else {
        entries
            .iter()
            .map(|entry| {
                let marker = if entry.kind == crate::app::EntryKind::Directory {
                    format!("{}/", entry.name)
                } else {
                    entry.name.clone()
                };
                let marker = if entry.is_hidden {
                    format!(". {marker}")
                } else {
                    marker
                };
                ListItem::new(Line::from(marker))
            })
            .collect::<Vec<_>>()
    };
    let list = List::new(items)
        .style(app.config.panel_style())
        .block(block)
        .highlight_style(active_style(app))
        .highlight_symbol("> ");

    let mut state = if app.active_panel == Panel::Places {
        list_state(None, entries.len())
    } else {
        list_state(Some(app.contents_selected), entries.len())
    };
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_preview(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    let block = panel_block("Preview", app.active_panel == Panel::Preview, app);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some(entry) = app.entries.get(app.contents_selected) {
        if is_supported_image(&entry.path) {
            let image_path = entry.path.clone();
            let caption_lines = app.preview.lines.clone();
            let caption_height = caption_lines.len().min(inner.height as usize).min(5) as u16;
            let caption_area = Rect {
                x: inner.x,
                y: inner.y,
                width: inner.width,
                height: caption_height,
            };
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
            if caption_area.width > 0 && caption_area.height > 0 {
                frame.render_widget(caption, caption_area);
            }

            let image_area = Rect {
                x: inner.x,
                y: inner.y + caption_height,
                width: inner.width,
                height: inner.height.saturating_sub(caption_height),
            };

            if image_area.width > 0 && image_area.height > 0 {
                app.last_image_area = Some(image_area);
                app.prefetch_visible_image_previews(image_area);

                if let Some(preview) = app.cached_image_preview(&image_path, image_area) {
                    if let Some(error) = preview.error.as_ref() {
                        let error = Paragraph::new(Line::from(error.clone()))
                            .style(app.config.panel_style())
                            .wrap(Wrap { trim: false });
                        frame.render_widget(error, image_area);
                    } else if let Some(protocol) = preview.protocol.as_ref() {
                        let image = TerminalImage::new(protocol);
                        frame.render_widget(image, image_area);
                    }
                } else {
                    let loading = Paragraph::new(Line::from("Preparing image preview..."))
                        .style(app.config.panel_style())
                        .wrap(Wrap { trim: false });
                    frame.render_widget(loading, image_area);
                }
            }

            return;
        }

        if is_supported_text(&entry.path) {
            let text_path = entry.path.clone();
            let caption_lines = app.preview.lines.clone();
            let caption_height = caption_lines.len().min(inner.height as usize).min(5) as u16;
            let caption_area = Rect {
                x: inner.x,
                y: inner.y,
                width: inner.width,
                height: caption_height,
            };
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
            if caption_area.width > 0 && caption_area.height > 0 {
                frame.render_widget(caption, caption_area);
            }

            let text_area = Rect {
                x: inner.x,
                y: inner.y + caption_height,
                width: inner.width,
                height: inner.height.saturating_sub(caption_height),
            };

            if text_area.width > 0 && text_area.height > 0 {
                if let Some(preview) = app.cached_text_preview(&text_path) {
                    if let Some(error) = preview.error.as_ref() {
                        let error = Paragraph::new(Line::from(error.clone()))
                            .style(app.config.panel_style())
                            .wrap(Wrap { trim: false });
                        frame.render_widget(error, text_area);
                    } else {
                        let max_scroll = preview
                            .lines
                            .len()
                            .saturating_sub(text_area.height as usize)
                            as u16;
                        let scroll = app.preview_scroll.min(max_scroll);
                        let content = Text::from(
                            preview
                                .lines
                                .iter()
                                .cloned()
                                .map(Line::from)
                                .collect::<Vec<_>>(),
                        );
                        let paragraph = Paragraph::new(content)
                            .style(app.config.panel_style())
                            .scroll((scroll, 0))
                            .wrap(Wrap { trim: false });
                        frame.render_widget(paragraph, text_area);
                    }
                } else {
                    let loading = Paragraph::new(Line::from("Preparing text preview..."))
                        .style(app.config.panel_style())
                        .wrap(Wrap { trim: false });
                    frame.render_widget(loading, text_area);
                }
            }

            return;
        }
    }

    let lines = app.preview.lines.clone();
    let text = Text::from(lines.into_iter().map(Line::from).collect::<Vec<_>>());
    let paragraph = Paragraph::new(text)
        .style(app.config.panel_style())
        .block(Block::default())
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}

fn draw_footer(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let message = app
        .status_message
        .clone()
        .unwrap_or_else(|| String::from("Ready"));
    let mut spans = vec![
        Span::styled("q", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" quit  "),
        Span::styled("?", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" help  "),
        Span::styled("s", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" settings  "),
        Span::styled("Tab", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" switch panel  "),
        Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" open  "),
        Span::styled("Backspace", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" parent  "),
        Span::styled("h", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" hidden  "),
        Span::styled("n", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" new file  "),
        Span::styled("Shift+n", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" new folder  "),
        Span::styled("r", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" rename  "),
        Span::styled("d", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" delete  "),
        Span::styled("c", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" copy  "),
        Span::styled("x", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" cut  "),
        Span::styled("p", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" paste  "),
        Span::styled("y", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" copy path  "),
        Span::styled("F5", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" refresh  "),
        Span::styled("/", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" search  "),
        Span::styled("Ctrl+f", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" recursive search"),
    ];

    if app.config.show_footer_tips {
        spans.push(Span::raw("  |  "));
        spans.push(Span::raw(message));
    } else {
        spans.clear();
        spans.push(Span::raw(message));
    }

    let footer = Paragraph::new(Line::from(spans))
        .style(app.config.muted_style())
        .wrap(Wrap { trim: false });
    frame.render_widget(footer, area);
}

fn draw_help(frame: &mut Frame<'_>, area: Rect, app: &App) {
    frame.render_widget(Clear, area);

    let lines = vec![
        Line::from("q        Quit"),
        Line::from("?        Toggle help"),
        Line::from("s        Open settings"),
        Line::from("Tab      Next panel"),
        Line::from("Shift+Tab Previous panel"),
        Line::from("Up/Down  Move selection"),
        Line::from("j/k      Move selection"),
        Line::from("Enter    Open selected item"),
        Line::from("Backspace Parent folder"),
        Line::from("h        Toggle hidden files"),
        Line::from("c        Mark selected item for copy"),
        Line::from("x        Mark selected item for move"),
        Line::from("p        Paste into current folder"),
        Line::from("y        Copy selected path"),
        Line::from("F5       Refresh folder"),
        Line::from("/        Search current folder"),
        Line::from("Ctrl+f   Recursive search"),
        Line::from("Settings  Theme, colors, borders, layout, footer tips"),
    ];

    let paragraph = Paragraph::new(lines)
        .style(app.config.panel_style())
        .block(panel_block("Help", true, app))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn draw_dialog(frame: &mut Frame<'_>, area: Rect, dialog: &Dialog, app: &App) {
    frame.render_widget(Clear, area);

    let (title, lines) = match dialog {
        Dialog::Input { title, value, .. } => (
            title.clone(),
            vec![
                Line::from(vec![
                    Span::styled("Name: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw(value.clone()),
                ]),
                Line::from(""),
                Line::from("Enter: confirm"),
                Line::from("Esc: cancel"),
                Line::from("Backspace: delete last character"),
            ],
        ),
        Dialog::ConfirmDelete { name, .. } => (
            String::from("Confirm delete"),
            vec![
                Line::from(format!("Move \"{name}\" to Recycle Bin?")),
                Line::from(""),
                Line::from("Y / Enter: yes"),
                Line::from("N / Esc: no"),
            ],
        ),
    };

    let paragraph = Paragraph::new(lines)
        .style(app.config.panel_style())
        .block(panel_block(&title, true, app))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn draw_settings(frame: &mut Frame<'_>, area: Rect, app: &App) {
    frame.render_widget(Clear, area);

    let rows = app.settings_rows();
    let items = rows
        .iter()
        .map(|row| ListItem::new(Line::from(row.clone())))
        .collect::<Vec<_>>();
    let list = List::new(items)
        .style(app.config.panel_style())
        .block(panel_block("Settings", true, app))
        .highlight_style(app.config.active_style())
        .highlight_symbol("> ");

    let list_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: area.height.saturating_sub(3),
    };

    let mut state = list_state(Some(app.settings_selected), rows.len());
    frame.render_stateful_widget(list, list_area, &mut state);

    let footer_area = Rect {
        x: area.x.saturating_add(1),
        y: area.y + area.height.saturating_sub(3),
        width: area.width.saturating_sub(2),
        height: 2,
    };
    let tip = Paragraph::new(vec![
        Line::from("Up/Down: select   Left/Right: change   Esc/S: close"),
        Line::from(format!(
            "Config: {}",
            crate::config::CabinConfig::config_path().display()
        )),
    ])
    .style(app.config.muted_style())
    .wrap(Wrap { trim: false });
    frame.render_widget(tip, footer_area);
}

fn panel_block<'a>(title: &'a str, active: bool, app: &App) -> Block<'a> {
    Block::default()
        .border_type(app.config.border_style.border_type())
        .borders(Borders::ALL)
        .title(title)
        .style(app.config.panel_style())
        .border_style(app.config.border_color_style(active))
}

fn active_style(app: &App) -> Style {
    app.config.active_style()
}

fn list_state(selected: Option<usize>, len: usize) -> ratatui::widgets::ListState {
    let mut state = ratatui::widgets::ListState::default();
    if len > 0 {
        state.select(selected.map(|index| index.min(len.saturating_sub(1))));
    }
    state
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
