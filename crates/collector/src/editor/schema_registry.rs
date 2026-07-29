//! Advisory field schema for the config editor's forms, mechanically
//! transcribed from the real `*ConfigDef` serde structs each component
//! crate already defines. This registry only decides which widget a
//! field gets (text box vs enum picker) and what help text to show; it
//! is never the source of truth for validity -- `*Config::from_value` is
//! called at save time regardless, so registry/type drift degrades to
//! "worse form widget," never "silently invalid config."

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
    /// One-line plain-English explanation shown while editing this
    /// field, ideally with a concrete example value.
    pub help: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct ComponentTypeSpec {
    pub type_name: &'static str,
    /// One-line plain-English description of what this type does,
    /// shown in the "add component" type picker and while editing.
    pub description: &'static str,
    pub fields: &'static [FieldSpec],
}

const fn f(
    key: &'static str,
    kind: FieldKind,
    required: bool,
    default: Option<&'static str>,
    help: &'static str,
) -> FieldSpec {
    FieldSpec { key, kind, required, default, help }
}

const ON_ERROR: FieldSpec = f(
    "on_error",
    FieldKind::Enum(&["drop", "pass", "dead_letter"]),
    false,
    Some("pass"),
    "What to do if this operator fails on an event: pass (keep the event, note the error and continue), drop (discard the event), or dead_letter (set it aside).",
);

// --- Receivers ---

const SYSLOG_FIELDS: &[FieldSpec] = &[
    f(
        "protocol",
        FieldKind::Enum(&["udp", "tcp"]),
        true,
        None,
        "Which network protocol to listen on.",
    ),
    // Defaulted (rather than a bare placeholder) since it must parse as a
    // real `SocketAddr` even in the drift test / a freshly-added component.
    f(
        "listen_address",
        FieldKind::Str,
        true,
        Some("0.0.0.0:514"),
        "IP and port to listen on, e.g. 0.0.0.0:514 -- 0.0.0.0 means \"listen on every network interface on this machine\".",
    ),
    f(
        "rfc",
        FieldKind::Enum(&["auto", "rfc3164", "rfc5424"]),
        false,
        Some("auto"),
        "Which syslog message format to expect. auto detects it automatically; most senders don't need this changed.",
    ),
    f(
        "framing",
        FieldKind::Enum(&["auto", "octet_counting", "non_transparent"]),
        false,
        Some("auto"),
        "TCP only: how messages are separated in the stream. auto detects it; most senders don't need this changed.",
    ),
    f(
        "max_message_size",
        FieldKind::Int,
        false,
        Some("65536"),
        "Largest single message sgcia will accept, in bytes (default 65536 = 64KB).",
    ),
];

const FILELOG_FIELDS: &[FieldSpec] = &[
    f(
        "include",
        FieldKind::StringList,
        true,
        None,
        "Which files to watch, as glob patterns -- e.g. /var/log/myapp/*.log. Separate multiple patterns with commas.",
    ),
    f(
        "exclude",
        FieldKind::StringList,
        false,
        None,
        "Optional: file patterns to skip even if they match include -- e.g. /var/log/myapp/*.gz.",
    ),
    f(
        "start_at",
        FieldKind::Enum(&["beginning", "end"]),
        false,
        Some("end"),
        "Where to start reading a file the first time sgcia sees it: end (only new lines from now on) or beginning (read the whole existing file too).",
    ),
    f(
        "poll_interval",
        FieldKind::Duration,
        false,
        Some("500ms"),
        "How often to check the file(s) for new lines -- e.g. 500ms, 2s.",
    ),
    f(
        "checkpoint_file",
        FieldKind::Str,
        true,
        None,
        "Where sgcia remembers how far it's read in each file, so a restart doesn't re-read or skip lines -- e.g. /var/lib/sgcia/myapp.checkpoint.json.",
    ),
];

