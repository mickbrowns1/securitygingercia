use crate::editor::model::EditorDoc;
use crate::editor::schema_registry::{self, ComponentCategory, FieldKind, FieldSpec};
use crate::editor::templates;
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
        help: "Which receivers feed this pipeline, by id -- e.g. syslog/udp, file_log/app. Comma-separate multiple.",
    },
    FieldSpec {
        key: "exporters",
        kind: FieldKind::StringList,
        required: false,
        default: None,
        help: "Where processed events get sent, by exporter id -- e.g. splunk_hec/sentinelone. Comma-separate multiple to fan out to more than one.",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopTab {
    Receivers,
    Exporters,
    Extensions,
    Pipelines,
}

impl TopTab {
    fn category(self) -> Option<ComponentCategory> {
        match self {
            TopTab::Receivers => Some(ComponentCategory::Receiver),
            TopTab::Exporters => Some(ComponentCategory::Exporter),
            TopTab::Extensions => Some(ComponentCategory::Extension),
            TopTab::Pipelines => None,
        }
    }

    fn next(self) -> Self {
        match self {
            TopTab::Receivers => TopTab::Exporters,
            TopTab::Exporters => TopTab::Extensions,
            TopTab::Extensions => TopTab::Pipelines,
            TopTab::Pipelines => TopTab::Receivers,
        }
    }

    fn prev(self) -> Self {
        match self {
            TopTab::Receivers => TopTab::Pipelines,
            TopTab::Exporters => TopTab::Receivers,
            TopTab::Extensions => TopTab::Exporters,
            TopTab::Pipelines => TopTab::Extensions,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TopTab::Receivers => "Receivers",
            TopTab::Exporters => "Exporters",
            TopTab::Extensions => "Extensions",
            TopTab::Pipelines => "Pipelines",
        }
    }
}

pub enum FieldEditState {
    Text(Input),
    Bool(bool),
    Enum { options: &'static [&'static str], selected: usize },
    StringList(Input),
    /// A receiver's inline `operators:` list -- edited through its own
    /// sub-screen (see `Screen::OperatorList` and friends), never as
    /// text; this just holds the staged value between App transitions.
    OperatorList(Vec<Value>),
}

pub struct FormState {
    /// `None` for the synthetic "pipeline" pseudo-component, which has no
    /// type of its own. Used for display (the type's description) --
    /// whether it's actually written into the value is `write_type_key`.
    pub type_name: Option<&'static str>,
    /// OTel infers a receiver/exporter/extension's type from its id
    /// prefix alone, so those forms must NOT write a `type:` key.
    /// Operators have no id of their own, so their form must.
    pub write_type_key: bool,
    pub fields_spec: &'static [FieldSpec],
    pub fields: Vec<FieldEditState>,
    pub focused: usize,
}

impl FormState {
    pub fn new(
        type_name: Option<&'static str>,
        write_type_key: bool,
        fields_spec: &'static [FieldSpec],
        value: &Value,
    ) -> Self {
        let fields = fields_spec.iter().map(|spec| field_edit_state_from(spec, value)).collect();
        Self { type_name, write_type_key, fields_spec, fields, focused: 0 }
    }

    pub fn to_value(&self) -> Value {
        let mut map = Map::new();
        for (spec, state) in self.fields_spec.iter().zip(&self.fields) {
            match state {
                FieldEditState::Text(input) => {
                    let text = input.value();
                    if text.is_empty() {
                        continue;
                    }
                    schema_registry::set_path(&mut map, spec.key, json!(text));
                }
                FieldEditState::Bool(checked) => {
                    schema_registry::set_path(&mut map, spec.key, json!(*checked));
                }
                FieldEditState::Enum { options, selected } => {
                    schema_registry::set_path(&mut map, spec.key, json!(options[*selected]));
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
                    schema_registry::set_path(&mut map, spec.key, json!(items));
                }
                FieldEditState::OperatorList(ops) => {
                    if ops.is_empty() {
                        continue;
                    }
                    schema_registry::set_path(&mut map, spec.key, Value::Array(ops.clone()));
                }
            }
        }
        if self.write_type_key {
            if let Some(type_name) = self.type_name {
                map.insert("type".to_string(), json!(type_name));
            }
        }
        Value::Object(map)
    }

