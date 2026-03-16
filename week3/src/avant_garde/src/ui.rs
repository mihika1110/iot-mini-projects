use ratatui::{
    crossterm::event::{self, Event, KeyCode},
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    symbols,
    text::Span,
    widgets::{
        canvas::{Canvas, Circle, Line, Rectangle},
        Axis, Block, Borders, Chart, Dataset, GraphType, List, ListItem, ListState, Paragraph,
    },
    Frame,
};
use std::{sync::Arc, time::Duration};
use tokio::sync::Mutex;
use crate::agent::{AgentStore, AgentState, AgentInfo, Coordinate};
use crate::logger::Logger;
use crate::localizer::{LocalizationResult, WeightedEstimate, localize_windowed, WindowNode};

#[derive(PartialEq)]
enum InputMode {
    Normal,
    Positioning,
}

struct AppState {
    list_state: ListState,
    input_mode: InputMode,
    input_x: String,
    input_y: String,
    focus_x: bool,
}

impl AppState {
    fn new() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            list_state,
            input_mode: InputMode::Normal,
            input_x: String::new(),
            input_y: String::new(),
            focus_x: true,
        }
    }
}

pub async fn run(store: Arc<Mutex<AgentStore>>) -> std::io::Result<()> {
    let mut terminal = ratatui::init();
    let mut app = AppState::new();
    
    loop {
        // Collect data
        let mut agents = {
            let s = store.lock().await;
            s.get_agents()
        };
        agents.sort_by(|a, b| a.id.cmp(&b.id));

        let selected_id = app.list_state.selected().and_then(|i| agents.get(i).map(|a| a.id.clone()));
        
        let mut distances: Vec<(f64, f64)> = Vec::new();
        let mut activations: Vec<(f64, f64)> = Vec::new();
        
        if let Some(id) = &selected_id {
            let s = store.lock().await;
            if let Ok(dist) = s.get_distance_slice(id) {
                distances = dist.into_iter().enumerate().map(|(i, d)| (i as f64, d as f64)).collect();
            }
            if let Ok(act) = s.get_activation_slice(id) {
                activations = act.into_iter().enumerate().map(|(i, a)| (i as f64, a as f64)).collect();
            }
        }

        let logs = Logger::global().get_logs();

        // Build windowed localization data from all active agents with positions.
        const LOC_WINDOW: usize = 32;
        let loc_estimates = {
            let s = store.lock().await;
            let window_nodes: Vec<crate::localizer::WindowNode> = agents.iter()
                .filter(|a| a.state == AgentState::ACTIVE && a.position.is_some())
                .filter_map(|a| {
                    let dists = s.get_distance_slice(&a.id).ok()?;
                    let movs  = s.get_activation_slice(&a.id).ok()?;
                    if dists.is_empty() { return None; }
                    Some(crate::localizer::WindowNode {
                        pos: a.position.unwrap(),
                        distances: dists,
                        movements: movs,
                    })
                })
                .collect();
            crate::localizer::localize_windowed(&window_nodes, LOC_WINDOW)
        };

        terminal.draw(|f| ui(f, &mut app, &agents, &distances, &activations, &logs, &loc_estimates))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match app.input_mode {
                    InputMode::Normal => match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Down => {
                            let i = match app.list_state.selected() {
                                Some(i) => if agents.is_empty() { 0 } else { (i + 1) % agents.len() },
                                None => 0,
                            };
                            app.list_state.select(Some(i));
                        }
                        KeyCode::Up => {
                            let i = match app.list_state.selected() {
                                Some(i) => if agents.is_empty() { 0 } else { if i == 0 { agents.len() - 1 } else { i - 1 } },
                                None => 0,
                            };
                            app.list_state.select(Some(i));
                        }
                        KeyCode::Char('p') => {
                            if selected_id.is_some() {
                                app.input_mode = InputMode::Positioning;
                                app.input_x.clear();
                                app.input_y.clear();
                                app.focus_x = true;
                            }
                        }
                        _ => {}
                    },
                    InputMode::Positioning => match key.code {
                        KeyCode::Enter => {
                            if !app.focus_x {
                                // Submit
                                if let (Some(id), Ok(x), Ok(y)) = (
                                    &selected_id,
                                    app.input_x.parse::<f32>(),
                                    app.input_y.parse::<f32>(),
                                ) {
                                    let mut s = store.lock().await;
                                    s.set_agent_position(id, Coordinate { x, y });
                                    crate::logger::info(&format!("Manual position set for {}: ({}, {})", id, x, y));
                                    app.input_mode = InputMode::Normal;
                                } else {
                                    crate::logger::error("Invalid X or Y coordinate input");
                                    app.input_mode = InputMode::Normal;
                                }
                            } else {
                                app.focus_x = false;
                            }
                        }
                        KeyCode::Esc => {
                            app.input_mode = InputMode::Normal;
                        }
                        KeyCode::Tab => {
                            app.focus_x = !app.focus_x;
                        }
                        KeyCode::Char(c) => {
                            if app.focus_x {
                                app.input_x.push(c);
                            } else {
                                app.input_y.push(c);
                            }
                        }
                        KeyCode::Backspace => {
                            if app.focus_x {
                                app.input_x.pop();
                            } else {
                                app.input_y.pop();
                            }
                        }
                        _ => {}
                    },
                }
            }
        }
    }
    ratatui::restore();
    Ok(())
}

