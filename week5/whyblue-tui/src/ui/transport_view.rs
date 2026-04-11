//! Detailed per-transport view with RSSI history and extended metrics.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Sparkline},
    Frame,
};

use crate::app::App;

const COLOR_BT: Color = Color::Rgb(0, 150, 255);
const COLOR_WIFI: Color = Color::Rgb(0, 220, 130);
const COLOR_BORDER: Color = Color::Rgb(60, 70, 100);
const COLOR_TITLE: Color = Color::Rgb(180, 190, 255);
const COLOR_DIM: Color = Color::Rgb(100, 100, 120);
const COLOR_OK: Color = Color::Rgb(80, 220, 100);
const COLOR_WARN: Color = Color::Rgb(255, 180, 50);
const COLOR_CRIT: Color = Color::Rgb(255, 70, 70);

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    render_transport_detail(
        f,
        " ⦿ Bluetooth Detail ",
        COLOR_BT,
        &app.snapshot.bt_metrics,
        &app.bt_rtt_history,
        &app.bt_rssi_history,
        app.snapshot.active_transport == whyblue_core::types::WbTransport::Bluetooth,
        cols[0],
    );

    render_transport_detail(
        f,
        " ⦿ Wi-Fi Detail ",
        COLOR_WIFI,
        &app.snapshot.wifi_metrics,
        &app.wifi_rtt_history,
        &app.wifi_rssi_history,
        app.snapshot.active_transport == whyblue_core::types::WbTransport::Wifi,
        cols[1],
    );
}

fn render_transport_detail(
    f: &mut Frame,
    title: &str,
    color: Color,
    metrics: &whyblue_core::types::LinkMetrics,
    rtt_history: &std::collections::VecDeque<u64>,
    rssi_history: &std::collections::VecDeque<u64>,
    is_primary: bool,
    area: Rect,
) {
    let border_color = if is_primary { color } else { COLOR_BORDER };

    let block = Block::default()
        .title(Span::styled(
            title,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),  // Metrics text
            Constraint::Length(3),  // RTT sparkline
            Constraint::Min(3),    // RSSI sparkline
        ])
        .split(inner);

    // ── Metrics text ──
    let alive_str = if metrics.alive { "● ALIVE" } else { "○ DEAD" };
    let alive_color = if metrics.alive { COLOR_OK } else { COLOR_CRIT };
    let primary_str = if is_primary { " [PRIMARY]" } else { "" };

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

    let lines = vec![
        Line::from(vec![
            Span::styled(alive_str, Style::default().fg(alive_color)),
            Span::styled(primary_str, Style::default().fg(color).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("RTT:       ", Style::default().fg(COLOR_DIM)),
            Span::styled(format!("{:.1} ms", metrics.rtt_ms), Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Loss:      ", Style::default().fg(COLOR_DIM)),
            Span::styled(format!("{:.2}%", metrics.loss_pct), Style::default().fg(loss_color)),
        ]),
        Line::from(vec![
            Span::styled("Throughput:", Style::default().fg(COLOR_DIM)),
            Span::styled(format!(" {:.0} kbps", metrics.throughput_kbps), Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Jitter:    ", Style::default().fg(COLOR_DIM)),
            Span::styled(format!("{:.1} ms", metrics.jitter_ms), Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Stability: ", Style::default().fg(COLOR_DIM)),
            Span::styled(format!("{:.3}", metrics.stability_score), Style::default().fg(score_color)),
        ]),
        Line::from(vec![
            Span::styled("RSSI:      ", Style::default().fg(COLOR_DIM)),
            Span::styled(format!("{} dBm", metrics.rssi_dbm), Style::default().fg(Color::White)),
            Span::raw("  "),
            Span::styled("Reconnects: ", Style::default().fg(COLOR_DIM)),
            Span::styled(format!("{}", metrics.reconnect_count), Style::default().fg(Color::White)),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), chunks[0]);

    // ── RTT sparkline ──
    let rtt_data: Vec<u64> = rtt_history.iter().copied().collect();
    if !rtt_data.is_empty() {
        let sparkline = Sparkline::default()
            .data(&rtt_data)
            .style(Style::default().fg(color))
            .block(
                Block::default()
                    .title(Span::styled(" RTT ", Style::default().fg(COLOR_DIM)))
                    .borders(Borders::TOP),
            );
        f.render_widget(sparkline, chunks[1]);
    }

    // ── RSSI sparkline ──
    let rssi_data: Vec<u64> = rssi_history.iter().copied().collect();
    if !rssi_data.is_empty() {
        let sparkline = Sparkline::default()
            .data(&rssi_data)
            .style(Style::default().fg(Color::Rgb(200, 150, 255)))
            .block(
                Block::default()
                    .title(Span::styled(" RSSI ", Style::default().fg(COLOR_DIM)))
                    .borders(Borders::TOP),
            );
        f.render_widget(sparkline, chunks[2]);
    }
}
