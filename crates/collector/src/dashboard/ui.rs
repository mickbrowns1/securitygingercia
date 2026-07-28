use crate::dashboard::app::App;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use ratatui::Frame;
use sg_core::MetricsSnapshot;

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
            Constraint::Percentage(34),
        ])
        .split(area);

    draw_header(frame, chunks[0], app);

    match &app.last_snapshot {
        Some(snap) => {
            draw_receivers(frame, chunks[1], snap);
            draw_pipelines(frame, chunks[2], snap);
            draw_exporters(frame, chunks[3], snap);
        }
        None => {
            let msg = Paragraph::new("waiting for first status snapshot...").block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("sgcia dashboard"),
            );
            frame.render_widget(msg, chunks[1]);
        }
    }
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let (text, style) = match &app.last_error {
        Some(e) => (
            format!("! unreachable: {} -- {e} (retrying)", app.status_addr),
            Style::default().fg(Color::Red),
        ),
        None => (
            format!("connected to {}", app.status_addr),
            Style::default().fg(Color::Green),
        ),
    };
    let para = Paragraph::new(Line::from(Span::styled(text, style))).block(
        Block::default()
            .borders(Borders::ALL)
            .title("sgcia dashboard (q to quit)"),
    );
    frame.render_widget(para, area);
}

fn draw_receivers(frame: &mut Frame, area: Rect, snap: &MetricsSnapshot) {
    let mut names: Vec<&String> = snap.receivers.keys().collect();
    names.sort();
    let rows = names.into_iter().map(|name| {
        let r = &snap.receivers[name];
        Row::new(vec![
            Cell::from(name.clone()),
            Cell::from(r.events_in.to_string()),
        ])
    });
    let table = Table::new(rows, [Constraint::Percentage(70), Constraint::Percentage(30)])
        .header(
            Row::new(vec!["receiver", "events in"])
                .style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .block(Block::default().borders(Borders::ALL).title("Receivers"));
    frame.render_widget(table, area);
}

fn draw_pipelines(frame: &mut Frame, area: Rect, snap: &MetricsSnapshot) {
    let mut names: Vec<&String> = snap.pipelines.keys().collect();
    names.sort();
    let rows = names.into_iter().map(|name| {
        let p = &snap.pipelines[name];
        Row::new(vec![
            Cell::from(name.clone()),
            Cell::from(p.events_in.to_string()),
            Cell::from(p.events_out.to_string()),
            Cell::from(p.events_dropped.to_string()),
            Cell::from(p.parse_errors.to_string()),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(30),
            Constraint::Percentage(17),
            Constraint::Percentage(17),
            Constraint::Percentage(18),
            Constraint::Percentage(18),
        ],
    )
    .header(
        Row::new(vec!["pipeline", "in", "out", "dropped", "parse errors"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    )
    .block(Block::default().borders(Borders::ALL).title("Pipelines"));
    frame.render_widget(table, area);
}

fn draw_exporters(frame: &mut Frame, area: Rect, snap: &MetricsSnapshot) {
    let mut names: Vec<&String> = snap.exporters.keys().collect();
    names.sort();
    let rows = names.into_iter().map(|name| {
        let e = &snap.exporters[name];
        let last_error = e
            .last_error
            .as_ref()
            .map(|le| le.message.clone())
            .unwrap_or_default();
        Row::new(vec![
            Cell::from(name.clone()),
            Cell::from(e.batches_sent.to_string()),
            Cell::from(e.retries.to_string()),
            Cell::from(e.batches_failed.to_string()),
            Cell::from(last_error),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(20),
            Constraint::Percentage(12),
            Constraint::Percentage(12),
            Constraint::Percentage(12),
            Constraint::Percentage(44),
        ],
    )
    .header(
        Row::new(vec!["exporter", "sent", "retries", "failed", "last error"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    )
    .block(Block::default().borders(Borders::ALL).title("Exporters"));
    frame.render_widget(table, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::net::SocketAddr;

    fn addr() -> SocketAddr {
        "127.0.0.1:7801".parse().unwrap()
    }

    #[test]
    fn renders_waiting_message_before_first_snapshot() {
        let app = App::new(addr());
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();

        let content: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(content.contains("waiting for first status snapshot"));
    }

    #[test]
    fn renders_error_banner_when_unreachable() {
        let mut app = App::new(addr());
        app.update(crate::dashboard::app::AppEvent::PollResult(Err(
            "Connection refused".to_string(),
        )));
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();

        let content: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(content.contains("unreachable"));
        assert!(content.contains("Connection refused"));
    }
}
