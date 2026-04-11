//! Main dashboard view — the primary TUI panel.
//!
//! Layout:
//! ┌─ State ─────┐ ┌─ Proximity ──────────────────────────┐
//! │ ● STATE     │ │ [████████░░] NEAR (conf: 0.87)       │
//! │ Active: BT  │ │ BT RSSI: -58   Wi-Fi RSSI: -42      │
//! └─────────────┘ └──────────────────────────────────────┘
//! ┌─ Bluetooth ─────────────┐ ┌─ Wi-Fi ──────────────────┐
//! │ RTT:  12ms  ▁▂▃▅▇█▅▃   │ │ RTT:  8ms  ▂▃▅▃▂▁       │
//! │ Loss: 0.2%              │ │ Loss: 0.0%               │
//! │ Score: 0.82 ● ALIVE     │ │ Score: 0.91 ● ALIVE      │
//! └─────────────────────────┘ └──────────────────────────┘
//! ┌─ Transitions ────────────────────────────────────────┐
//! │ 18:04:12  BtOnly → WifiOnly  (reason)               │
//! └──────────────────────────────────────────────────────┘

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Row, Sparkline, Table},
    Frame,
};

use crate::app::App;

/// Color palette
const COLOR_BG: Color = Color::Rgb(15, 15, 25);
const COLOR_BORDER: Color = Color::Rgb(60, 70, 100);
const COLOR_ACCENT: Color = Color::Rgb(100, 140, 255);
const COLOR_BT: Color = Color::Rgb(0, 150, 255);
const COLOR_WIFI: Color = Color::Rgb(0, 220, 130);
const COLOR_WARN: Color = Color::Rgb(255, 180, 50);
const COLOR_CRIT: Color = Color::Rgb(255, 70, 70);
const COLOR_OK: Color = Color::Rgb(80, 220, 100);
const COLOR_DIM: Color = Color::Rgb(100, 100, 120);
const COLOR_TITLE: Color = Color::Rgb(180, 190, 255);

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),  // State + Proximity
            Constraint::Length(8),  // Transport panels
            Constraint::Min(6),    // Transition log
        ])
        .split(area);

    render_top_bar(f, app, chunks[0]);
    render_transport_panels(f, app, chunks[1]);
    render_transition_log(f, app, chunks[2]);
}

fn render_top_bar(f: &mut Frame, app: &App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(22), Constraint::Min(30)])
        .split(area);

    // ── State indicator ──
    let state = &app.snapshot.state;
    let active = &app.snapshot.active_transport;
    let state_color = match state {
        whyblue_core::types::WbState::BtOnly => COLOR_BT,
        whyblue_core::types::WbState::WifiOnly => COLOR_WIFI,
        whyblue_core::types::WbState::DualReady => COLOR_OK,
        whyblue_core::types::WbState::Degraded => COLOR_CRIT,
        whyblue_core::types::WbState::HandoverBtToWifi
        | whyblue_core::types::WbState::HandoverWifiToBt => COLOR_WARN,
        _ => COLOR_DIM,
    };

    let state_text = vec![
        Line::from(vec![
            Span::styled("● ", Style::default().fg(state_color)),
            Span::styled(format!("{state}"), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("Active: ", Style::default().fg(COLOR_DIM)),
            Span::styled(format!("{active}"), Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Gen: ", Style::default().fg(COLOR_DIM)),
            Span::styled(
                format!("{}", app.snapshot.handover_generation),
                Style::default().fg(Color::White),
            ),
        ]),
    ];

    let state_block = Paragraph::new(state_text).block(
        Block::default()
            .title(Span::styled(" State ", Style::default().fg(COLOR_TITLE).add_modifier(Modifier::BOLD)))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(COLOR_BORDER)),
    );
    f.render_widget(state_block, cols[0]);

    // ── Proximity indicator ──
    let prox = &app.snapshot.proximity;
    let conf = app.snapshot.proximity_confidence;
    let bt_rssi = app.snapshot.bt_metrics.rssi_dbm;
    let wifi_rssi = app.snapshot.wifi_metrics.rssi_dbm;

    let prox_color = match prox {
        whyblue_core::types::Proximity::Near => COLOR_BT,
        whyblue_core::types::Proximity::Mid => COLOR_WARN,
        whyblue_core::types::Proximity::Far => COLOR_WIFI,
        whyblue_core::types::Proximity::Unknown => COLOR_DIM,
    };

    // Confidence gauge bar
    let gauge_filled = (conf * 20.0) as usize;
    let gauge_bar = format!(
        "[{}{}]",
        "█".repeat(gauge_filled),
        "░".repeat(20 - gauge_filled)
    );

    let prox_text = vec![
        Line::from(vec![
            Span::styled(&gauge_bar, Style::default().fg(prox_color)),
            Span::styled(format!(" {prox}"), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" (conf: {conf:.2})"), Style::default().fg(COLOR_DIM)),
        ]),
        Line::from(vec![
            Span::styled("BT RSSI: ", Style::default().fg(COLOR_BT)),
            Span::styled(format!("{bt_rssi} dBm"), Style::default().fg(Color::White)),
            Span::raw("  "),
            Span::styled("Wi-Fi RSSI: ", Style::default().fg(COLOR_WIFI)),
            Span::styled(format!("{wifi_rssi} dBm"), Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Uptime: ", Style::default().fg(COLOR_DIM)),
            Span::styled(format_duration(app.snapshot.uptime_secs), Style::default().fg(Color::White)),
            Span::raw("  "),
            Span::styled("Handovers: ", Style::default().fg(COLOR_DIM)),
            Span::styled(format!("{}", app.snapshot.handover_count), Style::default().fg(Color::White)),
        ]),
    ];

    let prox_block = Paragraph::new(prox_text).block(
        Block::default()
            .title(Span::styled(" Proximity ", Style::default().fg(COLOR_TITLE).add_modifier(Modifier::BOLD)))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(COLOR_BORDER)),
    );
    f.render_widget(prox_block, cols[1]);
}

