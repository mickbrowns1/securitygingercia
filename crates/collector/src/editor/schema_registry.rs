//! Advisory field schema for the config editor's forms, mechanically
//! transcribed from the real `*ConfigDef` serde structs each component
//! crate already defines. This registry only decides which widget a
//! field gets (text box vs enum picker); it is never the source of
//! truth for validity -- `*Config::from_value` is called at save time
//! regardless, so registry/type drift degrades to "worse form widget,"
//! never "silently invalid config."

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentCategory {
    Receiver,
    Exporter,
    Operator,
}

#[derive(Debug, Clone, Copy)]
pub enum FieldKind {
    Str,
    Int,
    Duration,
    Enum(&'static [&'static str]),
    StringList,
    /// Opaque nested object (e.g. `static_fields`, `batch`, `retry`),
    /// edited as raw inline JSON text rather than its own sub-form.
    Map,
}

#[derive(Debug, Clone, Copy)]
pub struct FieldSpec {
    pub key: &'static str,
    pub kind: FieldKind,
    pub required: bool,
    pub default: Option<&'static str>,
}

#[derive(Debug, Clone, Copy)]
pub struct ComponentTypeSpec {
    pub type_name: &'static str,
    pub fields: &'static [FieldSpec],
}

const fn f(key: &'static str, kind: FieldKind, required: bool, default: Option<&'static str>) -> FieldSpec {
    FieldSpec { key, kind, required, default }
}

const ON_ERROR: FieldSpec = f("on_error", FieldKind::Enum(&["drop", "pass", "dead_letter"]), false, Some("pass"));

// --- Receivers ---

const SYSLOG_FIELDS: &[FieldSpec] = &[
    f("protocol", FieldKind::Enum(&["udp", "tcp"]), true, None),
    // Defaulted (rather than a bare placeholder) since it must parse as a
    // real `SocketAddr` even in the drift test / a freshly-added component.
    f("listen_address", FieldKind::Str, true, Some("0.0.0.0:514")),
    f("rfc", FieldKind::Enum(&["auto", "rfc3164", "rfc5424"]), false, Some("auto")),
    f("framing", FieldKind::Enum(&["auto", "octet_counting", "non_transparent"]), false, Some("auto")),
    f("max_message_size", FieldKind::Int, false, Some("65536")),
];

const FILELOG_FIELDS: &[FieldSpec] = &[
    f("include", FieldKind::StringList, true, None),
    f("exclude", FieldKind::StringList, false, None),
    f("start_at", FieldKind::Enum(&["beginning", "end"]), false, Some("end")),
    f("poll_interval", FieldKind::Duration, false, Some("500ms")),
    f("checkpoint_file", FieldKind::Str, true, None),
];

const WINDOWS_EVENTLOG_FIELDS: &[FieldSpec] = &[
    f("channel", FieldKind::Str, true, None),
    f("query", FieldKind::Str, false, Some("*")),
    f("start_at", FieldKind::Enum(&["beginning", "end"]), false, Some("end")),
    f("bookmark_file", FieldKind::Str, true, None),
];

// --- Exporters ---

const S1HEC_FIELDS: &[FieldSpec] = &[
    // Defaulted since it must parse as a real URL even in the drift test.
    f("endpoint", FieldKind::Str, true, Some("https://example.invalid/services/collector/event")),
    f("token", FieldKind::Str, true, None),
    f("sourcetype", FieldKind::Str, true, None),
    f("datasource", FieldKind::Str, false, None),
    f("msgid_field", FieldKind::Str, false, None),
    f("static_fields", FieldKind::Map, false, None),
    f("batch", FieldKind::Map, false, None),
    f("retry", FieldKind::Map, false, None),
];

const SPLUNKHEC_FIELDS: &[FieldSpec] = &[
    // Defaulted since it must parse as a real URL even in the drift test.
    f("endpoint", FieldKind::Str, true, Some("https://splunk.example.invalid:8088/services/collector/event")),
    f("token", FieldKind::Str, true, None),
    f("source", FieldKind::Str, false, Some("sgcia")),
    f("sourcetype_field", FieldKind::Str, false, None),
    f("sourcetype", FieldKind::Str, false, None),
    f("index", FieldKind::Str, false, None),
    f("batch", FieldKind::Map, false, None),
    f("retry", FieldKind::Map, false, None),
];

const STDOUT_FIELDS: &[FieldSpec] = &[];

// --- Operators ---

const REGEX_FIELDS: &[FieldSpec] = &[
    f("pattern", FieldKind::Str, true, None),
    f("parse_from", FieldKind::Str, true, None),
    f("parse_to", FieldKind::Str, true, None),
    ON_ERROR,
];

const JSON_FIELDS: &[FieldSpec] = &[
    f("parse_from", FieldKind::Str, true, None),
    f("parse_to", FieldKind::Str, true, None),
    ON_ERROR,
];

const KV_FIELDS: &[FieldSpec] = &[
    f("parse_from", FieldKind::Str, true, None),
    f("parse_to", FieldKind::Str, true, None),
    f("pair_delimiter", FieldKind::Str, false, Some(" ")),
    f("kv_delimiter", FieldKind::Str, false, Some("=")),
    ON_ERROR,
];

const SEVERITY_FIELDS: &[FieldSpec] = &[
    f("parse_from", FieldKind::Str, true, None),
    f("preset", FieldKind::Enum(&["syslog"]), true, Some("syslog")),
    ON_ERROR,
];

const TIMESTAMP_FIELDS: &[FieldSpec] = &[
    f("parse_from", FieldKind::Str, true, None),
    f("layout", FieldKind::Str, true, None),
    ON_ERROR,
];