fn ui(
    f: &mut Frame,
    app: &mut AppState,
    agents: &[AgentInfo],
    distances: &[(f64, f64)],
    activations: &[(f64, f64)],
    logs: &[String],
    locs: &[crate::localizer::WeightedEstimate],
) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)].as_ref())
        .split(f.area());

    // Left List
    let items: Vec<ListItem> = agents
        .iter()
        .map(|a| {
            let status = if a.state == AgentState::ACTIVE { "🟢" } else { "🔴" };
            let pos = if let Some(p) = &a.position { format!("({:.1},{:.1})", p.x, p.y) } else { "No Pos".to_string() };
            ListItem::new(format!("{} {} [{}]", status, a.id, pos))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().title("Agents (p to set pos)").borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD).fg(Color::Yellow))
        .highlight_symbol("> ");

    f.render_stateful_widget(list, chunks[0], &mut app.list_state);

    // Right Area
    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(35), // Data Plot
            Constraint::Percentage(35), // Map Canvas
            Constraint::Percentage(30), // Logs
        ].as_ref())
        .split(chunks[1]);

    // Data Plot
    let dist_dataset = Dataset::default()
        .name("Distance")
        .marker(symbols::Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(Color::Cyan))
        .data(distances);

    let act_dataset = Dataset::default()
        .name("Movement")
        .marker(symbols::Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(Color::Magenta))
        .data(activations);

    let max_dist = distances.iter().map(|&(_, y)| y).fold(10.0f64, f64::max);
    let max_x = distances.len().max(10) as f64;

    let chart = Chart::new(vec![dist_dataset, act_dataset])
        .block(Block::default().title("Agent Data").borders(Borders::ALL))
        .x_axis(Axis::default().title("Samples").bounds([0.0, max_x]).labels(vec![Span::from("0"), Span::from(format!("{}", max_x))]))
        .y_axis(Axis::default().title("Value").bounds([0.0, max_dist]).labels(vec![Span::from("0"), Span::from(format!("{:.1}", max_dist))]));

    f.render_widget(chart, right_chunks[0]);

    // Map Canvas — auto-scale to bounding box of all placed agents
    let positioned: Vec<(f64, f64)> = agents.iter()
        .filter_map(|a| a.position.map(|p| (p.x as f64, p.y as f64)))
        .collect();

    let (x_min, x_max, y_min, y_max) = if positioned.is_empty() {
        (-10.0, 110.0, -10.0, 60.0) // default field
    } else {
        let xs = positioned.iter().map(|&(x, _)| x);
        let ys = positioned.iter().map(|&(_, y)| y);
        let x0 = xs.clone().fold(f64::INFINITY, f64::min);
        let x1 = xs.fold(f64::NEG_INFINITY, f64::max);
        let y0 = ys.clone().fold(f64::INFINITY, f64::min);
        let y1 = ys.fold(f64::NEG_INFINITY, f64::max);
        // add 15% padding on each side; ensure a minimum span of 10 units
        let pad_x = ((x1 - x0) * 0.15).max(5.0);
        let pad_y = ((y1 - y0) * 0.15).max(5.0);
        (x0 - pad_x, x1 + pad_x, y0 - pad_y, y1 + pad_y)
    };

    let map = Canvas::default()
        .block(Block::default().title("Agent Positions").borders(Borders::ALL))
        .marker(symbols::Marker::Braille)
        .x_bounds([x_min, x_max])
        .y_bounds([y_min, y_max])
        .paint(|ctx| {
            // draw a faint bounding rectangle spanning all agent positions
            if !positioned.is_empty() {
                let (wx, wy) = (x_min + (x_max - x_min) * 0.0, y_min + (y_max - y_min) * 0.0);
                ctx.draw(&Rectangle {
                    x: x_min,
                    y: y_min,
                    width: x_max - x_min,
                    height: y_max - y_min,
                    color: Color::DarkGray,
                });
                let _ = (wx, wy); // suppress unused warning
            }
            for agent in agents {
                if let Some(pos) = &agent.position {
                    let color = if agent.state == AgentState::ACTIVE { Color::Green } else { Color::Red };
                    ctx.print(pos.x as f64, pos.y as f64, Span::styled("●", Style::default().fg(color)));
                    ctx.print(pos.x as f64, pos.y as f64 - 2.0, agent.id.clone());
                }
            }

            // ── Localization overlay ──────────────────────────────────────────
            for est in locs {
                let weight = est.weight;
                let color = if weight > 0.8 {
                    Color::LightGreen
                } else if weight > 0.5 {
                    Color::Yellow
                } else if weight > 0.2 {
                    Color::Rgb(100, 100, 0) // Dim yellow
                } else {
                    Color::Rgb(50, 50, 50)  // Faded grey
                };

                match &est.result {
                    LocalizationResult::NoData => {}

                    LocalizationResult::Circle { center, radius } => {
                        ctx.draw(&Circle {
                            x: center.x as f64,
                            y: center.y as f64,
                            radius: *radius as f64,
                            color,
                        });
                    }

                    LocalizationResult::Arc { p1, p2, midpoint } => {
                        // Draw the chord between the two intersection candidates
                        ctx.draw(&Line {
                            x1: p1.x as f64, y1: p1.y as f64,
                            x2: p2.x as f64, y2: p2.y as f64,
                            color,
                        });
                        // Mark each candidate point
                        ctx.print(p1.x as f64, p1.y as f64,
                            Span::styled("◇", Style::default().fg(color)));
                        ctx.print(p2.x as f64, p2.y as f64,
                            Span::styled("◇", Style::default().fg(color)));
                        // Mark midpoint
                        ctx.print(midpoint.x as f64, midpoint.y as f64,
                            Span::styled("✦", Style::default().fg(color)));
                    }

                    LocalizationResult::Point { pos, .. } => {
                        // Crosshair at estimated position
                        ctx.draw(&Line {
                            x1: pos.x as f64 - 1.5, y1: pos.y as f64,
                            x2: pos.x as f64 + 1.5, y2: pos.y as f64,
                            color,
                        });
                        ctx.draw(&Line {
                            x1: pos.x as f64, y1: pos.y as f64 - 1.5,
                            x2: pos.x as f64, y2: pos.y as f64 + 1.5,
                            color,
                        });
                        ctx.print(pos.x as f64 + 0.5, pos.y as f64 + 0.5,
                            Span::styled("★", Style::default().fg(color)));
                    }
                }
            }
        });
    f.render_widget(map, right_chunks[1]);

    // — Localization overlay label ———————————————-
    // Show info about the MOST RECENT estimate
    let loc_label = if let Some(last) = locs.last() {
        match &last.result {
            LocalizationResult::NoData => "Localization: no active nodes".to_string(),
            LocalizationResult::Circle { radius, .. } =>
                format!("Localization: 1 node — circle r={:.1} (w={:.2})", radius, last.weight),
            LocalizationResult::Arc { midpoint, .. } => 
                format!("Localization: 2 nodes — chord mid=({:.1},{:.1}) (w={:.2})", midpoint.x, midpoint.y, last.weight),
            LocalizationResult::Point { pos, residual } =>
                format!("Localization: 3+ node fix → ({:.1},{:.1}) err={:.2} (w={:.2})",
                    pos.x, pos.y, residual, last.weight),
        }
    } else {
        "Localization: waiting for motion...".to_string()
    };
    crate::logger::info(&loc_label);

    // Logs
    let log_content = logs.join("\n");
    let log_paragraph = Paragraph::new(log_content)
        .block(Block::default().title("System Logs").borders(Borders::ALL))
        .wrap(ratatui::widgets::Wrap { trim: true });
    f.render_widget(log_paragraph, right_chunks[2]);

    // Input Popup
    if app.input_mode == InputMode::Positioning {
        let area = centered_rect(60, 20, f.area());
        f.render_widget(ratatui::widgets::Clear, area); // This clears the background
        let input_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Length(3)].as_ref())
            .margin(1)
            .split(area);

        let x_style = if app.focus_x { Style::default().fg(Color::Yellow) } else { Style::default() };
        let y_style = if !app.focus_x { Style::default().fg(Color::Yellow) } else { Style::default() };

        let x_input = Paragraph::new(app.input_x.as_str())
            .style(x_style)
            .block(Block::default().borders(Borders::ALL).title("Enter X Coordinate"));
        f.render_widget(x_input, input_chunks[0]);

        let y_input = Paragraph::new(app.input_y.as_str())
            .style(y_style)
            .block(Block::default().borders(Borders::ALL).title("Enter Y Coordinate (TAB to switch, ENTER to submit)"));
        f.render_widget(y_input, input_chunks[1]);
    }
}

/// helper function to create a centered rect using up certain percentage of the available rect `r`
fn centered_rect(percent_x: u16, percent_y: u16, r: ratatui::layout::Rect) -> ratatui::layout::Rect {
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


