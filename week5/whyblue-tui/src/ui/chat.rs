use ratatui::{
    layout::{Constraint, Direction, Layout, Rect, Alignment},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::App;

const COLOR_BORDER: Color = Color::Rgb(60, 70, 100);
const COLOR_TITLE: Color = Color::Rgb(180, 190, 255);
const COLOR_PEER_MSG: Color = Color::Rgb(50, 220, 100); // Greenish
const COLOR_SELF_MSG: Color = Color::Rgb(100, 150, 255); // Blueish

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(Span::styled(
            " Live Data Stream ",
            Style::default().fg(COLOR_TITLE).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(COLOR_BORDER));
    
    // We render each message. "Peer" left-aligned, "You" right-aligned
    let mut lines = Vec::new();
    
    if app.snapshot.chat_log.is_empty() {
        lines.push(Line::from(vec![Span::raw("Stream is empty. Send a message to begin!")]));
    } else {
        for (sender, message) in &app.snapshot.chat_log {
            if sender == "You" {
                lines.push(Line::from(vec![
                    Span::styled(format!("You: {message}"), Style::default().fg(COLOR_SELF_MSG).add_modifier(Modifier::BOLD))
                ]).alignment(Alignment::Right));
            } else {
                lines.push(Line::from(vec![
                    Span::styled(format!("{sender}: {message}"), Style::default().fg(COLOR_PEER_MSG).add_modifier(Modifier::BOLD))
                ]).alignment(Alignment::Left));
            }
        }
    }
    
    // Auto-scroll logic happens automatically because we populate it directly
    let log_view = Paragraph::new(lines).block(block);
    
    f.render_widget(log_view, area);
}