    fn focused_kind(&self) -> Option<FieldKind> {
        self.fields_spec.get(self.focused).map(|s| s.kind)
    }

    /// Moves focus to the next field -- used when leaving the operator
    /// sub-editor, so the natural "Esc, then Enter" gesture to finish
    /// editing operators and save the whole form actually submits it,
    /// rather than Enter re-drilling into the (still-focused) operators
    /// field.
    fn advance_focus(&mut self) {
        self.focused = (self.focused + 1) % self.fields.len().max(1);
    }

    /// Current value of this form's `operators:` field, if it has one.
    pub fn operators(&self) -> Vec<Value> {
        match self.operator_list_index().and_then(|i| self.fields.get(i)) {
            Some(FieldEditState::OperatorList(ops)) => ops.clone(),
            _ => Vec::new(),
        }
    }

    fn set_operators(&mut self, ops: Vec<Value>) {
        if let Some(i) = self.operator_list_index() {
            if let Some(FieldEditState::OperatorList(slot)) = self.fields.get_mut(i) {
                *slot = ops;
            }
        }
    }

    fn operator_list_index(&self) -> Option<usize> {
        self.fields_spec.iter().position(|s| matches!(s.kind, FieldKind::OperatorList))
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
                FieldEditState::Bool(checked) => {
                    if matches!(key.code, KeyCode::Left | KeyCode::Right | KeyCode::Char(' ')) {
                        *checked = !*checked;
                    }
                }
                FieldEditState::Text(input) | FieldEditState::StringList(input) => {
                    input.handle_event(&Event::Key(key));
                }
                FieldEditState::OperatorList(_) => {} // handled one level up (Enter drills in)
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

fn field_edit_state_from(spec: &FieldSpec, value: &Value) -> FieldEditState {
    let current = schema_registry::get_path(value, spec.key);
    match spec.kind {
        FieldKind::Enum(options) => {
            let current_str = current
                .and_then(|v| v.as_str())
                .or(spec.default)
                .unwrap_or(options[0]);
            let selected = options.iter().position(|o| *o == current_str).unwrap_or(0);
            FieldEditState::Enum { options, selected }
        }
        FieldKind::Bool => {
            let b = current
                .and_then(|v| v.as_bool())
                .or_else(|| spec.default.and_then(|d| d.parse::<bool>().ok()))
                .unwrap_or(false);
            FieldEditState::Bool(b)
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
        FieldKind::OperatorList => {
            let ops = current.and_then(|v| v.as_array()).cloned().unwrap_or_default();
            FieldEditState::OperatorList(ops)
        }
        FieldKind::Str | FieldKind::Duration => {
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

pub enum Screen {
    TopLevel { tab: TopTab, selected: usize },
    PickType { category: ComponentCategory, selected: usize },
    NameNewComponent { category: ComponentCategory, type_name: &'static str, input: Input },
    NameNewPipeline { input: Input },
    EditComponent { category: ComponentCategory, id: String, form: FormState },
    EditPipeline { id: String, form: FormState },
    ConfirmRemove { category: ComponentCategory, id: String, blocking: Vec<String> },
    /// The receiver/exporter/extension form being edited is `form`;
    /// `selected` indexes into its current `operators:` list.
    OperatorList { category: ComponentCategory, id: String, form: FormState, selected: usize },
    OperatorPickType { category: ComponentCategory, id: String, form: FormState, selected: usize },
    /// `index` is `None` for a brand new operator being appended, `Some`
    /// for editing an existing list entry in place.
    OperatorEdit {
        category: ComponentCategory,
        id: String,
        form: FormState,
        index: Option<usize>,
        op_form: FormState,
    },
    /// Curated source-template flow (Receivers tab only, key `T`):
    /// browse -> name -> fill params -> review the generated receiver ->
    /// confirm insert. See `templates.rs`.
    TemplateBrowse { selected: usize },
    NameTemplateReceiver { template_key: &'static str, input: Input },
    EditTemplateParams { template_key: &'static str, id: String, form: FormState },
    /// `params` is kept (not just the built `receiver`) so Esc here can
    /// return to `EditTemplateParams` pre-filled with what was already
    /// typed, instead of resetting to blank defaults.
    ReviewTemplateReceiver { template_key: &'static str, id: String, params: Value, receiver: Value },
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
    /// Full-screen keybinding reference, toggled by `?`. Drawn as an
    /// overlay on top of whatever `screen` currently is, which stays
    /// untouched underneath -- any key dismisses it and returns exactly
    /// where you were.
    pub show_help: bool,
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
            show_help: false,
        }
    }

    /// `?` opens the help overlay from a browsing/list screen, where it
    /// can't collide with typing a literal `?` into a text field --
    /// form screens (component/pipeline/operator editing, naming a new
    /// component) don't intercept it, so `?` types normally there.
    fn screen_accepts_help_hotkey(&self) -> bool {
        matches!(
            self.screen,
            Screen::TopLevel { .. }
                | Screen::PickType { .. }
                | Screen::ConfirmRemove { .. }
                | Screen::OperatorList { .. }
                | Screen::OperatorPickType { .. }
                | Screen::TemplateBrowse { .. }
        )
    }

    fn map_for(&self, category: ComponentCategory) -> &Map<String, Value> {
        match category {
            ComponentCategory::Receiver => &self.doc.receivers,
            ComponentCategory::Exporter => &self.doc.exporters,
            ComponentCategory::Extension => &self.doc.extensions,
        }
    }

    fn map_for_mut(&mut self, category: ComponentCategory) -> &mut Map<String, Value> {
        match category {
            ComponentCategory::Receiver => &mut self.doc.receivers,
            ComponentCategory::Exporter => &mut self.doc.exporters,
            ComponentCategory::Extension => &mut self.doc.extensions,
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
        if self.show_help {
            self.show_help = false;
            return;
        }
        if key.code == KeyCode::Char('?') && self.screen_accepts_help_hotkey() {
            self.show_help = true;
            return;
        }
        let screen = std::mem::take(&mut self.screen);
        self.screen = self.handle_key(screen, key);
    }

    fn handle_key(&mut self, screen: Screen, key: KeyEvent) -> Screen {
        match screen {
            Screen::TopLevel { tab, selected } => self.handle_top_level(tab, selected, key),
            Screen::PickType { category, selected } => self.handle_pick_type(category, selected, key),
            Screen::NameNewComponent { category, type_name, mut input } => match key.code {
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
                        let form = FormState::new(Some(type_name), false, spec.fields, &seed);
                        Screen::EditComponent { category, id, form }
                    }
                }
                _ => {
                    input.handle_event(&Event::Key(key));
                    Screen::NameNewComponent { category, type_name, input }
                }
            },
            Screen::NameNewPipeline { mut input } => match key.code {
                KeyCode::Esc => Screen::TopLevel { tab: TopTab::Pipelines, selected: 0 },
                KeyCode::Enter => {
                    let id = input.value().trim().to_string();
                    if id.is_empty() || self.doc.pipelines.contains_key(&id) {
                        self.status_message = Some(format!("'{id}' is empty or already exists"));
                        Screen::NameNewPipeline { input }
                    } else {
                        let form = FormState::new(None, false, PIPELINE_FIELDS, &json!({}));
                        Screen::EditPipeline { id, form }
                    }
                }
                _ => {
                    input.handle_event(&Event::Key(key));
                    Screen::NameNewPipeline { input }
                }
            },
            Screen::EditComponent { category, id, mut form } => {
                if key.code == KeyCode::Enter && matches!(form.focused_kind(), Some(FieldKind::OperatorList)) {
                    return Screen::OperatorList { category, id, form, selected: 0 };
                }
                match form.on_key(key) {
                    FormOutcome::Continue => Screen::EditComponent { category, id, form },
                    FormOutcome::Cancel => Screen::TopLevel { tab: tab_for(category), selected: 0 },
                    FormOutcome::Submit => {
                        let value = form.to_value();
                        self.map_for_mut(category).insert(id.clone(), value);
                        self.dirty = true;
                        self.status_message = Some(format!("{id} updated (press 's' to save to disk)"));
                        Screen::TopLevel { tab: tab_for(category), selected: 0 }
                    }
                }
            }
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
            Screen::OperatorList { category, id, form, selected } => {
                self.handle_operator_list(category, id, form, selected, key)
            }
            Screen::OperatorPickType { category, id, form, selected } => {
                self.handle_operator_pick_type(category, id, form, selected, key)
            }
            Screen::OperatorEdit { category, id, form, index, mut op_form } => match op_form.on_key(key) {
                FormOutcome::Continue => Screen::OperatorEdit { category, id, form, index, op_form },
                FormOutcome::Cancel => {
                    Screen::OperatorList { category, id, form, selected: index.unwrap_or(0) }
                }
                FormOutcome::Submit => {
                    let value = op_form.to_value();
                    let mut ops = form.operators();
                    let selected = match index {
                        Some(i) if i < ops.len() => {
                            ops[i] = value;
                            i
                        }
                        _ => {
                            ops.push(value);
                            ops.len() - 1
                        }
                    };
                    let mut form = form;
                    form.set_operators(ops);
                    Screen::OperatorList { category, id, form, selected }
                }
            },
            Screen::TemplateBrowse { selected } => self.handle_template_browse(selected, key),
            Screen::NameTemplateReceiver { template_key, mut input } => match key.code {
                KeyCode::Esc => Screen::TopLevel { tab: TopTab::Receivers, selected: 0 },
                KeyCode::Enter => {
                    let id = input.value().trim().to_string();
                    if id.is_empty() || self.doc.receivers.contains_key(&id) {
                        self.status_message = Some(format!("'{id}' is empty or already exists"));
                        Screen::NameTemplateReceiver { template_key, input }
                    } else {
                        let template = templates::find(template_key).expect("template_key comes from the registry");
                        let form = FormState::new(None, false, template.params, &json!({}));
                        Screen::EditTemplateParams { template_key, id, form }
                    }
                }
                _ => {
                    input.handle_event(&Event::Key(key));
                    Screen::NameTemplateReceiver { template_key, input }
                }
            },
            Screen::EditTemplateParams { template_key, id, mut form } => match form.on_key(key) {
                FormOutcome::Continue => Screen::EditTemplateParams { template_key, id, form },
                FormOutcome::Cancel => Screen::TopLevel { tab: TopTab::Receivers, selected: 0 },
                FormOutcome::Submit => {
                    let template = templates::find(template_key).expect("template_key comes from the registry");
                    let params = form.to_value();
                    let receiver = (template.build)(&params);
                    Screen::ReviewTemplateReceiver { template_key, id, params, receiver }
                }
            },
            Screen::ReviewTemplateReceiver { template_key, id, params, receiver } => match key.code {
                KeyCode::Enter => {
                    self.doc.receivers.insert(id.clone(), receiver);
                    self.dirty = true;
                    self.status_message = Some(format!("{id} created from template (press 's' to save to disk)"));
                    Screen::TopLevel { tab: TopTab::Receivers, selected: 0 }
                }
                KeyCode::Esc => {
                    let template = templates::find(template_key).expect("template_key comes from the registry");
                    let form = FormState::new(None, false, template.params, &params);
                    Screen::EditTemplateParams { template_key, id, form }
                }
                _ => Screen::ReviewTemplateReceiver { template_key, id, params, receiver },
            },
        }
    }

    fn handle_template_browse(&mut self, selected: usize, key: KeyEvent) -> Screen {
        let list = templates::SOURCE_TEMPLATES;
        match key.code {
            KeyCode::Esc => Screen::TopLevel { tab: TopTab::Receivers, selected: 0 },
            KeyCode::Down => Screen::TemplateBrowse {
                selected: if list.is_empty() { 0 } else { (selected + 1).min(list.len() - 1) },
            },
            KeyCode::Up => Screen::TemplateBrowse { selected: selected.saturating_sub(1) },
            KeyCode::Enter => {
                let Some(template) = list.get(selected) else {
                    return Screen::TemplateBrowse { selected };
                };
                Screen::NameTemplateReceiver {
                    template_key: template.key,
                    input: Input::new(template.default_id.to_string()),
                }
            }
            _ => Screen::TemplateBrowse { selected },
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
            KeyCode::Char('T') if tab == TopTab::Receivers => Screen::TemplateBrowse { selected: 0 },
            KeyCode::Enter => {
                let Some(id) = ids.get(selected).cloned() else {
                    return Screen::TopLevel { tab, selected };
                };
                match tab.category() {
                    Some(category) => {
                        let value = self.map_for(category)[&id].clone();
                        let type_name = schema_registry::component_type(&id);
                        let spec = schema_registry::types_for(category)
                            .iter()
                            .find(|s| s.type_name == type_name)
                            .or_else(|| schema_registry::types_for(category).first());
                        let Some(spec) = spec else {
                            return Screen::TopLevel { tab, selected };
                        };
                        let form = FormState::new(Some(spec.type_name), false, spec.fields, &value);
                        Screen::EditComponent { category, id, form }
                    }
                    None => {
                        let value = self.doc.pipelines[&id].clone();
                        let form = FormState::new(None, false, PIPELINE_FIELDS, &value);
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
                        let mut message = format!("save failed: {e}");
                        if let Some(hint) = save_error_hint(&message) {
                            message.push_str(hint);
                        }
                        self.status_message = Some(message);
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

    fn handle_operator_list(
        &mut self,
        category: ComponentCategory,
        id: String,
        form: FormState,
        selected: usize,
        key: KeyEvent,
    ) -> Screen {
        let ops = form.operators();
        match key.code {
            KeyCode::Esc => {
                let mut form = form;
                form.advance_focus();
                Screen::EditComponent { category, id, form }
            }
            KeyCode::Down => {
                let next = if ops.is_empty() { 0 } else { (selected + 1).min(ops.len() - 1) };
                Screen::OperatorList { category, id, form, selected: next }
            }
            KeyCode::Up => Screen::OperatorList { category, id, form, selected: selected.saturating_sub(1) },
            KeyCode::Char('a') => Screen::OperatorPickType { category, id, form, selected: 0 },
            KeyCode::Enter => {
                let Some(op_value) = ops.get(selected).cloned() else {
                    return Screen::OperatorList { category, id, form, selected };
                };
                let type_name = op_value.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let Some(spec) = schema_registry::operator_type(type_name) else {
                    return Screen::OperatorList { category, id, form, selected };
                };
                let op_form = FormState::new(Some(spec.type_name), true, spec.fields, &op_value);
                Screen::OperatorEdit { category, id, form, index: Some(selected), op_form }
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                let mut form = form;
                if selected < ops.len() {
                    let mut ops = ops;
                    ops.remove(selected);
                    form.set_operators(ops);
                }
                Screen::OperatorList { category, id, form, selected: 0 }
            }
            _ => Screen::OperatorList { category, id, form, selected },
        }
    }

    fn handle_operator_pick_type(
        &mut self,
        category: ComponentCategory,
        id: String,
        form: FormState,
        selected: usize,
        key: KeyEvent,
    ) -> Screen {
        let types = schema_registry::operator_types();
        match key.code {
            KeyCode::Esc => Screen::OperatorList { category, id, form, selected: 0 },
            KeyCode::Down => Screen::OperatorPickType {
                category,
                id,
                form,
                selected: if types.is_empty() { 0 } else { (selected + 1).min(types.len() - 1) },
            },
            KeyCode::Up => Screen::OperatorPickType { category, id, form, selected: selected.saturating_sub(1) },
            KeyCode::Enter => {
                let Some(spec) = types.get(selected) else {
                    return Screen::OperatorPickType { category, id, form, selected };
                };
                let seed = schema_registry::minimal_value(spec);
                let op_form = FormState::new(Some(spec.type_name), true, spec.fields, &seed);
                Screen::OperatorEdit { category, id, form, index: None, op_form }
            }
            _ => Screen::OperatorPickType { category, id, form, selected },
        }
    }
}

fn tab_for(category: ComponentCategory) -> TopTab {
    match category {
        ComponentCategory::Receiver => TopTab::Receivers,
        ComponentCategory::Exporter => TopTab::Exporters,
        ComponentCategory::Extension => TopTab::Extensions,
    }
}

// The real binary's own error text for this is accurate but easy to miss
// the implication of if you're not already expecting it -- most people
// hit this by testing a config (that still has the example's
// windows_event_log/security receiver) on a non-Windows box, so spell out
// the fix right where the failure shows up instead of just the raw error.
fn save_error_hint(message: &str) -> Option<&'static str> {
    if message.contains("windows_event_log") {
        Some(" -- windows_event_log only works if sgcia-otelcol runs on Windows; remove it and its pipeline to test elsewhere")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    #[test]
    fn save_error_hint_flags_the_windows_event_log_pipeline_build_failure() {
        let message = "save failed: invalid config: Error: failed to build pipelines: \
            failed to create \"windows_event_log/security\" receiver for data type \"logs\": \
            windows eventlog receiver is only supported on Windows";
        let hint = save_error_hint(message).expect("should recognize this failure");
        assert!(hint.contains("Windows"));
    }

    #[test]
    fn save_error_hint_is_none_for_unrelated_failures() {
        assert!(save_error_hint("save failed: invalid config: Error: requires a non-empty \"token\"").is_none());
    }

    /// FILE_LOG_FIELDS order is include, exclude, start_at, poll_interval,
    /// storage, operators -- `operators` is index 5, so 5 Tabs from a
    /// freshly opened EditComponent (focused=0) land on it.
    fn tab_to_operators_field(app: &mut App) {
        for _ in 0..5 {
            app.on_key(key(KeyCode::Tab));
        }
    }

    fn sample_doc() -> EditorDoc {
        EditorDoc::parse(
            r#"
receivers:
  file_log/app:
    include: ["/var/log/app/*.log"]
    operators:
      - type: add
        field: attributes.sourcetype
        value: myapp
exporters:
  splunk_hec/sentinelone:
    endpoint: "https://example.invalid/services/collector/event"
    token: "tok"
extensions:
  file_storage:
    directory: /var/lib/sgcia/otelcol-storage
service:
  extensions: [file_storage]
  pipelines:
    logs/app:
      receivers: [file_log/app]
      exporters: [splunk_hec/sentinelone]
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
    fn question_mark_opens_help_from_top_level_and_any_key_dismisses_it() {
        let mut app = App::new(PathBuf::from("x.yaml"), sample_doc());
        app.on_key(key(KeyCode::Char('?')));
        assert!(app.show_help);
        // The underlying screen must be untouched while help is showing.
        assert!(matches!(app.screen, Screen::TopLevel { .. }));

        app.on_key(key(KeyCode::Char('z'))); // any key, not specifically Esc
        assert!(!app.show_help);
        assert!(matches!(app.screen, Screen::TopLevel { .. }));
    }

    #[test]
    fn question_mark_types_literally_into_a_focused_text_field() {
        let mut app = App::new(PathBuf::from("x.yaml"), sample_doc());
        app.on_key(key(KeyCode::Char('a'))); // PickType
        app.on_key(key(KeyCode::Enter)); // pick first receiver type -> NameNewComponent
        app.on_key(key(KeyCode::Char('?'))); // should type '?', not open help
        assert!(!app.show_help);
        match &app.screen {
            Screen::NameNewComponent { input, .. } => assert_eq!(input.value(), "?"),
            _ => panic!("expected NameNewComponent"),
        }
    }

    #[test]
    fn tab_cycles_through_top_tabs() {
        let mut app = App::new(PathBuf::from("x.yaml"), sample_doc());
        app.on_key(key(KeyCode::Tab));
        assert!(matches!(app.screen, Screen::TopLevel { tab: TopTab::Exporters, .. }));
        app.on_key(key(KeyCode::Tab));
        assert!(matches!(app.screen, Screen::TopLevel { tab: TopTab::Extensions, .. }));
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
        app.on_key(key(KeyCode::Enter)); // open file_log/app for edit, focused on `include`
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
                assert_eq!(id, "file_log/app");
                assert_eq!(blocking, &vec!["logs/app".to_string()]);
            }
            _ => panic!("expected ConfirmRemove"),
        }
        assert!(app.doc.receivers.contains_key("file_log/app"), "not removed yet");

        app.on_key(key(KeyCode::Char('y')));
        assert!(!app.doc.receivers.contains_key("file_log/app"));
        assert!(
            !app.doc.pipelines["logs/app"]["receivers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "file_log/app"),
            "dangling reference must be stripped from the pipeline too"
        );
    }

    #[test]
    fn removing_an_extension_never_requires_confirmation() {
        let mut app = App::new(PathBuf::from("x.yaml"), sample_doc());
        app.on_key(key(KeyCode::Tab)); // Exporters
        app.on_key(key(KeyCode::Tab)); // Extensions
        app.on_key(key(KeyCode::Char('d')));
        assert!(!app.doc.extensions.contains_key("file_storage"));
        assert!(matches!(app.screen, Screen::TopLevel { tab: TopTab::Extensions, .. }));
    }

    #[test]
    fn add_component_flow_creates_a_new_receiver_without_a_type_key() {
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

        let value = app.doc.receivers.get("syslog/new").expect("receiver created");
        assert!(value.get("type").is_none(), "OTel infers type from the id, not a type: key");
    }

    #[test]
    fn entering_operator_list_field_opens_operator_list_screen() {
        let mut app = App::new(PathBuf::from("x.yaml"), sample_doc());
        app.on_key(key(KeyCode::Enter)); // open file_log/app for edit, focused on `include`
        tab_to_operators_field(&mut app);
        app.on_key(key(KeyCode::Enter)); // drill into the operator list
        assert!(matches!(app.screen, Screen::OperatorList { .. }));
    }

    #[test]
    fn operator_list_shows_the_existing_add_operator() {
        let mut app = App::new(PathBuf::from("x.yaml"), sample_doc());
        app.on_key(key(KeyCode::Enter));
        tab_to_operators_field(&mut app);
        app.on_key(key(KeyCode::Enter));
        match &app.screen {
            Screen::OperatorList { form, .. } => {
                let ops = form.operators();
                assert_eq!(ops.len(), 1);
                assert_eq!(ops[0]["type"], "add");
            }
            _ => panic!("expected OperatorList"),
        }
    }

    #[test]
    fn adding_a_new_operator_persists_through_to_the_component() {
        let mut app = App::new(PathBuf::from("x.yaml"), sample_doc());
        app.on_key(key(KeyCode::Enter)); // EditComponent file_log/app
        tab_to_operators_field(&mut app);
        app.on_key(key(KeyCode::Enter)); // OperatorList
        app.on_key(key(KeyCode::Char('a'))); // OperatorPickType
        assert!(matches!(app.screen, Screen::OperatorPickType { .. }));

        app.on_key(key(KeyCode::Down)); // move off `add` to `remove` (or wherever) -- just exercise navigation
        app.on_key(key(KeyCode::Up));
        app.on_key(key(KeyCode::Enter)); // pick `add` -> OperatorEdit
        assert!(matches!(app.screen, Screen::OperatorEdit { index: None, .. }));

        app.on_key(key(KeyCode::Enter)); // submit op form as-is (seeded defaults) -> back to OperatorList
        match &app.screen {
            Screen::OperatorList { form, .. } => assert_eq!(form.operators().len(), 2),
            _ => panic!("expected OperatorList"),
        }

        app.on_key(key(KeyCode::Esc)); // back to EditComponent, focus advanced off `operators`
        app.on_key(key(KeyCode::Enter)); // submit the whole component form

        let value = app.doc.receivers.get("file_log/app").unwrap();
        assert_eq!(value["operators"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn removing_an_operator_from_the_list() {
        let mut app = App::new(PathBuf::from("x.yaml"), sample_doc());
        app.on_key(key(KeyCode::Enter));
        tab_to_operators_field(&mut app);
        app.on_key(key(KeyCode::Enter)); // OperatorList with 1 item
        app.on_key(key(KeyCode::Char('d')));
        match &app.screen {
            Screen::OperatorList { form, .. } => assert!(form.operators().is_empty()),
            _ => panic!("expected OperatorList"),
        }
    }

    #[test]
    fn template_flow_creates_a_new_receiver_from_the_first_template() {
        let mut app = App::new(PathBuf::from("x.yaml"), sample_doc());
        app.on_key(key(KeyCode::Char('T')));
        assert!(matches!(app.screen, Screen::TemplateBrowse { .. }));

        app.on_key(key(KeyCode::Enter)); // pick the first template (cisco_asa) -> NameTemplateReceiver, prefilled
        assert!(matches!(app.screen, Screen::NameTemplateReceiver { .. }));

        app.on_key(key(KeyCode::Enter)); // accept the prefilled id -> EditTemplateParams
        assert!(matches!(app.screen, Screen::EditTemplateParams { .. }));

        app.on_key(key(KeyCode::Enter)); // submit params as-is (seeded defaults) -> ReviewTemplateReceiver
        assert!(matches!(app.screen, Screen::ReviewTemplateReceiver { .. }));

        app.on_key(key(KeyCode::Enter)); // confirm insert -> TopLevel
        assert!(matches!(app.screen, Screen::TopLevel { tab: TopTab::Receivers, .. }));

        let value = app.doc.receivers.get("syslog/cisco_asa").expect("receiver created from template");
        assert_eq!(value["protocol"], "rfc3164");
        assert!(value["operators"].as_array().unwrap().len() >= 3);
    }

    #[test]
    fn esc_on_review_returns_to_params_with_previous_edits_preserved() {
        let mut app = App::new(PathBuf::from("x.yaml"), sample_doc());
        app.on_key(key(KeyCode::Char('T')));
        app.on_key(key(KeyCode::Enter)); // NameTemplateReceiver
        app.on_key(key(KeyCode::Enter)); // EditTemplateParams, focused on `transport` (first field)
        app.on_key(key(KeyCode::Right)); // flip transport from udp -> tcp
        app.on_key(key(KeyCode::Enter)); // submit -> ReviewTemplateReceiver
        match &app.screen {
            Screen::ReviewTemplateReceiver { receiver, .. } => {
                assert!(receiver.get("tcp").is_some(), "tcp transport should be reflected in the built receiver");
            }
            _ => panic!("expected ReviewTemplateReceiver"),
        }

        app.on_key(key(KeyCode::Esc)); // back to EditTemplateParams, not reset to blank
        match &app.screen {
            Screen::EditTemplateParams { form, .. } => match &form.fields[0] {
                FieldEditState::Enum { options, selected } => assert_eq!(options[*selected], "tcp"),
                _ => panic!("expected Enum field"),
            },
            _ => panic!("expected EditTemplateParams"),
        }
    }

    #[test]
    fn capital_t_only_opens_template_browse_on_receivers_tab() {
        let mut app = App::new(PathBuf::from("x.yaml"), sample_doc());
        app.on_key(key(KeyCode::Tab)); // switch to Exporters tab
        app.on_key(key(KeyCode::Char('T')));
        assert!(
            matches!(app.screen, Screen::TopLevel { tab: TopTab::Exporters, .. }),
            "T should be a no-op outside the Receivers tab"
        );
    }
}
