use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Gauge, List, ListItem, Paragraph, Row, Table, Tabs},
};

use super::app::{App, AppScreen, CheckCopyMode};
use crate::models::DuplicateStatus;

const TABS: [&str; 4] = [" Home ", " Scan ", " Check/Copy ", " Drives "];

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Outer layout: tab bar (1) | content | status bar (1)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .split(area);

    render_tabs(frame, app, chunks[0]);
    render_status(frame, app, chunks[2]);

    match app.screen {
        AppScreen::Home => render_home(frame, app, chunks[1]),
        AppScreen::Scan => render_scan(frame, app, chunks[1]),
        AppScreen::CheckCopy => render_check_copy(frame, app, chunks[1]),
        AppScreen::Drives => render_drives(frame, app, chunks[1]),
    }
}

fn render_tabs(frame: &mut Frame, app: &App, area: Rect) {
    let tab_idx = match app.screen {
        AppScreen::Home => 0,
        AppScreen::Scan => 1,
        AppScreen::CheckCopy => 2,
        AppScreen::Drives => 3,
    };

    let spinner = if app.active_task.is_some() {
        "⠋ "
    } else {
        ""
    };

    let tabs = Tabs::new(TABS)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" local_image_db {spinner}")),
        )
        .select(tab_idx)
        .style(Style::default().fg(Color::White))
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .divider("|");

    frame.render_widget(tabs, area);
}

fn render_status(frame: &mut Frame, app: &App, area: Rect) {
    let msg = app
        .status_message
        .as_deref()
        .unwrap_or("Ready  [1-4] Switch tab  [q] Quit");
    let p = Paragraph::new(msg).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(p, area);
}

// ── Home ─────────────────────────────────────────────────────────────────────

fn render_home(frame: &mut Frame, app: &App, area: Rect) {
    let h = &app.home;
    let loading = if h.loading { " (loading…)" } else { "" };

    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Images indexed:   ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{}{}", h.image_count, loading),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Drives known:     ", Style::default().fg(Color::Gray)),
            Span::styled(
                h.drive_count.to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Missing hashes:   ", Style::default().fg(Color::Gray)),
            Span::styled(
                h.unhashed_count.to_string(),
                Style::default()
                    .fg(if h.unhashed_count > 0 {
                        Color::Yellow
                    } else {
                        Color::Green
                    })
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  [r] Refresh stats",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let p = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Database Statistics "),
    );
    frame.render_widget(p, area);
}

// ── Scan ─────────────────────────────────────────────────────────────────────

fn render_scan(frame: &mut Frame, app: &App, area: Rect) {
    let s = &app.scan;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7), // inputs
            Constraint::Length(5), // progress
            Constraint::Fill(1),   // log
        ])
        .split(area);

    // Input block
    let path_style = focused_style(s.focused_field == 0 && !s.running);
    let label_style = focused_style(s.focused_field == 1 && !s.running);

    let input_text = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  Path:  "),
            Span::styled(format!("[{}]", s.path_input), path_style),
        ]),
        Line::from(vec![
            Span::raw("  Label: "),
            Span::styled(format!("[{}]", s.label_input), label_style),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                if s.force_rehash {
                    "[x] Force rehash"
                } else {
                    "[ ] Force rehash"
                },
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw("    "),
            Span::styled(
                if s.running {
                    "Running…"
                } else {
                    "[Enter] Start Scan  [Tab] Next field  [Space] Toggle rehash"
                },
                Style::default().fg(Color::DarkGray),
            ),
        ]),
    ];

    frame.render_widget(
        Paragraph::new(input_text)
            .block(Block::default().borders(Borders::ALL).title(" Scan Drive ")),
        chunks[0],
    );

    // Progress block
    render_progress_gauge(
        frame,
        chunks[1],
        " Progress ",
        s.progress_current,
        s.progress_total,
        &s.progress_detail,
        s.result.as_deref(),
    );

    // Log block
    render_log(frame, chunks[2], " Log ", &s.log);
}

// ── Check/Copy ───────────────────────────────────────────────────────────────

fn render_check_copy(frame: &mut Frame, app: &App, area: Rect) {
    let cc = &app.check_copy;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7), // inputs
            Constraint::Fill(1),   // candidate table
            Constraint::Length(5), // progress
        ])
        .split(area);

    // Input block
    let sd_style = focused_style(cc.focused_field == 0 && !cc.running);
    let drive_style = focused_style(cc.focused_field == 1 && !cc.running);

    let mode_label = match cc.mode {
        CheckCopyMode::Check => "Check",
        CheckCopyMode::Copy => "Copy ",
    };

    let hint = if cc.running {
        "Running…"
    } else if cc.loading {
        "Loading candidates…"
    } else {
        "[c] Check  [x] Copy  [Tab] Next field  [h] Toggle hash"
    };

    let input_text = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  SD Path:    "),
            Span::styled(format!("[{}]", cc.sd_input), sd_style),
        ]),
        Line::from(vec![
            Span::raw("  Drive Path: "),
            Span::styled(format!("[{}]", cc.drive_input), drive_style),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                if cc.use_hash {
                    "[x] Hash dedup"
                } else {
                    "[ ] Hash dedup"
                },
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw(format!("   Mode: {mode_label}   ")),
            Span::styled(hint, Style::default().fg(Color::DarkGray)),
        ]),
    ];

    frame.render_widget(
        Paragraph::new(input_text).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Check / Copy "),
        ),
        chunks[0],
    );

    // Candidate table
    render_candidate_table(frame, app, chunks[1]);

    // Progress
    render_progress_gauge(
        frame,
        chunks[2],
        " Progress ",
        cc.progress_current,
        cc.progress_total,
        &cc.progress_detail,
        cc.result.as_deref(),
    );
}

