use crate::editor::app::{App, FieldEditState, FormState, Screen, TopTab};
use crate::editor::schema_registry::{self, ComponentCategory};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

const PIPELINE_DESCRIPTION: &str = "A pipeline wires everything together: events come in from the listed receivers, flow through the listed operators in order, then get sent out to the listed exporters.";

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(area);

    match &app.screen {
        Screen::TopLevel { tab, selected } => draw_top_level(frame, chunks[0], app, *tab, *selected),
        Screen::PickType { category, selected } => draw_pick_type(frame, chunks[0], *category, *selected),
        Screen::NameNewComponent { category, type_name, input } => draw_name_prompt(
            frame,
            chunks[0],
            &format!("new {type_name} id"),
            input.value(),
            describe_type(*category, type_name),
        ),
        Screen::NameNewPipeline { input } => {
            draw_name_prompt(frame, chunks[0], "new pipeline id", input.value(), PIPELINE_DESCRIPTION)
        }
        Screen::EditComponent { category, id, form } => {
            let description = form.type_name.map(|t| describe_type(*category, t)).unwrap_or("");
            draw_form(frame, chunks[0], &format!("editing {id}"), description, form)
        }
        Screen::EditPipeline { id, form } => {
            draw_form(frame, chunks[0], &format!("editing pipeline {id}"), PIPELINE_DESCRIPTION, form)
        }
        Screen::ConfirmRemove { id, blocking, .. } => draw_confirm_remove(frame, chunks[0], id, blocking),
    }

    draw_status_bar(frame, chunks[1], app);
}

fn describe_type(category: ComponentCategory, type_name: &str) -> &'static str {
    schema_registry::types_for(category)
        .iter()
        .find(|s| s.type_name == type_name)
        .map(|s| s.description)
        .unwrap_or("")
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

fn draw_pick_type(frame: &mut Frame, area: Rect, category: ComponentCategory, selected: usize) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("pick a type (Up/Down to move, Enter to confirm, Esc to cancel)");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let types = schema_registry::types_for(category);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(types.len().max(1) as u16), Constraint::Min(2)])
        .split(inner);

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
    frame.render_widget(List::new(items), chunks[0]);

    let description = types.get(selected).map(|s| s.description).unwrap_or("");
    frame.render_widget(
        Paragraph::new(description)
            .style(Style::default().fg(Color::Cyan))
            .wrap(Wrap { trim: true }),
        chunks[1],
    );
}

fn draw_name_prompt(frame: &mut Frame, area: Rect, title: &str, value: &str, description: &str) {
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Min(2)])
        .split(inner);

    frame.render_widget(Paragraph::new(value), chunks[0]);
    frame.render_widget(
        Paragraph::new(description)
            .style(Style::default().fg(Color::Cyan))
            .wrap(Wrap { trim: true }),
        chunks[2],
    );
}

fn draw_form(frame: &mut Frame, area: Rect, title: &str, description: &str, form: &FormState) {
    // Render the bordered box first and lay everything else out inside
    // its *inner* rect (`block.inner(area)`, not `area` itself) --
    // otherwise the field text gets drawn under the border and its
    // leftmost character(s) get clipped by it.
    let block = Block::default().borders(Borders::ALL).title(format!(
        "{title} -- Tab: next field  Left/Right: cycle choice  Enter: save  Esc: cancel"
    ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let field_count = form.fields.len().max(1) as u16;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(if description.is_empty() { 0 } else { 2 }),
            Constraint::Length(field_count),
            Constraint::Length(1),
            Constraint::Min(2),
        ])
        .split(inner);

    if !description.is_empty() {
        frame.render_widget(
            Paragraph::new(description)
                .style(Style::default().add_modifier(Modifier::ITALIC))
                .wrap(Wrap { trim: true }),
            chunks[0],
        );
    }

    let field_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Length(1); form.fields.len().max(1)])
        .split(chunks[1]);

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
            Span::styled(format!("{:<20}", spec.key), style),
            Span::raw(value_text),
        ]);
        if let Some(row) = field_rows.get(i) {
            frame.render_widget(Paragraph::new(line), *row);
        }
    }

    let focused_help = form.fields_spec.get(form.focused).map(|s| s.help).unwrap_or("");
    frame.render_widget(
        Paragraph::new(focused_help)
            .style(Style::default().fg(Color::Cyan))
            .wrap(Wrap { trim: true }),
        chunks[3],
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

    #[test]
    fn edit_component_form_shows_field_names_uncut_and_help_text() {
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
        let mut app = App::new(PathBuf::from("x.yaml"), doc);
        app.on_key(crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Enter));
        let content = render_to_string(&app);

        // The bug this regression-tests: the bordered box was drawn over
        // the same raw area as the field text (not its inset `inner()`
        // rect), clipping the leftmost character of every field label.
        assert!(content.contains("include"), "field label must not be clipped");
        assert!(content.contains("checkpoint_file"));
        // Help text for whichever field starts focused should be visible
        // somewhere on screen.
        assert!(content.contains("glob patterns") || content.contains("checkpoint"));
    }

    #[test]
    fn pick_type_screen_shows_description_of_selected_type() {
        let app = App::new(PathBuf::from("x.yaml"), EditorDoc::new());
        let mut app = app;
        app.on_key(crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Char('a')));
        let content = render_to_string(&app);
        assert!(content.contains("syslog"));
        assert!(content.contains("Listens for syslog messages"));
    }
}
