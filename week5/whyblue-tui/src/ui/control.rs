//! Control panel — manual overrides, config tuning, and command input.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::App;

const COLOR_BORDER: Color = Color::Rgb(60, 70, 100);
const COLOR_TITLE: Color = Color::Rgb(180, 190, 255);
const COLOR_ACCENT: Color = Color::Rgb(100, 140, 255);
const COLOR_DIM: Color = Color::Rgb(100, 100, 120);
const COLOR_BT: Color = Color::Rgb(0, 150, 255);
const COLOR_WIFI: Color = Color::Rgb(0, 220, 130);
const COLOR_KEY: Color = Color::Rgb(255, 200, 100);

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8),  // Hotkeys
            Constraint::Length(8),  // Config summary
            Constraint::Min(3),    // Command input
        ])
        .split(area);

    render_hotkeys(f, app, chunks[0]);
    render_config_summary(f, app, chunks[1]);
    render_command_input(f, app, chunks[2]);
}

fn render_hotkeys(f: &mut Frame, _app: &App, area: Rect) {
    let lines = vec![
        Line::from(vec![
            Span::styled(" F1 ", Style::default().fg(Color::Black).bg(COLOR_BT)),
            Span::styled(" Force Bluetooth   ", Style::default().fg(Color::White)),
            Span::styled(" F2 ", Style::default().fg(Color::Black).bg(COLOR_WIFI)),
            Span::styled(" Force Wi-Fi", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled(" F3 ", Style::default().fg(Color::Black).bg(COLOR_ACCENT)),
            Span::styled(" Auto Mode         ", Style::default().fg(Color::White)),
            Span::styled(" Q  ", Style::default().fg(Color::Black).bg(Color::Rgb(255, 70, 70))),
            Span::styled(" Quit", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled(" Tab", Style::default().fg(Color::Black).bg(COLOR_KEY)),
            Span::styled(" Switch Panel      ", Style::default().fg(Color::White)),
            Span::styled(" :  ", Style::default().fg(Color::Black).bg(COLOR_KEY)),
            Span::styled(" Command Mode", Style::default().fg(Color::White)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Commands: ", Style::default().fg(COLOR_DIM)),
            Span::styled("force bt", Style::default().fg(COLOR_ACCENT)),
            Span::raw(" | "),
            Span::styled("force wifi", Style::default().fg(COLOR_ACCENT)),
            Span::raw(" | "),
            Span::styled("auto", Style::default().fg(COLOR_ACCENT)),
            Span::raw(" | "),
            Span::styled("test <msg>", Style::default().fg(COLOR_ACCENT)),
            Span::raw(" | "),
            Span::styled("reconnect", Style::default().fg(COLOR_ACCENT)),
        ]),
    ];

    let block = Block::default()
        .title(Span::styled(
            " Controls ",
            Style::default().fg(COLOR_TITLE).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(COLOR_BORDER));

    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_config_summary(f: &mut Frame, _app: &App, area: Rect) {
    // Show key config values (could be made dynamic with IPC)
    let lines = vec![
        Line::from(vec![
            Span::styled("BT RSSI Near:    ", Style::default().fg(COLOR_DIM)),
            Span::styled("-65 dBm", Style::default().fg(Color::White)),
            Span::raw("    "),
            Span::styled("Switch Cooldown: ", Style::default().fg(COLOR_DIM)),
            Span::styled("5000 ms", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("BT RSSI Far:     ", Style::default().fg(COLOR_DIM)),
            Span::styled("-78 dBm", Style::default().fg(Color::White)),
            Span::raw("    "),
            Span::styled("Probe Interval:  ", Style::default().fg(COLOR_DIM)),
            Span::styled("500 ms", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Wi-Fi RSSI Weak: ", Style::default().fg(COLOR_DIM)),
            Span::styled("-75 dBm", Style::default().fg(Color::White)),
            Span::raw("    "),
            Span::styled("Hysteresis:      ", Style::default().fg(COLOR_DIM)),
            Span::styled("3000 ms / 5 samples", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Handover Overlap:", Style::default().fg(COLOR_DIM)),
            Span::styled(" 2000 ms", Style::default().fg(Color::White)),
            Span::raw("    "),
            Span::styled("Dwell Time:      ", Style::default().fg(COLOR_DIM)),
            Span::styled("3000 ms", Style::default().fg(Color::White)),
        ]),
    ];

    let block = Block::default()
        .title(Span::styled(
            " Configuration ",
            Style::default().fg(COLOR_TITLE).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(COLOR_BORDER));

    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_command_input(f: &mut Frame, app: &App, area: Rect) {
    let input_text = if app.command_active {
        Line::from(vec![
            Span::styled(":", Style::default().fg(COLOR_ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled(&app.command_input, Style::default().fg(Color::White)),
            Span::styled("▌", Style::default().fg(COLOR_ACCENT)), // Cursor
        ])
    } else {
        Line::from(vec![Span::styled(
            "Press ':' to enter command mode",
            Style::default().fg(COLOR_DIM),
        )])
    };

    let block = Block::default()
        .title(Span::styled(
            " Command ",
            Style::default().fg(COLOR_TITLE).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if app.command_active {
            COLOR_ACCENT
        } else {
            COLOR_BORDER
        }));

    f.render_widget(Paragraph::new(vec![input_text]).block(block), area);
}
