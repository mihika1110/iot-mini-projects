//! FSM state visualization panel.
//!
//! Shows a text-based state machine diagram with the current state
//! highlighted, plus the transition history.

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
const COLOR_ACTIVE: Color = Color::Rgb(100, 255, 150);
const COLOR_DIM: Color = Color::Rgb(70, 70, 90);
const COLOR_ARROW: Color = Color::Rgb(100, 140, 255);

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(14), Constraint::Length(6)])
        .split(area);

    render_fsm_diagram(f, app, chunks[0]);
    render_state_info(f, app, chunks[1]);
}

fn render_fsm_diagram(f: &mut Frame, app: &App, area: Rect) {
    let current = app.snapshot.state;

    let _states = [
        ("INIT", whyblue_core::types::WbState::Init),
        ("DISCOVERING", whyblue_core::types::WbState::Discovering),
        ("BT_ONLY", whyblue_core::types::WbState::BtOnly),
        ("WIFI_ONLY", whyblue_core::types::WbState::WifiOnly),
        ("DUAL_READY", whyblue_core::types::WbState::DualReady),
        ("HANDOVER_BT→WIFI", whyblue_core::types::WbState::HandoverBtToWifi),
        ("HANDOVER_WIFI→BT", whyblue_core::types::WbState::HandoverWifiToBt),
        ("DEGRADED", whyblue_core::types::WbState::Degraded),
        ("RECOVERY", whyblue_core::types::WbState::Recovery),
    ];

    // Build FSM diagram lines
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        "  ┌──────────────── FSM State Machine ────────────────┐",
        Style::default().fg(COLOR_BORDER),
    )));

    // Row 1: INIT ──► DISCOVERING
    lines.push(make_fsm_row(
        "  │  ",
        &[("INIT", current == whyblue_core::types::WbState::Init)],
        " ───► ",
        &[("DISCOVERING", current == whyblue_core::types::WbState::Discovering)],
        "          │",
    ));

    // Row 2: Arrow down
    lines.push(Line::from(Span::styled(
        "  │           │                    │                   │",
        Style::default().fg(COLOR_DIM),
    )));

    // Row 3: DUAL_READY
    lines.push(make_fsm_row(
        "  │           └────────► ",
        &[("DUAL_READY", current == whyblue_core::types::WbState::DualReady)],
        " ◄───────┘",
        &[],
        "    │",
    ));

    // Row 4: Split
    lines.push(Line::from(Span::styled(
        "  │                     ╱            ╲                 │",
        Style::default().fg(COLOR_DIM),
    )));

    // Row 5: BT_ONLY ←──→ WIFI_ONLY
    lines.push(make_fsm_row(
        "  │            ",
        &[("BT_ONLY", current == whyblue_core::types::WbState::BtOnly)],
        " ◄─────► ",
        &[("WIFI_ONLY", current == whyblue_core::types::WbState::WifiOnly)],
        "      │",
    ));

    // Row 6: Handover arrows
    lines.push(Line::from(Span::styled(
        "  │            │ ╲              ╱ │                    │",
        Style::default().fg(COLOR_DIM),
    )));

    // Row 7: Handover states
    lines.push(make_fsm_row(
        "  │   ",
        &[("HO_BT→WF", current == whyblue_core::types::WbState::HandoverBtToWifi)],
        "    ",
        &[("HO_WF→BT", current == whyblue_core::types::WbState::HandoverWifiToBt)],
        "        │",
    ));

    // Row 8: To degraded
    lines.push(Line::from(Span::styled(
        "  │                        │                           │",
        Style::default().fg(COLOR_DIM),
    )));

    // Row 9: DEGRADED ──► RECOVERY
    lines.push(make_fsm_row(
        "  │            ",
        &[("DEGRADED", current == whyblue_core::types::WbState::Degraded)],
        " ───► ",
        &[("RECOVERY", current == whyblue_core::types::WbState::Recovery)],
        "       │",
    ));

    lines.push(Line::from(Span::styled(
        "  └────────────────────────────────────────────────────┘",
        Style::default().fg(COLOR_BORDER),
    )));

    let block = Block::default()
        .title(Span::styled(
            " FSM Diagram ",
            Style::default().fg(COLOR_TITLE).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(COLOR_BORDER));

    let paragraph = Paragraph::new(lines).block(block);
    f.render_widget(paragraph, area);
}

fn make_fsm_row(
    prefix: &str,
    left: &[(&str, bool)],
    mid: &str,
    right: &[(&str, bool)],
    suffix: &str,
) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::styled(prefix.to_string(), Style::default().fg(COLOR_DIM)));

    for (name, active) in left {
        let style = if *active {
            Style::default()
                .fg(COLOR_ACTIVE)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            Style::default().fg(COLOR_DIM)
        };
        spans.push(Span::styled(format!("[{name}]"), style));
    }

    spans.push(Span::styled(mid.to_string(), Style::default().fg(COLOR_ARROW)));

    for (name, active) in right {
        let style = if *active {
            Style::default()
                .fg(COLOR_ACTIVE)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            Style::default().fg(COLOR_DIM)
        };
        spans.push(Span::styled(format!("[{name}]"), style));
    }

    spans.push(Span::styled(suffix.to_string(), Style::default().fg(COLOR_DIM)));
    Line::from(spans)
}

fn render_state_info(f: &mut Frame, app: &App, area: Rect) {
    let snap = &app.snapshot;

    let lines = vec![
        Line::from(vec![
            Span::styled("Session ID: ", Style::default().fg(COLOR_DIM)),
            Span::styled(
                format!("0x{:08X}", snap.session_id),
                Style::default().fg(Color::White),
            ),
            Span::raw("    "),
            Span::styled("Generation: ", Style::default().fg(COLOR_DIM)),
            Span::styled(
                format!("{}", snap.handover_generation),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("TX: ", Style::default().fg(COLOR_DIM)),
            Span::styled(
                format_bytes(snap.total_tx_bytes),
                Style::default().fg(Color::White),
            ),
            Span::raw("    "),
            Span::styled("RX: ", Style::default().fg(COLOR_DIM)),
            Span::styled(
                format_bytes(snap.total_rx_bytes),
                Style::default().fg(Color::White),
            ),
        ]),
    ];

    let block = Block::default()
        .title(Span::styled(
            " Session Info ",
            Style::default().fg(COLOR_TITLE).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(COLOR_BORDER));

    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