fn render_transport_panels(f: &mut Frame, app: &App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    render_single_transport(
        f,
        " ⦿ Bluetooth ",
        COLOR_BT,
        &app.snapshot.bt_metrics,
        &app.bt_rtt_history,
        app.snapshot.active_transport == whyblue_core::types::WbTransport::Bluetooth,
        cols[0],
    );

    render_single_transport(
        f,
        " ⦿ Wi-Fi ",
        COLOR_WIFI,
        &app.snapshot.wifi_metrics,
        &app.wifi_rtt_history,
        app.snapshot.active_transport == whyblue_core::types::WbTransport::Wifi,
        cols[1],
    );
}

fn render_single_transport(
    f: &mut Frame,
    title: &str,
    color: Color,
    metrics: &whyblue_core::types::LinkMetrics,
    rtt_history: &std::collections::VecDeque<u64>,
    is_primary: bool,
    area: Rect,
) {
    let inner_chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Length(2), Constraint::Min(2)])
        .split(area);

    let border_color = if is_primary {
        color
    } else {
        COLOR_BORDER
    };

    let alive_indicator = if metrics.alive {
        Span::styled("● ALIVE", Style::default().fg(COLOR_OK))
    } else {
        Span::styled("○ DEAD", Style::default().fg(COLOR_CRIT))
    };

    let primary_tag = if is_primary {
        Span::styled(" [PRIMARY]", Style::default().fg(color).add_modifier(Modifier::BOLD))
    } else {
        Span::raw("")
    };

    let loss_color = if metrics.loss_pct < 1.0 {
        COLOR_OK
    } else if metrics.loss_pct < 10.0 {
        COLOR_WARN
    } else {
        COLOR_CRIT
    };

    let score_color = if metrics.stability_score > 0.7 {
        COLOR_OK
    } else if metrics.stability_score > 0.4 {
        COLOR_WARN
    } else {
        COLOR_CRIT
    };

    let metrics_text = vec![
        Line::from(vec![
            Span::styled("RTT: ", Style::default().fg(COLOR_DIM)),
            Span::styled(format!("{:.1}ms", metrics.rtt_ms), Style::default().fg(Color::White)),
            Span::raw("  "),
            Span::styled("Loss: ", Style::default().fg(COLOR_DIM)),
            Span::styled(format!("{:.1}%", metrics.loss_pct), Style::default().fg(loss_color)),
            Span::raw("  "),
            alive_indicator,
            primary_tag,
        ]),
        Line::from(vec![
            Span::styled("Tput: ", Style::default().fg(COLOR_DIM)),
            Span::styled(
                format!("{:.0} kbps", metrics.throughput_kbps),
                Style::default().fg(Color::White),
            ),
            Span::raw("  "),
            Span::styled("Score: ", Style::default().fg(COLOR_DIM)),
            Span::styled(
                format!("{:.2}", metrics.stability_score),
                Style::default().fg(score_color),
            ),
            Span::raw("  "),
            Span::styled("Jitter: ", Style::default().fg(COLOR_DIM)),
            Span::styled(
                format!("{:.1}ms", metrics.jitter_ms),
                Style::default().fg(Color::White),
            ),
        ]),
    ];

    let block = Block::default()
        .title(Span::styled(
            title,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    f.render_widget(block, area);
    f.render_widget(Paragraph::new(metrics_text), inner_chunks[0]);

    // RTT sparkline
    let data: Vec<u64> = rtt_history.iter().copied().collect();
    if !data.is_empty() {
        let sparkline = Sparkline::default()
            .data(&data)
            .style(Style::default().fg(color));
        f.render_widget(sparkline, inner_chunks[1]);
    }
}

fn render_transition_log(f: &mut Frame, app: &App, area: Rect) {
    let rows: Vec<Row> = app
        .transition_log
        .iter()
        .rev()
        .take(10)
        .map(|event| {
            let color = match event.to_state {
                whyblue_core::types::WbState::BtOnly => COLOR_BT,
                whyblue_core::types::WbState::WifiOnly => COLOR_WIFI,
                whyblue_core::types::WbState::Degraded => COLOR_CRIT,
                whyblue_core::types::WbState::HandoverBtToWifi
                | whyblue_core::types::WbState::HandoverWifiToBt => COLOR_WARN,
                _ => COLOR_DIM,
            };
            Row::new(vec![
                event.timestamp.clone(),
                format!("{}", event.from_state),
                "→".to_string(),
                format!("{}", event.to_state),
                event.reason.clone(),
            ])
            .style(Style::default().fg(color))
        })
        .collect();

    let widths = [
        Constraint::Length(10),
        Constraint::Length(18),
        Constraint::Length(2),
        Constraint::Length(18),
        Constraint::Min(20),
    ];

    let table = Table::new(rows, widths)
        .block(
            Block::default()
                .title(Span::styled(
                    " FSM Transitions ",
                    Style::default().fg(COLOR_TITLE).add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(COLOR_BORDER)),
        )
        .header(
            Row::new(vec!["Time", "From", "", "To", "Reason"])
                .style(Style::default().fg(COLOR_ACCENT).add_modifier(Modifier::BOLD)),
        );

    f.render_widget(table, area);
}

fn format_duration(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h {m}m {s}s")
    } else if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}