const ADD_FIELDS: &[FieldSpec] = &[
    f("field", FieldKind::Str, true, None),
    f("value", FieldKind::Str, true, None),
];

const REMOVE_FIELDS: &[FieldSpec] = &[f("field", FieldKind::Str, true, None)];

const FROM_TO_FIELDS: &[FieldSpec] = &[
    f("from", FieldKind::Str, true, None),
    f("to", FieldKind::Str, true, None),
];

const RECEIVER_TYPES: &[ComponentTypeSpec] = &[
    ComponentTypeSpec { type_name: "syslog", fields: SYSLOG_FIELDS },
    ComponentTypeSpec { type_name: "filelog", fields: FILELOG_FIELDS },
    ComponentTypeSpec { type_name: "windows_eventlog", fields: WINDOWS_EVENTLOG_FIELDS },
];

const EXPORTER_TYPES: &[ComponentTypeSpec] = &[
    ComponentTypeSpec { type_name: "s1hec", fields: S1HEC_FIELDS },
    ComponentTypeSpec { type_name: "splunkhec", fields: SPLUNKHEC_FIELDS },
    ComponentTypeSpec { type_name: "stdout", fields: STDOUT_FIELDS },
];

const OPERATOR_TYPES: &[ComponentTypeSpec] = &[
    ComponentTypeSpec { type_name: "regex", fields: REGEX_FIELDS },
    ComponentTypeSpec { type_name: "json", fields: JSON_FIELDS },
    ComponentTypeSpec { type_name: "kv", fields: KV_FIELDS },
    ComponentTypeSpec { type_name: "severity", fields: SEVERITY_FIELDS },
    ComponentTypeSpec { type_name: "timestamp", fields: TIMESTAMP_FIELDS },
    ComponentTypeSpec { type_name: "add", fields: ADD_FIELDS },
    ComponentTypeSpec { type_name: "remove", fields: REMOVE_FIELDS },
    ComponentTypeSpec { type_name: "copy", fields: FROM_TO_FIELDS },
    ComponentTypeSpec { type_name: "move", fields: FROM_TO_FIELDS },
    ComponentTypeSpec { type_name: "rename", fields: FROM_TO_FIELDS },
];

pub fn types_for(category: ComponentCategory) -> &'static [ComponentTypeSpec] {
    match category {
        ComponentCategory::Receiver => RECEIVER_TYPES,
        ComponentCategory::Exporter => EXPORTER_TYPES,
        ComponentCategory::Operator => OPERATOR_TYPES,
    }
}

/// Builds a minimal JSON value from a spec's required fields, using each
/// field's `default` where given and a harmless placeholder otherwise --
/// used both by "add component" (to seed a new component's initial
/// value) and by the drift test below.
pub fn minimal_value(spec: &ComponentTypeSpec) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert("type".to_string(), serde_json::json!(spec.type_name));
    for field in spec.fields {
        if !field.required {
            continue;
        }
        let value = match (field.kind, field.default) {
            (_, Some(default)) => serde_json::json!(default),
            (FieldKind::StringList, None) => serde_json::json!(["placeholder"]),
            (FieldKind::Int, None) => serde_json::json!(1),
            (FieldKind::Map, None) => serde_json::json!({}),
            (FieldKind::Enum(options), None) => serde_json::json!(options.first().copied().unwrap_or("")),
            (FieldKind::Str | FieldKind::Duration, None) => serde_json::json!("placeholder"),
        };
        map.insert(field.key.to_string(), value);
    }
    serde_json::Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catches registry/real-type drift: every registered type's
    /// minimal (required-fields-only) value must actually validate
    /// against the real `*Config::from_value` / operator builder.
    #[test]
    fn every_registered_type_round_trips_through_its_real_validator() {
        for spec in types_for(ComponentCategory::Receiver) {
            let value = minimal_value(spec);
            let id = format!("{}/test", spec.type_name);
            let result: Result<(), String> = match spec.type_name {
                "syslog" => sg_receiver_syslog::SyslogConfig::from_value(&id, &value).map(|_| ()),
                "filelog" => sg_receiver_file::FileLogConfig::from_value(&id, &value).map(|_| ()),
                "windows_eventlog" => {
                    sg_receiver_winevtlog::WinEventLogConfig::from_value(&id, &value).map(|_| ())
                }
                other => panic!("no drift test wired for receiver type '{other}'"),
            };
            assert!(result.is_ok(), "{}: {:?}", spec.type_name, result.err());
        }

        for spec in types_for(ComponentCategory::Exporter) {
            if spec.type_name == "stdout" {
                continue; // no config fields, nothing to validate
            }
            let value = minimal_value(spec);
            let id = format!("{}/test", spec.type_name);
            let result: Result<(), String> = match spec.type_name {
                "s1hec" => sg_exporter_s1_hec::S1HecConfig::from_value(&id, &value).map(|_| ()),
                "splunkhec" => sg_exporter_splunk_hec::SplunkHecConfig::from_value(&id, &value).map(|_| ()),
                other => panic!("no drift test wired for exporter type '{other}'"),
            };
            assert!(result.is_ok(), "{}: {:?}", spec.type_name, result.err());
        }

        for spec in types_for(ComponentCategory::Operator) {
            let mut value = minimal_value(spec);
            value["type"] = serde_json::json!(spec.type_name);
            let result = sg_operators::build_one(&format!("{}/test", spec.type_name), &value);
            assert!(result.is_ok(), "{}: {:?}", spec.type_name, result.err());
        }
    }
}
