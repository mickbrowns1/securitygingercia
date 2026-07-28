use crate::editor::app::{App, FieldEditState, FormState, Screen, TopTab};
use crate::editor::schema_registry;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(area);

    match &app.screen {
        Screen::TopLevel { tab, selected } => draw_top_level(frame, chunks[0], app, *tab, *selected),
        Screen::PickType { category, selected } => draw_pick_type(frame, chunks[0], *category, *selected),
        Screen::NameNewComponent { type_name, input, .. } => {
            draw_name_prompt(frame, chunks[0], &format!("new {type_name} id"), input.value())
        }
        Screen::NameNewPipeline { input } => draw_name_prompt(frame, chunks[0], "new pipeline id", input.value()),
        Screen::EditComponent { id, form, .. } => draw_form(frame, chunks[0], &format!("editing {id}"), form),
        Screen::EditPipeline { id, form } => draw_form(frame, chunks[0], &format!("editing pipeline {id}"), form),
        Screen::ConfirmRemove { id, blocking, .. } => draw_confirm_remove(frame, chunks[0], id, blocking),
    }

    draw_status_bar(frame, chunks[1], app);
}

fn draw_top_level(frame: &mut Frame, area: Rect, app: &App, tab: TopTab, selected: usize) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(area);

    let tabs = [TopTab::Receivers, TopTab::Operators, TopTab::Exporters, TopTab::Pipelines];
    let tab_line = Line::from(
        tabs.iter()
            .map(|t| {
                let label = format!(" {} ", t.label());
                if *t == tab {
                    Span::styled(label, Style::default().add_modifier(Modifier::REVERSED))
                } else {
                    Span::raw(label)
                }
            })
            .collect::<Vec<_>>(),
    );
    frame.render_widget(
        Paragraph::new(tab_line).block(Block::default().borders(Borders::ALL).title("sgcia config editor")),
        chunks[0],
    );

    let ids = ids_for_display(app, tab);
    let items: Vec<ListItem> = if ids.is_empty() {
        vec![ListItem::new("(none -- press 'a' to add one)")]
    } else {
        ids.iter()
            .enumerate()
            .map(|(i, id)| {
                let style = if i == selected {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                };
                ListItem::new(id.clone()).style(style)
            })
            .collect()
    };
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title("a: add  Enter: edit  d: remove  s: save  Tab: switch  q: quit"),
    );
    frame.render_widget(list, chunks[1]);
}

fn ids_for_display(app: &App, tab: TopTab) -> Vec<String> {
    let mut ids: Vec<String> = match tab {
        TopTab::Receivers => app.doc.receivers.keys().cloned().collect(),
        TopTab::Operators => app.doc.operators.keys().cloned().collect(),
        TopTab::Exporters => app.doc.exporters.keys().cloned().collect(),
        TopTab::Pipelines => app.doc.pipelines.keys().cloned().collect(),
    };
    ids.sort();
    ids
}

fn draw_pick_type(frame: &mut Frame, area: Rect, category: schema_registry::ComponentCategory, selected: usize) {
    let types = schema_registry::types_for(category);
    let items: Vec<ListItem> = types
        .iter()
        .enumerate()
        .map(|(i, spec)| {
            let style = if i == selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            ListItem::new(spec.type_name).style(style)
        })
        .collect();
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title("pick a type (Enter to confirm, Esc to cancel)"),
    );
    frame.render_widget(list, area);
}

fn draw_name_prompt(frame: &mut Frame, area: Rect, title: &str, value: &str) {
    let para = Paragraph::new(value).block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(para, area);
}

fn draw_form(frame: &mut Frame, area: Rect, title: &str, form: &FormState) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Length(1); form.fields.len().max(1)])
        .split(area);

    for (i, (spec, state)) in form.fields_spec.iter().zip(&form.fields).enumerate() {
        let value_text = match state {
            FieldEditState::Text(input) | FieldEditState::StringList(input) | FieldEditState::RawJson(input) => {
                input.value().to_string()
            }
            FieldEditState::Enum { options, selected } => options[*selected].to_string(),
        };
        let focused = i == form.focused;
        let style = if focused {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let line = Line::from(vec![
            Span::styled(format!("{:<16}", spec.key), style),
            Span::raw(value_text),
        ]);
        if let Some(row) = rows.get(i) {
            frame.render_widget(Paragraph::new(line), *row);
        }
    }

    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("{title} -- Tab: next field  Enter: save  Esc: cancel")),
        area,
    );
}

fn draw_confirm_remove(frame: &mut Frame, area: Rect, id: &str, blocking: &[String]) {
    let text = format!(
        "'{id}' is still used by pipeline(s): {}\n\nPress 'y' to remove it anyway (and strip it from those pipelines), any other key to cancel.",
        blocking.join(", ")
    );
    let para = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .title("confirm removal")
            .style(Style::default().fg(Color::Red)),
    );
    frame.render_widget(para, area);
}

fn draw_status_bar(frame: &mut Frame, area: Rect, app: &App) {
    let dirty_marker = if app.dirty { " [unsaved changes]" } else { "" };
    let text = match &app.status_message {
        Some(msg) => format!("{msg}{dirty_marker}"),
        None => format!("{}{dirty_marker}", app.config_path.display()),
    };
    frame.render_widget(Paragraph::new(text).block(Block::default().borders(Borders::ALL)), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::model::EditorDoc;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;

    fn render_to_string(app: &App) -> String {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn top_level_shows_placeholder_when_empty() {
        let app = App::new(PathBuf::from("x.yaml"), EditorDoc::new());
        let content = render_to_string(&app);
        assert!(content.contains("press 'a' to add one"));
    }

    #[test]
    fn top_level_lists_existing_receivers() {
        let doc = EditorDoc::parse(
            r#"
receivers:
  filelog/app:
    include: ["/var/log/app/*.log"]
    checkpoint_file: "/tmp/app.checkpoint.json"
service:
  pipelines: {}
"#,
        )
        .unwrap();
        let app = App::new(PathBuf::from("x.yaml"), doc);
        let content = render_to_string(&app);
        assert!(content.contains("filelog/app"));
    }

    #[test]
    fn status_bar_shows_unsaved_marker_when_dirty() {
        let mut app = App::new(PathBuf::from("x.yaml"), EditorDoc::new());
        app.dirty = true;
        let content = render_to_string(&app);
        assert!(content.contains("unsaved changes"));
    }
}
