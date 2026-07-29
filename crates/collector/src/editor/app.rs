use crate::editor::model::EditorDoc;
use crate::editor::schema_registry::{self, ComponentCategory, FieldKind, FieldSpec};
use crossterm::event::{Event, KeyCode, KeyEvent};
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use tui_input::backend::crossterm::EventHandler;
use tui_input::Input;

const PIPELINE_FIELDS: &[FieldSpec] = &[
    FieldSpec {
        key: "receivers",
        kind: FieldKind::StringList,
        required: false,
        default: None,
        help: "Which receivers feed this pipeline, by id -- e.g. syslog/udp, filelog/app. Comma-separate multiple.",
    },
    FieldSpec {
        key: "operators",
        kind: FieldKind::StringList,
        required: false,
        default: None,
        help: "Which operators to run, in order, by id -- e.g. extract_asa_fields, add_datasource. Comma-separate multiple; leave blank to skip parsing entirely.",
    },
    FieldSpec {
        key: "exporters",
        kind: FieldKind::StringList,
        required: false,
        default: None,
        help: "Where processed events get sent, by exporter id -- e.g. sentinelone_hec. Comma-separate multiple to fan out to more than one.",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopTab {
    Receivers,
    Operators,
    Exporters,
    Pipelines,
}

impl TopTab {
    fn category(self) -> Option<ComponentCategory> {
        match self {
            TopTab::Receivers => Some(ComponentCategory::Receiver),
            TopTab::Operators => Some(ComponentCategory::Operator),
            TopTab::Exporters => Some(ComponentCategory::Exporter),
            TopTab::Pipelines => None,
        }
    }

    fn next(self) -> Self {
        match self {
            TopTab::Receivers => TopTab::Operators,
            TopTab::Operators => TopTab::Exporters,
            TopTab::Exporters => TopTab::Pipelines,
            TopTab::Pipelines => TopTab::Receivers,
        }
    }

    fn prev(self) -> Self {
        match self {
            TopTab::Receivers => TopTab::Pipelines,
            TopTab::Operators => TopTab::Receivers,
            TopTab::Exporters => TopTab::Operators,
            TopTab::Pipelines => TopTab::Exporters,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TopTab::Receivers => "Receivers",
            TopTab::Operators => "Operators",
            TopTab::Exporters => "Exporters",
            TopTab::Pipelines => "Pipelines",
        }
    }
}

pub enum FieldEditState {
    Text(Input),
    Enum { options: &'static [&'static str], selected: usize },
    StringList(Input),
    RawJson(Input),
}

pub struct FormState {
    /// `None` for the synthetic "pipeline" pseudo-component, which has no
    /// `type` key of its own.
    pub type_name: Option<&'static str>,
    pub fields_spec: &'static [FieldSpec],
    pub fields: Vec<FieldEditState>,
    pub focused: usize,
}

impl FormState {
    pub fn new(type_name: Option<&'static str>, fields_spec: &'static [FieldSpec], value: &Value) -> Self {
        let fields = fields_spec
            .iter()
            .map(|spec| field_edit_state_from(spec, value.get(spec.key)))
            .collect();
        Self { type_name, fields_spec, fields, focused: 0 }
    }

    pub fn to_value(&self) -> Value {
        let mut map = Map::new();
        if let Some(type_name) = self.type_name {
            map.insert("type".to_string(), json!(type_name));
        }
        for (spec, state) in self.fields_spec.iter().zip(&self.fields) {
            match state {
                FieldEditState::Text(input) => {
                    let text = input.value();
                    if text.is_empty() {
                        continue;
                    }
                    map.insert(spec.key.to_string(), parse_scalar(spec.kind, text));
                }
                FieldEditState::Enum { options, selected } => {
                    map.insert(spec.key.to_string(), json!(options[*selected]));
                }
                FieldEditState::StringList(input) => {
                    let items: Vec<&str> = input
                        .value()
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .collect();
                    if items.is_empty() {
                        continue;
                    }
                    map.insert(spec.key.to_string(), json!(items));
                }
                FieldEditState::RawJson(input) => {
                    let text = input.value();
                    if text.trim().is_empty() {
                        continue;
                    }
                    let parsed = serde_json::from_str(text).unwrap_or_else(|_| json!(text));
                    map.insert(spec.key.to_string(), parsed);
                }
            }
        }
        Value::Object(map)
    }

    fn on_key(&mut self, key: KeyEvent) -> FormOutcome {
        match key.code {
            KeyCode::Enter => return FormOutcome::Submit,
            KeyCode::Esc => return FormOutcome::Cancel,
            KeyCode::Tab => {
                self.focused = (self.focused + 1) % self.fields.len().max(1);
                return FormOutcome::Continue;
            }
            KeyCode::BackTab => {
                self.focused = self.focused.checked_sub(1).unwrap_or(self.fields.len().saturating_sub(1));
                return FormOutcome::Continue;
            }
            _ => {}
        }
        if let Some(field) = self.fields.get_mut(self.focused) {
            match field {
                FieldEditState::Enum { options, selected } => match key.code {
                    KeyCode::Left => *selected = selected.checked_sub(1).unwrap_or(options.len() - 1),
                    KeyCode::Right => *selected = (*selected + 1) % options.len(),
                    _ => {}
                },
                FieldEditState::Text(input) | FieldEditState::StringList(input) | FieldEditState::RawJson(input) => {
                    input.handle_event(&Event::Key(key));
                }
            }
        }
        FormOutcome::Continue
    }
}

enum FormOutcome {
    Continue,
    Submit,
    Cancel,
}

fn field_edit_state_from(spec: &FieldSpec, current: Option<&Value>) -> FieldEditState {
    match spec.kind {
        FieldKind::Enum(options) => {
            let current_str = current
                .and_then(|v| v.as_str())
                .or(spec.default)
                .unwrap_or(options[0]);
            let selected = options.iter().position(|o| *o == current_str).unwrap_or(0);
            FieldEditState::Enum { options, selected }
        }
        FieldKind::StringList => {
            let joined = current
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            FieldEditState::StringList(Input::new(joined))
        }
        FieldKind::Map => {
            let text = current
                .map(|v| v.to_string())
                .unwrap_or_else(|| "{}".to_string());
            FieldEditState::RawJson(Input::new(text))
        }
        FieldKind::Str | FieldKind::Int | FieldKind::Duration => {
            let text = current
                .map(value_to_display_string)
                .or_else(|| spec.default.map(str::to_string))
                .unwrap_or_default();
            FieldEditState::Text(Input::new(text))
        }
    }
}

fn value_to_display_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn parse_scalar(kind: FieldKind, text: &str) -> Value {
    match kind {
        FieldKind::Int => text.parse::<i64>().map(|n| json!(n)).unwrap_or_else(|_| json!(text)),
        _ => json!(text),
    }
}

pub enum Screen {
    TopLevel { tab: TopTab, selected: usize },
    PickType { category: ComponentCategory, selected: usize },
    NameNewComponent { category: ComponentCategory, type_name: &'static str, input: Input },
    NameNewPipeline { input: Input },
    EditComponent { category: ComponentCategory, id: String, form: FormState },
    EditPipeline { id: String, form: FormState },
    ConfirmRemove { category: ComponentCategory, id: String, blocking: Vec<String> },
}

impl Default for Screen {
    fn default() -> Self {
        Screen::TopLevel { tab: TopTab::Receivers, selected: 0 }
    }
}

pub struct App {
    pub config_path: PathBuf,
    pub doc: EditorDoc,
    pub screen: Screen,
    pub status_message: Option<String>,
    pub dirty: bool,
    pub should_quit: bool,
}

impl App {
    pub fn new(config_path: PathBuf, doc: EditorDoc) -> Self {
        Self {
            config_path,
            doc,
            screen: Screen::default(),
            status_message: None,
            dirty: false,
            should_quit: false,
        }
    }

    fn map_for(&self, category: ComponentCategory) -> &Map<String, Value> {
        match category {
            ComponentCategory::Receiver => &self.doc.receivers,
            ComponentCategory::Operator => &self.doc.operators,
            ComponentCategory::Exporter => &self.doc.exporters,
        }
    }

    fn map_for_mut(&mut self, category: ComponentCategory) -> &mut Map<String, Value> {
        match category {
            ComponentCategory::Receiver => &mut self.doc.receivers,
            ComponentCategory::Operator => &mut self.doc.operators,
            ComponentCategory::Exporter => &mut self.doc.exporters,
        }
    }

    fn ids_for_tab(&self, tab: TopTab) -> Vec<String> {
        let mut ids: Vec<String> = match tab.category() {
            Some(category) => self.map_for(category).keys().cloned().collect(),
            None => self.doc.pipelines.keys().cloned().collect(),
        };
        ids.sort();
        ids
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        let screen = std::mem::take(&mut self.screen);
        self.screen = self.handle_key(screen, key);
    }

    fn handle_key(&mut self, screen: Screen, key: KeyEvent) -> Screen {
        match screen {
            Screen::TopLevel { tab, selected } => self.handle_top_level(tab, selected, key),
            Screen::PickType { category, selected } => self.handle_pick_type(category, selected, key),
            Screen::NameNewComponent { category, type_name, mut input } => {
                match key.code {
                    KeyCode::Esc => Screen::TopLevel { tab: tab_for(category), selected: 0 },
                    KeyCode::Enter => {
                        let id = input.value().trim().to_string();
                        if id.is_empty() || self.map_for(category).contains_key(&id) {
                            self.status_message = Some(format!("'{id}' is empty or already exists"));
                            Screen::NameNewComponent { category, type_name, input }
                        } else {
                            let spec = schema_registry::types_for(category)
                                .iter()
                                .find(|s| s.type_name == type_name)
                                .expect("type_name comes from the registry");
                            let seed = schema_registry::minimal_value(spec);
                            let form = FormState::new(Some(type_name), spec.fields, &seed);
                            Screen::EditComponent { category, id, form }
                        }
                    }
                    _ => {
                        input.handle_event(&Event::Key(key));
                        Screen::NameNewComponent { category, type_name, input }
                    }
                }
            }
            Screen::NameNewPipeline { mut input } => match key.code {
                KeyCode::Esc => Screen::TopLevel { tab: TopTab::Pipelines, selected: 0 },
                KeyCode::Enter => {
                    let id = input.value().trim().to_string();
                    if id.is_empty() || self.doc.pipelines.contains_key(&id) {
                        self.status_message = Some(format!("'{id}' is empty or already exists"));
                        Screen::NameNewPipeline { input }
                    } else {
                        let form = FormState::new(None, PIPELINE_FIELDS, &json!({}));
                        Screen::EditPipeline { id, form }
                    }
                }
                _ => {
                    input.handle_event(&Event::Key(key));
                    Screen::NameNewPipeline { input }
                }
            },
            Screen::EditComponent { category, id, mut form } => match form.on_key(key) {
                FormOutcome::Continue => Screen::EditComponent { category, id, form },
                FormOutcome::Cancel => Screen::TopLevel { tab: tab_for(category), selected: 0 },
                FormOutcome::Submit => {
                    let value = form.to_value();
                    self.map_for_mut(category).insert(id.clone(), value);
                    self.dirty = true;
                    self.status_message = Some(format!("{id} updated (press 's' to save to disk)"));
                    Screen::TopLevel { tab: tab_for(category), selected: 0 }
                }
            },
            Screen::EditPipeline { id, mut form } => match form.on_key(key) {
                FormOutcome::Continue => Screen::EditPipeline { id, form },
                FormOutcome::Cancel => Screen::TopLevel { tab: TopTab::Pipelines, selected: 0 },
                FormOutcome::Submit => {
                    let value = form.to_value();
                    self.doc.pipelines.insert(id.clone(), value);
                    self.dirty = true;
                    self.status_message = Some(format!("{id} updated (press 's' to save to disk)"));
                    Screen::TopLevel { tab: TopTab::Pipelines, selected: 0 }
                }
            },
            Screen::ConfirmRemove { category, id, blocking: _ } => match key.code {
                KeyCode::Char('y') => {
                    self.doc.strip_from_pipelines(category, &id);
                    self.map_for_mut(category).remove(&id);
                    self.dirty = true;
                    self.status_message = Some(format!("removed {id}"));
                    Screen::TopLevel { tab: tab_for(category), selected: 0 }
                }
                _ => Screen::TopLevel { tab: tab_for(category), selected: 0 },
            },
        }
    }

    fn handle_top_level(&mut self, tab: TopTab, selected: usize, key: KeyEvent) -> Screen {
        let ids = self.ids_for_tab(tab);
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.should_quit = true;
                Screen::TopLevel { tab, selected }
            }
            KeyCode::Tab => Screen::TopLevel { tab: tab.next(), selected: 0 },
            KeyCode::BackTab => Screen::TopLevel { tab: tab.prev(), selected: 0 },
            KeyCode::Down => {
                let next = if ids.is_empty() { 0 } else { (selected + 1).min(ids.len() - 1) };
                Screen::TopLevel { tab, selected: next }
            }
            KeyCode::Up => Screen::TopLevel { tab, selected: selected.saturating_sub(1) },
            KeyCode::Char('a') => match tab.category() {
                Some(category) => Screen::PickType { category, selected: 0 },
                None => Screen::NameNewPipeline { input: Input::default() },
            },
            KeyCode::Enter => {
                let Some(id) = ids.get(selected).cloned() else {
                    return Screen::TopLevel { tab, selected };
                };
                match tab.category() {
                    Some(category) => {
                        let value = self.map_for(category)[&id].clone();
                        let type_name = component_type_name(category, &id, &value);
                        let spec = schema_registry::types_for(category)
                            .iter()
                            .find(|s| s.type_name == type_name)
                            .or_else(|| schema_registry::types_for(category).first());
                        let Some(spec) = spec else {
                            return Screen::TopLevel { tab, selected };
                        };
                        let form = FormState::new(Some(spec.type_name), spec.fields, &value);
                        Screen::EditComponent { category, id, form }
                    }
                    None => {
                        let value = self.doc.pipelines[&id].clone();
                        let form = FormState::new(None, PIPELINE_FIELDS, &value);
                        Screen::EditPipeline { id, form }
                    }
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                let Some(id) = ids.get(selected).cloned() else {
                    return Screen::TopLevel { tab, selected };
                };
                match tab.category() {
                    Some(category) => {
                        let blocking = self.doc.pipelines_referencing(category, &id);
                        if blocking.is_empty() {
                            self.map_for_mut(category).remove(&id);
                            self.dirty = true;
                            self.status_message = Some(format!("removed {id}"));
                            Screen::TopLevel { tab, selected: 0 }
                        } else {
                            Screen::ConfirmRemove { category, id, blocking }
                        }
                    }
                    None => {
                        self.doc.pipelines.remove(&id);
                        self.dirty = true;
                        self.status_message = Some(format!("removed {id}"));
                        Screen::TopLevel { tab, selected: 0 }
                    }
                }
            }
            KeyCode::Char('s') => {
                match self.doc.save(&self.config_path) {
                    Ok(()) => {
                        self.dirty = false;
                        self.status_message = Some(format!("saved {}", self.config_path.display()));
                    }
                    Err(e) => {
                        self.status_message = Some(format!("save failed: {e}"));
                    }
                }
                Screen::TopLevel { tab, selected }
            }
            _ => Screen::TopLevel { tab, selected },
        }
    }

    fn handle_pick_type(&mut self, category: ComponentCategory, selected: usize, key: KeyEvent) -> Screen {
        let types = schema_registry::types_for(category);
        match key.code {
            KeyCode::Esc => Screen::TopLevel { tab: tab_for(category), selected: 0 },
            KeyCode::Down => Screen::PickType {
                category,
                selected: if types.is_empty() { 0 } else { (selected + 1).min(types.len() - 1) },
            },
            KeyCode::Up => Screen::PickType { category, selected: selected.saturating_sub(1) },
            KeyCode::Enter => {
                let Some(spec) = types.get(selected) else {
                    return Screen::PickType { category, selected };
                };
                Screen::NameNewComponent {
                    category,
                    type_name: spec.type_name,
                    input: Input::default(),
                }
            }
            _ => Screen::PickType { category, selected },
        }
    }
}

fn tab_for(category: ComponentCategory) -> TopTab {
    match category {
        ComponentCategory::Receiver => TopTab::Receivers,
        ComponentCategory::Operator => TopTab::Operators,
        ComponentCategory::Exporter => TopTab::Exporters,
    }
}

/// Determines a component's registry type name the same way `build.rs`
/// actually dispatches it at runtime: receivers are typed by their id's
/// `type/name` prefix (they have no `type:` field of their own),
/// exporters and operators by an explicit `type:` key in their value
/// (falling back to the id prefix only because `build_exporter` does).
/// Getting this wrong means editing an existing component silently
/// resolves to the wrong type's form.
fn component_type_name(category: ComponentCategory, id: &str, value: &Value) -> String {
    match category {
        ComponentCategory::Receiver => sg_config::component_type(id).to_string(),
        ComponentCategory::Exporter | ComponentCategory::Operator => value
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| sg_config::component_type(id))
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    fn sample_doc() -> EditorDoc {
        EditorDoc::parse(
            r#"
receivers:
  filelog/app:
    include: ["/var/log/app/*.log"]
    checkpoint_file: "/tmp/app.checkpoint.json"
exporters:
  sentinelone_hec:
    type: s1hec
    endpoint: "https://example.invalid/services/collector/event"
    token: "tok"
    sourcetype: "app_ts_parser"
service:
  pipelines:
    logs/app:
      receivers: [filelog/app]
      exporters: [sentinelone_hec]
"#,
        )
        .unwrap()
    }

    #[test]
    fn q_requests_quit_from_top_level() {
        let mut app = App::new(PathBuf::from("x.yaml"), sample_doc());
        app.on_key(key(KeyCode::Char('q')));
        assert!(app.should_quit);
    }

    #[test]
    fn tab_cycles_through_top_tabs() {
        let mut app = App::new(PathBuf::from("x.yaml"), sample_doc());
        app.on_key(key(KeyCode::Tab));
        assert!(matches!(app.screen, Screen::TopLevel { tab: TopTab::Operators, .. }));
    }

    #[test]
    fn enter_opens_edit_component_for_selected_receiver() {
        let mut app = App::new(PathBuf::from("x.yaml"), sample_doc());
        app.on_key(key(KeyCode::Enter));
        assert!(matches!(
            app.screen,
            Screen::EditComponent { category: ComponentCategory::Receiver, .. }
        ));
    }

    #[test]
    fn editing_and_submitting_a_component_marks_doc_dirty() {
        let mut app = App::new(PathBuf::from("x.yaml"), sample_doc());
        app.on_key(key(KeyCode::Enter)); // open filelog/app for edit
        app.on_key(key(KeyCode::Enter)); // submit immediately (no field changes)
        assert!(app.dirty);
        assert!(matches!(app.screen, Screen::TopLevel { .. }));
    }

    #[test]
    fn removing_a_referenced_receiver_requires_confirmation() {
        let mut app = App::new(PathBuf::from("x.yaml"), sample_doc());
        app.on_key(key(KeyCode::Char('d')));
        match &app.screen {
            Screen::ConfirmRemove { id, blocking, .. } => {
                assert_eq!(id, "filelog/app");
                assert_eq!(blocking, &vec!["logs/app".to_string()]);
            }
            _ => panic!("expected ConfirmRemove"),
        }
        assert!(app.doc.receivers.contains_key("filelog/app"), "not removed yet");

        app.on_key(key(KeyCode::Char('y')));
        assert!(!app.doc.receivers.contains_key("filelog/app"));
        assert!(
            !app.doc.pipelines["logs/app"]["receivers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "filelog/app"),
            "dangling reference must be stripped from the pipeline too"
        );
    }

    #[test]
    fn add_component_flow_creates_a_new_receiver() {
        let mut app = App::new(PathBuf::from("x.yaml"), sample_doc());
        app.on_key(key(KeyCode::Char('a')));
        assert!(matches!(app.screen, Screen::PickType { category: ComponentCategory::Receiver, .. }));

        app.on_key(key(KeyCode::Enter)); // pick the first receiver type (syslog)
        assert!(matches!(app.screen, Screen::NameNewComponent { .. }));

        for c in "syslog/new".chars() {
            app.on_key(key(KeyCode::Char(c)));
        }
        app.on_key(key(KeyCode::Enter)); // confirm name -> EditComponent

        assert!(matches!(app.screen, Screen::EditComponent { .. }));
        app.on_key(key(KeyCode::Enter)); // submit form as-is (seeded defaults)

        assert!(app.doc.receivers.contains_key("syslog/new"));
    }
}
