use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::app::{App, Dialog, Panel};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let size = frame.size();

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(5), Constraint::Length(2)])
        .split(size);

    draw_header(frame, outer[0]);
    draw_body(frame, outer[1], app);
    draw_footer(frame, outer[2], app);

    if app.help_visible {
        draw_help(frame, centered_rect(60, 55, size));
    }

    if let Some(dialog) = app.dialog.as_ref() {
        draw_dialog(frame, centered_rect(58, 38, size), dialog);
    }
}

fn draw_header(frame: &mut Frame<'_>, area: Rect) {
    let title = Paragraph::new(Line::from(vec![
        Span::styled("Cabin", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::raw("terminal file manager"),
    ]));
    frame.render_widget(title, area);
}

fn draw_body(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let widths = if area.width < 90 {
        [Constraint::Length(18), Constraint::Min(24), Constraint::Length(28)]
    } else {
        [Constraint::Length(24), Constraint::Min(30), Constraint::Length(34)]
    };

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(widths)
        .split(area);

    draw_places(frame, columns[0], app);
    draw_contents(frame, columns[1], app);
    draw_preview(frame, columns[2], app);
}

fn draw_places(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let active = app.active_panel == Panel::Places;
    let block = panel_block("Places", active);
    let items = app
        .places
        .iter()
        .map(|place| ListItem::new(Line::from(place.name.clone())))
        .collect::<Vec<_>>();
    let list = List::new(items)
        .block(block)
        .highlight_style(active_style())
        .highlight_symbol("> ");

    let mut state = list_state(app.places_selected, app.places.len());
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_contents(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let active = app.active_panel == Panel::Contents;
    let block = panel_block("Contents", active);
    let items = if app.entries.is_empty() {
        vec![ListItem::new(Line::from("Empty folder"))]
    } else {
        app.entries
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
        .block(block)
        .highlight_style(active_style())
        .highlight_symbol("> ");

    let mut state = list_state(app.contents_selected, app.entries.len());
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_preview(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let block = panel_block("Preview", app.active_panel == Panel::Preview);
    let lines = app.preview.lines.clone();
    let text = Text::from(lines.into_iter().map(Line::from).collect::<Vec<_>>());
    let paragraph = Paragraph::new(text)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn draw_footer(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let message = app
        .status_message
        .clone()
        .unwrap_or_else(|| String::from("Ready"));
    let footer = Paragraph::new(Line::from(vec![
        Span::styled("q", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" quit  "),
        Span::styled("?", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" help  "),
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
        Span::styled("y", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" copy path  "),
        Span::styled("F5", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" refresh  "),
        Span::raw("  |  "),
        Span::raw(message),
    ]))
    .wrap(Wrap { trim: false });
    frame.render_widget(footer, area);
}

fn draw_help(frame: &mut Frame<'_>, area: Rect) {
    frame.render_widget(Clear, area);

    let lines = vec![
        Line::from("q        Quit"),
        Line::from("?        Toggle help"),
        Line::from("Tab      Next panel"),
        Line::from("Shift+Tab Previous panel"),
        Line::from("Up/Down  Move selection"),
        Line::from("j/k      Move selection"),
        Line::from("Enter    Open selected item"),
        Line::from("Backspace Parent folder"),
        Line::from("h        Toggle hidden files"),
        Line::from("y        Copy selected path"),
        Line::from("F5       Refresh folder"),
    ];

    let paragraph = Paragraph::new(lines)
        .block(panel_block("Help", true))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn draw_dialog(frame: &mut Frame<'_>, area: Rect, dialog: &Dialog) {
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
        .block(panel_block(&title, true))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn panel_block(title: &str, active: bool) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(if active {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        })
}

fn active_style() -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

fn list_state(selected: usize, len: usize) -> ratatui::widgets::ListState {
    let mut state = ratatui::widgets::ListState::default();
    if len > 0 {
        state.select(Some(selected.min(len.saturating_sub(1))));
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