const WINDOWS_EVENTLOG_FIELDS: &[FieldSpec] = &[
    f(
        "channel",
        FieldKind::Str,
        true,
        None,
        "Which Windows Event Log to read -- e.g. Security, System, Application.",
    ),
    f(
        "query",
        FieldKind::Str,
        false,
        Some("*"),
        "XPath filter for which events to include. * means all events on this channel.",
    ),
    f(
        "start_at",
        FieldKind::Enum(&["beginning", "end"]),
        false,
        Some("end"),
        "Where to start the first time: end (only new events from now on) or beginning (replay everything currently in the log).",
    ),
    f(
        "bookmark_file",
        FieldKind::Str,
        true,
        None,
        "Where sgcia remembers its place in the log so a restart resumes correctly -- e.g. C:\\ProgramData\\sgcia\\security.bookmark.xml.",
    ),
];

// --- Exporters ---

const S1HEC_FIELDS: &[FieldSpec] = &[
    // Defaulted since it must parse as a real URL even in the drift test.
    f(
        "endpoint",
        FieldKind::Str,
        true,
        Some("https://example.invalid/services/collector/event"),
        "The full HEC ingest URL your SentinelOne DataPipeline gave you -- e.g. https://xdr.us1.sentinelone.net/services/collector/event.",
    ),
    f(
        "token",
        FieldKind::Str,
        true,
        None,
        "The HEC auth token for that endpoint. Use ${SOME_VAR} to read it from an environment variable instead of writing the real secret here.",
    ),
    f(
        "sourcetype",
        FieldKind::Str,
        true,
        None,
        "Tells DataPipeline which parser to use for these logs. Convention: <product>_ts_parser -- e.g. cisco_asa_ts_parser.",
    ),
    f(
        "datasource",
        FieldKind::Str,
        false,
        None,
        "Optional label for where these logs came from -- e.g. cisco_asa. Becomes a searchable field.",
    ),
    f(
        "msgid_field",
        FieldKind::Str,
        false,
        None,
        "Optional: which extracted attribute to use as this event's routing/message-id key -- e.g. attributes.msgid.",
    ),
    f(
        "static_fields",
        FieldKind::Map,
        false,
        None,
        "Optional: extra fixed fields attached to every event, as JSON -- e.g. {\"tags\": \"prod-datacenter-1\"}.",
    ),
    f(
        "batch",
        FieldKind::Map,
        false,
        None,
        "Optional: how events are grouped before sending, as JSON -- e.g. {\"max_events\": 100, \"max_bytes\": 1048576, \"flush_interval\": \"2s\"}. Leave blank for sensible defaults.",
    ),
    f(
        "retry",
        FieldKind::Map,
        false,
        None,
        "Optional: what to do when a send fails, as JSON -- e.g. {\"max_attempts\": 5, \"initial_backoff\": \"500ms\", \"max_backoff\": \"30s\"}. Leave blank for sensible defaults.",
    ),
];

const SPLUNKHEC_FIELDS: &[FieldSpec] = &[
    // Defaulted since it must parse as a real URL even in the drift test.
    f(
        "endpoint",
        FieldKind::Str,
        true,
        Some("https://splunk.example.invalid:8088/services/collector/event"),
        "The full HEC ingest URL -- e.g. https://splunk.example.com:8088/services/collector/event.",
    ),
    f(
        "token",
        FieldKind::Str,
        true,
        None,
        "The HEC auth token. Use ${SOME_VAR} to read it from an environment variable instead of writing the real secret here.",
    ),
    f(
        "source",
        FieldKind::Str,
        false,
        Some("sgcia"),
        "Label shown in Splunk for where this data came from.",
    ),
    f(
        "sourcetype_field",
        FieldKind::Str,
        false,
        None,
        "Optional: which extracted attribute to use as the Splunk sourcetype -- e.g. attributes.sourcetype.",
    ),
    f(
        "sourcetype",
        FieldKind::Str,
        false,
        None,
        "Optional: a fixed sourcetype to use instead of (or as a fallback for) sourcetype_field.",
    ),
    f(
        "index",
        FieldKind::Str,
        false,
        None,
        "Optional: which Splunk index to send events into.",
    ),
    f(
        "batch",
        FieldKind::Map,
        false,
        None,
        "Optional: how events are grouped before sending, as JSON. Leave blank for sensible defaults.",
    ),
    f(
        "retry",
        FieldKind::Map,
        false,
        None,
        "Optional: retry behavior on send failure, as JSON. Leave blank for sensible defaults.",
    ),
];

