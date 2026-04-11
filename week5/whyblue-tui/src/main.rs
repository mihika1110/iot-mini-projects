//! WhyBlue TUI Dashboard — main entry point.
//!
//! A ratatui-based terminal dashboard for monitoring and controlling
//! the WhyBlue dual-transport networking daemon.

mod app;
mod ipc_client;
mod ui;

use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Tabs},
    Frame, Terminal,
};

use app::{ActivePanel, App};

/// IPC socket path (should match daemon config).
const DEFAULT_IPC_PATH: &str = "/tmp/whyblue.sock";

/// Dashboard tick rate in milliseconds.
const TICK_RATE_MS: u64 = 200;

/// Poll rate for daemon status in ticks (200ms * 2 = 400ms).
const STATUS_POLL_TICKS: u64 = 2;

/// Reconnect attempt interval in ticks.
const RECONNECT_TICKS: u64 = 25; // ~5 seconds

#[tokio::main]
async fn main() -> Result<()> {
    // Determine IPC path from args or default
    let ipc_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_IPC_PATH.to_string());

    // Set up terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
    let mut app = App::new(ipc_path);

    // Try initial connection
    app.try_connect().await;

    // Main event loop
    let result = run_app(&mut terminal, &mut app).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(e) = result {
        eprintln!("Error: {e}");
    }

    Ok(())
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        // Draw
        terminal.draw(|f| draw_ui(f, app))?;

        // Handle events with timeout
        if event::poll(Duration::from_millis(TICK_RATE_MS))? {
            if let Event::Key(key) = event::read()? {
                // Only handle key press events (not release/repeat on some terminals)
                if key.kind == KeyEventKind::Press {
                    app.handle_key(key.code).await;
                }
            }
        }

        // Check if should quit
        if app.should_quit {
            return Ok(());
        }

        // Tick
        app.tick();

        // Poll daemon status
        if app.tick % STATUS_POLL_TICKS == 0 {
            app.poll_status().await;
        }

        // Retry connection if disconnected
        if !app.connected && app.tick % RECONNECT_TICKS == 0 {
            app.try_connect().await;
        }
    }
}

fn draw_ui(f: &mut Frame, app: &App) {
    let size = f.area();

    // Overall layout: title bar + tabs + main content + status bar
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Title + tabs
            Constraint::Min(10),   // Main content
            Constraint::Length(1), // Status bar
        ])
        .split(size);

    // ── Title bar with tabs ──
    let tab_titles = vec![
        ActivePanel::Dashboard.label(),
        ActivePanel::TransportDetail.label(),
        ActivePanel::StateView.label(),
        ActivePanel::Chat.label(),
        ActivePanel::Control.label(),
    ];

    let selected_idx = match app.panel {
        ActivePanel::Dashboard => 0,
        ActivePanel::TransportDetail => 1,
        ActivePanel::StateView => 2,
        ActivePanel::Chat => 3,
        ActivePanel::Control => 4,
    };

    let tabs = Tabs::new(tab_titles)
        .block(
            Block::default()
                .title(Span::styled(
                    " ◈ WhyBlue Dashboard ",
                    Style::default()
                        .fg(Color::Rgb(140, 170, 255))
                        .add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(60, 70, 100))),
        )
        .select(selected_idx)
        .style(Style::default().fg(Color::Rgb(100, 100, 120)))
        .highlight_style(
            Style::default()
                .fg(Color::Rgb(100, 220, 255))
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )
        .divider(Span::styled(" │ ", Style::default().fg(Color::Rgb(60, 70, 100))));

    f.render_widget(tabs, outer[0]);

    // ── Main content based on selected tab ──
    match app.panel {
        ActivePanel::Dashboard => ui::dashboard::render(f, app, outer[1]),
        ActivePanel::TransportDetail => ui::transport_view::render(f, app, outer[1]),
        ActivePanel::StateView => ui::state_view::render(f, app, outer[1]),
        ActivePanel::Chat => ui::chat::render(f, app, outer[1]),
        ActivePanel::Control => ui::control::render(f, app, outer[1]),
    }

    // ── Status bar ──
    let connection_indicator = if app.connected {
        Span::styled(" ● ", Style::default().fg(Color::Rgb(80, 220, 100)))
    } else {
        Span::styled(" ○ ", Style::default().fg(Color::Rgb(255, 70, 70)))
    };

    let status_line = Line::from(vec![
        connection_indicator,
        Span::styled(&app.status_msg, Style::default().fg(Color::Rgb(150, 150, 170))),
        Span::raw("  "),
        Span::styled(
            format!("v0.1.0 │ {:>5} ticks", app.tick),
            Style::default().fg(Color::Rgb(70, 70, 90)),
        ),
    ]);

    f.render_widget(
        Paragraph::new(status_line).style(Style::default().bg(Color::Rgb(20, 22, 35))),
        outer[2],
    );
}