fn render_candidate_table(frame: &mut Frame, app: &App, area: Rect) {
    let cc = &app.check_copy;

    let header = Row::new(vec![
        Cell::from("Filename").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Status").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Destination").style(Style::default().add_modifier(Modifier::BOLD)),
    ])
    .style(Style::default().fg(Color::Yellow));

    let visible_height = area.height.saturating_sub(3) as usize; // borders + header
    let offset = cc.table_offset;

    let rows: Vec<Row> = cc
        .to_copy
        .iter()
        .skip(offset)
        .take(visible_height)
        .enumerate()
        .map(|(i, c)| {
            let is_selected = (i + offset) == cc.selected_row;
            let (status_str, status_color) = match &c.status {
                DuplicateStatus::New => ("New", Color::Green),
                DuplicateStatus::SizeConflict => ("SizeConflict", Color::Yellow),
                DuplicateStatus::HashConflict => ("HashConflict", Color::Magenta),
                DuplicateStatus::AlreadyExists { .. } => ("Exists", Color::DarkGray),
            };
            let row_style = if is_selected {
                Style::default().bg(Color::DarkGray)
            } else {
                Style::default()
            };
            let dest = c.destination_path.to_string_lossy().to_string();
            Row::new(vec![
                Cell::from(c.source.filename.clone()),
                Cell::from(status_str).style(Style::default().fg(status_color)),
                Cell::from(dest),
            ])
            .style(row_style)
        })
        .collect();

    let summary = format!(
        " Candidates: {} to copy, {} already on drive ",
        cc.to_copy.len(),
        cc.already.len()
    );

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(30),
            Constraint::Length(14),
            Constraint::Fill(1),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title(summary));

    frame.render_widget(table, area);
}

// ── Drives ───────────────────────────────────────────────────────────────────

fn render_drives(frame: &mut Frame, app: &App, area: Rect) {
    let d = &app.drives;

    let header = Row::new(vec![
        Cell::from("ID").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Label").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Path").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Last Scanned").style(Style::default().add_modifier(Modifier::BOLD)),
    ])
    .style(Style::default().fg(Color::Yellow));

    let visible_height = area.height.saturating_sub(5) as usize;

    let rows: Vec<Row> = d
        .drives
        .iter()
        .enumerate()
        .take(visible_height)
        .map(|(i, dr)| {
            let is_selected = i == d.selected_row;
            let last = dr
                .last_scanned_at
                .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "never".to_string());
            let row_style = if is_selected {
                Style::default().bg(Color::DarkGray)
            } else {
                Style::default()
            };
            Row::new(vec![
                Cell::from(dr.id.to_string()),
                Cell::from(dr.label.as_str()),
                Cell::from(dr.root_path.as_str()),
                Cell::from(last),
            ])
            .style(row_style)
        })
        .collect();

    let hint = " [a] Add  [d] Remove  [r] Refresh  [↑↓] Navigate ";
    let loading = if d.loading { " (loading…)" } else { "" };

    let table = Table::new(
        rows,
        [
            Constraint::Length(5),
            Constraint::Percentage(20),
            Constraint::Fill(1),
            Constraint::Length(18),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Drives{loading}{hint}")),
    );

    frame.render_widget(table, area);

    // Add-drive overlay
    if d.add_open {
        render_add_drive_overlay(frame, app, area);
    }
}

fn render_add_drive_overlay(frame: &mut Frame, app: &App, area: Rect) {
    let d = &app.drives;

    let popup_area = centered_rect(60, 40, area);
    frame.render_widget(Clear, popup_area);

    let path_style = focused_style(d.add_focused == 0);
    let label_style = focused_style(d.add_focused == 1);

    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  Path:  "),
            Span::styled(format!("[{}]", d.add_path), path_style),
        ]),
        Line::from(vec![
            Span::raw("  Label: "),
            Span::styled(format!("[{}]", d.add_label), label_style),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  [Enter] Save   [Esc] Cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let p = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Add Drive ")
            .style(Style::default().fg(Color::Yellow)),
    );
    frame.render_widget(p, popup_area);
}

// ── Shared helpers ────────────────────────────────────────────────────────────

fn render_progress_gauge(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    current: u64,
    total: u64,
    detail: &str,
    result: Option<&str>,
) {
    let ratio = if total > 0 {
        (current as f64 / total as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let label = if let Some(r) = result {
        r.to_string()
    } else if total > 0 {
        format!("{current}/{total}  {detail}")
    } else {
        "Idle".to_string()
    };

    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title(title))
        .gauge_style(Style::default().fg(Color::Cyan).bg(Color::DarkGray))
        .ratio(ratio)
        .label(label);

    frame.render_widget(gauge, area);
}

fn render_log(frame: &mut Frame, area: Rect, title: &str, log: &[String]) {
    let items: Vec<ListItem> = log
        .iter()
        .rev()
        .take(area.height.saturating_sub(2) as usize)
        .rev()
        .map(|l| ListItem::new(l.as_str()).style(Style::default().fg(Color::DarkGray)))
        .collect();

    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(list, area);
}

fn focused_style(is_focused: bool) -> Style {
    if is_focused {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    }
}

/// Return a rect centered within `area` at the given percentage dimensions.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