const STDOUT_FIELDS: &[FieldSpec] = &[];

// --- Operators ---

const REGEX_FIELDS: &[FieldSpec] = &[
    f(
        "pattern",
        FieldKind::Str,
        true,
        None,
        r#"The regex pattern, with (?P<name>...) capture groups for each field to extract -- e.g. ^%ASA-(?P<severity>\d)-(?P<msgid>\d+): (?P<message>.*)$"#,
    ),
    f(
        "parse_from",
        FieldKind::Str,
        true,
        None,
        "Which field to read the text from -- usually body.",
    ),
    f(
        "parse_to",
        FieldKind::Str,
        true,
        None,
        "Where to put the extracted fields -- usually attributes.",
    ),
    ON_ERROR,
];

const JSON_FIELDS: &[FieldSpec] = &[
    f(
        "parse_from",
        FieldKind::Str,
        true,
        None,
        "Which field holds the JSON text to parse -- usually body.",
    ),
    f(
        "parse_to",
        FieldKind::Str,
        true,
        None,
        "Where to put the parsed result -- usually attributes (merges keys in) or body (replaces it).",
    ),
    ON_ERROR,
];

const KV_FIELDS: &[FieldSpec] = &[
    f(
        "parse_from",
        FieldKind::Str,
        true,
        None,
        "Which field holds the key=value text -- usually body.",
    ),
    f(
        "parse_to",
        FieldKind::Str,
        true,
        None,
        "Where to put the extracted fields -- usually attributes.",
    ),
    f(
        "pair_delimiter",
        FieldKind::Str,
        false,
        Some(" "),
        "Character that separates one key=value pair from the next (default: a single space).",
    ),
    f(
        "kv_delimiter",
        FieldKind::Str,
        false,
        Some("="),
        "Character that separates a key from its value (default: =).",
    ),
    ON_ERROR,
];

const SEVERITY_FIELDS: &[FieldSpec] = &[
    f(
        "parse_from",
        FieldKind::Str,
        true,
        None,
        "Which field holds the severity number -- e.g. attributes.severity.",
    ),
    f(
        "preset",
        FieldKind::Enum(&["syslog"]),
        true,
        Some("syslog"),
        "Which numbering scheme to use. Currently only \"syslog\" (0=emergency ... 7=debug) is supported.",
    ),
    ON_ERROR,
];

const TIMESTAMP_FIELDS: &[FieldSpec] = &[
    f(
        "parse_from",
        FieldKind::Str,
        true,
        None,
        "Which field holds the timestamp text -- e.g. attributes.ts.",
    ),
    f(
        "layout",
        FieldKind::Str,
        true,
        None,
        "The timestamp format: rfc3339, epoch (whole seconds), epoch_ms (milliseconds), or a custom strftime-style pattern.",
    ),
    ON_ERROR,
];

const ADD_FIELDS: &[FieldSpec] = &[
    f(
        "field",
        FieldKind::Str,
        true,
        None,
        "Which field to set -- e.g. attributes.datasource.",
    ),
    f(
        "value",
        FieldKind::Str,
        true,
        None,
        "The fixed value to set it to -- e.g. cisco_asa.",
    ),
];

const REMOVE_FIELDS: &[FieldSpec] = &[f(
    "field",
    FieldKind::Str,
    true,
    None,
    "Which field to delete -- e.g. attributes.secret.",
)];

const FROM_TO_FIELDS: &[FieldSpec] = &[
    f(
        "from",
        FieldKind::Str,
        true,
        None,
        "Field to read the value from -- e.g. attributes.message.",
    ),
    f(
        "to",
        FieldKind::Str,
        true,
        None,
        "Field to write the value to -- e.g. body.",
    ),
];

const RECEIVER_TYPES: &[ComponentTypeSpec] = &[
    ComponentTypeSpec {
        type_name: "syslog",
        description: "Listens for syslog messages over the network (UDP or TCP).",
        fields: SYSLOG_FIELDS,
    },
    ComponentTypeSpec {
        type_name: "filelog",
        description: "Tails one or more log files from disk, following new lines as they're written (like `tail -f`).",
        fields: FILELOG_FIELDS,
    },
    ComponentTypeSpec {
        type_name: "windows_eventlog",
        description: "Reads events from a Windows Event Log channel (Windows only).",
        fields: WINDOWS_EVENTLOG_FIELDS,
    },
];

const EXPORTER_TYPES: &[ComponentTypeSpec] = &[
    ComponentTypeSpec {
        type_name: "s1hec",
        description: "Sends collected logs to a SentinelOne DataPipeline HEC endpoint.",
        fields: S1HEC_FIELDS,
    },
    ComponentTypeSpec {
        type_name: "splunkhec",
        description: "Sends collected logs to a generic Splunk-compatible HEC endpoint.",
        fields: SPLUNKHEC_FIELDS,
    },
    ComponentTypeSpec {
        type_name: "stdout",
        description: "Debug exporter: prints every event as a line of JSON to the terminal. No fields to configure -- useful for testing a pipeline before wiring up a real destination.",
        fields: STDOUT_FIELDS,
    },
];

const OPERATOR_TYPES: &[ComponentTypeSpec] = &[
    ComponentTypeSpec {
        type_name: "regex",
        description: "Extracts named fields out of the log text using a regular expression.",
        fields: REGEX_FIELDS,
    },
    ComponentTypeSpec {
        type_name: "json",
        description: "Parses a field's text as JSON and merges the result in.",
        fields: JSON_FIELDS,
    },
    ComponentTypeSpec {
        type_name: "kv",
        description: "Parses key=value pairs out of a field's text -- e.g. user=neo action=login.",
        fields: KV_FIELDS,
    },
    ComponentTypeSpec {
        type_name: "severity",
        description: "Maps a numeric severity value onto a standard severity label.",
        fields: SEVERITY_FIELDS,
    },
    ComponentTypeSpec {
        type_name: "timestamp",
        description: "Parses a field's text as a timestamp and uses it as the event's official time.",
        fields: TIMESTAMP_FIELDS,
    },
    ComponentTypeSpec {
        type_name: "add",
        description: "Sets a field to a fixed value on every event.",
        fields: ADD_FIELDS,
    },
    ComponentTypeSpec {
        type_name: "remove",
        description: "Deletes a field from every event.",
        fields: REMOVE_FIELDS,
    },
    ComponentTypeSpec {
        type_name: "copy",
        description: "Copies a field's value to a second location, keeping the original too.",
        fields: FROM_TO_FIELDS,
    },
    ComponentTypeSpec {
        type_name: "move",
        description: "Moves a field's value to a new location, removing it from the original.",
        fields: FROM_TO_FIELDS,
    },
    ComponentTypeSpec {
        type_name: "rename",
        description: "Renames a field (same as move -- relocates the value and removes the old key).",
        fields: FROM_TO_FIELDS,
    },
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

    /// Every field and every type must have non-empty help text -- this
    /// is the whole point of the registry existing, so an empty string
    /// slipping in would silently defeat it.
    #[test]
    fn every_field_and_type_has_help_text() {
        for category in [
            ComponentCategory::Receiver,
            ComponentCategory::Exporter,
            ComponentCategory::Operator,
        ] {
            for spec in types_for(category) {
                assert!(
                    !spec.description.is_empty(),
                    "{} has no description",
                    spec.type_name
                );
                for field in spec.fields {
                    assert!(
                        !field.help.is_empty(),
                        "{}.{} has no help text",
                        spec.type_name,
                        field.key
                    );
                }
            }
        }
    }
}
