//! Advisory field schema for the config editor's forms, transcribed from
//! the real field names documented by each OTel Collector component this
//! distribution ships (see `otelcol/builder-config.yaml`) -- `syslog`,
//! `file_log`, `windows_event_log` receivers, `splunk_hec`/`debug`
//! exporters, `file_storage`/`health_check`/`statuscfg` extensions, and
//! the `pkg/stanza` operator vocabulary used inline inside receivers.
//!
//! This registry only decides which widget a field gets and what help
//! text to show; it is never the source of truth for validity -- the
//! real `sgcia-otelcol validate` binary is run at save time regardless,
//! so registry/reality drift degrades to "worse form widget," never
//! "silently invalid config."

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentCategory {
    Receiver,
    Exporter,
    Extension,
}

#[derive(Debug, Clone, Copy)]
pub enum FieldKind {
    Str,
    Bool,
    Duration,
    Enum(&'static [&'static str]),
    StringList,
    /// The receiver's own inline `operators:` list -- edited through a
    /// dedicated sub-screen (add/edit/reorder/remove), never as text.
    OperatorList,
}

#[derive(Debug, Clone, Copy)]
pub struct FieldSpec {
    /// A dot-separated path into the component's value, e.g. `"endpoint"`
    /// or `"udp.listen_address"` / `"retry_on_failure.enabled"` -- lets
    /// one flat field list describe OTel's nested config blocks without
    /// the editor needing a sub-form per nesting level.
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

const OPERATORS: FieldSpec = f(
    "operators",
    FieldKind::OperatorList,
    false,
    None,
    "Inline parsing chain run on every event from this receiver, in order -- e.g. extract fields with a regex, then map severity, then add a sourcetype. Press Enter to manage the list.",
);

const STORAGE: FieldSpec = f(
    "storage",
    FieldKind::Str,
    false,
    None,
    "Optional: id of a file_storage extension to persist read position across restarts -- e.g. file_storage. Leave blank to track position in memory only (lost on restart).",
);

// --- Receivers ---

const SYSLOG_FIELDS: &[FieldSpec] = &[
    f(
        "protocol",
        FieldKind::Enum(&["rfc3164", "rfc5424", "none"]),
        true,
        Some("rfc3164"),
        "Which syslog message format to expect. \"none\" keeps the message body as-is (still decodes a leading <34>-style PRI header if present).",
    ),
    // Defaulted (rather than left required-but-blank) since at least one
    // of udp.listen_address/tcp.listen_address must actually be set --
    // the receiver refuses to start with neither -- and this is the one
    // that's pre-filled so a freshly-added syslog receiver validates
    // immediately instead of failing with "need tcp config or udp config".
    f(
        "udp.listen_address",
        FieldKind::Str,
        true,
        Some("0.0.0.0:514"),
        "Set this to listen over UDP, e.g. 0.0.0.0:514 -- clear it if using tcp.listen_address instead (at least one of the two is required). 0.0.0.0 means \"every network interface on this machine\".",
    ),
    f(
        "tcp.listen_address",
        FieldKind::Str,
        false,
        None,
        "Set this to listen over TCP, e.g. 0.0.0.0:601 -- leave blank if using udp.listen_address instead.",
    ),
    f(
        "enable_octet_counting",
        FieldKind::Bool,
        false,
        Some("false"),
        "TCP only: enable RFC 6587 octet-counting framing (each message prefixed with its byte length). Most TCP senders don't need this.",
    ),
    OPERATORS,
];

const FILE_LOG_FIELDS: &[FieldSpec] = &[
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
        "Where to start reading a file the first time it's seen: end (only new lines from now on) or beginning (read the whole existing file too).",
    ),
    f(
        "poll_interval",
        FieldKind::Duration,
        false,
        Some("200ms"),
        "How often to check the file(s) for new lines -- e.g. 200ms, 2s.",
    ),
    STORAGE,
    OPERATORS,
];

const WINDOWS_EVENT_LOG_FIELDS: &[FieldSpec] = &[
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
    STORAGE,
    OPERATORS,
];

// --- Exporters ---

const SPLUNK_HEC_FIELDS: &[FieldSpec] = &[
    // Defaulted since it must parse as a real URL even in the drift test.
    f(
        "endpoint",
        FieldKind::Str,
        true,
        Some("https://example.invalid:8088/services/collector/event"),
        "The full HEC ingest URL -- e.g. https://xdr.us1.sentinelone.net/services/collector/event.",
    ),
    f(
        "token",
        FieldKind::Str,
        true,
        None,
        "The HEC auth token for that endpoint. Use ${SOME_VAR} to read it from an environment variable instead of writing the real secret here.",
    ),
    f(
        "source",
        FieldKind::Str,
        false,
        None,
        "Optional Splunk \"source\" value. Leave blank unless otel_attrs_to_hec_metadata.source below is also blank -- see its help text.",
    ),
    f(
        "sourcetype",
        FieldKind::Str,
        false,
        None,
        "Optional Splunk \"sourcetype\" value. Leave blank unless otel_attrs_to_hec_metadata.sourcetype below is also blank -- see its help text.",
    ),
    f(
        "index",
        FieldKind::Str,
        false,
        None,
        "Optional: which Splunk/DataPipeline index to send events into.",
    ),
    f(
        "otel_attrs_to_hec_metadata.source",
        FieldKind::Str,
        false,
        Some(""),
        "Leave blank (the default here) to keep 'source' out of the top-level HEC envelope -- any attribute named 'source' set by this receiver's operators then falls into the event's nested fields{} object instead, which is what SentinelOne DataPipeline expects.",
    ),
    f(
        "otel_attrs_to_hec_metadata.sourcetype",
        FieldKind::Str,
        false,
        Some(""),
        "Leave blank (the default here) for the same reason as otel_attrs_to_hec_metadata.source -- keeps 'sourcetype' in fields{} rather than the top-level envelope.",
    ),
    f(
        "otel_attrs_to_hec_metadata.index",
        FieldKind::Str,
        false,
        Some(""),
        "Leave blank (the default here) for the same reason as otel_attrs_to_hec_metadata.source -- keeps 'index' in fields{} rather than the top-level envelope.",
    ),
    f(
        "sending_queue.enabled",
        FieldKind::Bool,
        false,
        Some("true"),
        "Buffer events in memory and retry sends in the background rather than blocking the pipeline on every request.",
    ),
    f(
        "retry_on_failure.enabled",
        FieldKind::Bool,
        false,
        Some("true"),
        "Retry with backoff when a send fails, rather than dropping the batch immediately.",
    ),
];

const DEBUG_FIELDS: &[FieldSpec] = &[f(
    "verbosity",
    FieldKind::Enum(&["basic", "normal", "detailed"]),
    false,
    Some("normal"),
    "How much detail to print per event to the terminal -- detailed dumps every field, useful for testing a pipeline before wiring up a real destination.",
)];

const DATASET_FIELDS: &[FieldSpec] = &[
    f(
        "dataset_url",
        FieldKind::Str,
        true,
        Some("https://app.scalyr.com"),
        "The SentinelOne Singularity Data Lake (formerly Scalyr/DataSet) ingest URL -- https://app.scalyr.com, or https://app.eu.scalyr.com for the EU instance.",
    ),
    f(
        "api_key",
        FieldKind::Str,
        true,
        None,
        "The DataSet/SDL write log token. Use ${SOME_VAR} to read it from an environment variable instead of writing the real secret here.",
    ),
    f(
        "server_host.use_hostname",
        FieldKind::Bool,
        false,
        Some("true"),
        "Fall back to this collector's own OS hostname for the event's serverHost if nothing more specific is set. Ignored if an event or its resource already carries a serverHost/host.name attribute -- see server_host.server_host below for the full lookup order.",
    ),
    f(
        "server_host.server_host",
        FieldKind::Str,
        false,
        None,
        "Fixed fallback serverHost value, used only if use_hostname is false and no event/resource attribute supplies one (required in that case -- the exporter fails to start otherwise). To set serverHost per-event instead, add a plain `serverHost` attribute via this receiver's own `add` operator -- it takes priority over everything here.",
    ),
];

const LOGBUFFER_FIELDS: &[FieldSpec] = &[f(
    "endpoint",
    FieldKind::Str,
    false,
    Some("http://127.0.0.1:7801"),
    "Loopback address of this collector's own statuscfg extension -- must match statuscfg's endpoint field. Feeds the web UI's log viewer; not a real destination, safe to add to every logs pipeline.",
)];

// --- Extensions ---

const FILE_STORAGE_FIELDS: &[FieldSpec] = &[
    f(
        "directory",
        FieldKind::Str,
        true,
        Some("/var/lib/sgcia/otelcol-storage"),
        "Directory where receivers using this extension (via their storage field) persist read position across restarts.",
    ),
    f(
        "create_directory",
        FieldKind::Bool,
        false,
        Some("true"),
        "Create the directory above automatically if it doesn't exist yet.",
    ),
];

const HEALTH_CHECK_FIELDS: &[FieldSpec] = &[f(
    "endpoint",
    FieldKind::Str,
    false,
    Some("127.0.0.1:13133"),
    "Address this extension's health-check HTTP endpoint listens on.",
)];

const STATUSCFG_FIELDS: &[FieldSpec] = &[
    f(
        "endpoint",
        FieldKind::Str,
        false,
        Some("127.0.0.1:7801"),
        "Address the dashboard/editor's /status + /config HTTP endpoint listens on -- 127.0.0.1:7801 matches sgcia dashboard's own default, so it works with no flags.",
    ),
    f(
        "config_path",
        FieldKind::Str,
        true,
        None,
        "The same file passed to this collector's own --config flag at startup -- this extension re-reads it to serve /config, since there's no API to read back the running config.",
    ),
    f(
        "metrics_url",
        FieldKind::Str,
        false,
        Some("http://localhost:8888/metrics"),
        "Where this collector's own internal Prometheus telemetry is exposed (service.telemetry.metrics in this same file) -- scraped on every /status request.",
    ),
];

// --- Operators (pkg/stanza vocabulary, nested inside a receiver) ---

const ON_ERROR: FieldSpec = f(
    "on_error",
    FieldKind::Enum(&["send", "send_quiet", "drop", "drop_quiet"]),
    false,
    Some("send"),
    "What to do if this operator fails on an event: send (forward the event unparsed, log the error) or drop (discard it). The _quiet variants only log at debug level.",
);

const REGEX_PARSER_FIELDS: &[FieldSpec] = &[
    // Defaulted to a trivial but *valid* pattern -- rather than a bare
    // placeholder string -- since the real component requires at least
    // one (?P<name>...) named capture group and rejects anything
    // without one at startup.
    f(
        "regex",
        FieldKind::Str,
        true,
        Some(r"(?P<message>.*)"),
        r#"Go regular expression, with (?P<name>...) capture groups for each field to extract -- e.g. ^%ASA-(?P<severity>\d)-(?P<msgid>\d+):\s*(?P<message>.*)$"#,
    ),
    f(
        "parse_from",
        FieldKind::Str,
        false,
        Some("body"),
        "Which field to read the text from.",
    ),
    f(
        "parse_to",
        FieldKind::Str,
        false,
        Some("attributes"),
        "Where to put the extracted fields.",
    ),
    ON_ERROR,
];

const JSON_PARSER_FIELDS: &[FieldSpec] = &[
    f(
        "parse_from",
        FieldKind::Str,
        false,
        Some("body"),
        "Which field holds the JSON text to parse.",
    ),
    f(
        "parse_to",
        FieldKind::Str,
        false,
        Some("attributes"),
        "Where to put the parsed result (merges keys in if attributes, replaces if body).",
    ),
    ON_ERROR,
];

const KEY_VALUE_PARSER_FIELDS: &[FieldSpec] = &[
    f(
        "parse_from",
        FieldKind::Str,
        false,
        Some("body"),
        "Which field holds the key=value text.",
    ),
    f(
        "parse_to",
        FieldKind::Str,
        false,
        Some("attributes"),
        "Where to put the extracted fields.",
    ),
    f(
        "delimiter",
        FieldKind::Str,
        false,
        Some("="),
        "Character that separates a key from its value.",
    ),
    f(
        "pair_delimiter",
        FieldKind::Str,
        false,
        None,
        "Character that separates one key=value pair from the next. Leave blank for whitespace.",
    ),
    ON_ERROR,
];

const SEVERITY_PARSER_FIELDS: &[FieldSpec] = &[
    f(
        "parse_from",
        FieldKind::Str,
        true,
        Some("attributes.severity"),
        "Which field holds the severity value -- e.g. attributes.severity.",
    ),
    f(
        "preset",
        FieldKind::Str,
        false,
        Some("default"),
        "Which numbering scheme to interpret the value with. \"default\" follows the OTel/syslog severity convention.",
    ),
    ON_ERROR,
];

const TIME_PARSER_FIELDS: &[FieldSpec] = &[
    f(
        "parse_from",
        FieldKind::Str,
        true,
        Some("attributes.ts"),
        "Which field holds the timestamp text -- e.g. attributes.ts.",
    ),
    f(
        "layout_type",
        FieldKind::Enum(&["strptime", "gotime", "epoch"]),
        false,
        Some("strptime"),
        "How to interpret the layout field below: strptime (%Y-%m-%d style), gotime (Go's reference-time style), or epoch (numeric seconds/ms since 1970).",
    ),
    f(
        "layout",
        FieldKind::Str,
        true,
        Some("%Y-%m-%dT%H:%M:%SZ"),
        "The timestamp format, in the style selected by layout_type -- e.g. %Y-%m-%dT%H:%M:%SZ for strptime.",
    ),
    ON_ERROR,
];

const ADD_FIELDS: &[FieldSpec] = &[
    f(
        "field",
        FieldKind::Str,
        true,
        Some("attributes.datasource"),
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
    Some("attributes.secret"),
    "Which field to delete -- e.g. attributes.secret.",
)];

const FROM_TO_FIELDS: &[FieldSpec] = &[
    f(
        "from",
        FieldKind::Str,
        true,
        Some("attributes.message"),
        "Field to read the value from -- e.g. attributes.message.",
    ),
    f(
        "to",
        FieldKind::Str,
        true,
        Some("body"),
        "Field to write the value to -- e.g. body.",
    ),
];

const RECEIVER_TYPES: &[ComponentTypeSpec] = &[
    ComponentTypeSpec {
        type_name: "syslog",
        description: "Listens for syslog messages over the network (UDP and/or TCP).",
        fields: SYSLOG_FIELDS,
    },
    ComponentTypeSpec {
        type_name: "file_log",
        description: "Tails one or more log files from disk, following new lines as they're written (like `tail -f`).",
        fields: FILE_LOG_FIELDS,
    },
    ComponentTypeSpec {
        type_name: "windows_event_log",
        description: "Reads events from a Windows Event Log channel (Windows only).",
        fields: WINDOWS_EVENT_LOG_FIELDS,
    },
];

const EXPORTER_TYPES: &[ComponentTypeSpec] = &[
    ComponentTypeSpec {
        type_name: "splunk_hec",
        description: "Sends collected logs to a Splunk-compatible HEC endpoint (including SentinelOne DataPipeline).",
        fields: SPLUNK_HEC_FIELDS,
    },
    ComponentTypeSpec {
        type_name: "debug",
        description: "Prints every event to the terminal instead of sending it anywhere -- useful for testing a pipeline before wiring up a real destination.",
        fields: DEBUG_FIELDS,
    },
    ComponentTypeSpec {
        type_name: "dataset",
        description: "Sends collected logs to SentinelOne Singularity Data Lake (formerly Scalyr/DataSet). Alpha stability upstream.",
        fields: DATASET_FIELDS,
    },
    ComponentTypeSpec {
        type_name: "logbuffer",
        description: "Feeds this collector's own web UI log viewer over loopback -- not a real destination, local visibility only.",
        fields: LOGBUFFER_FIELDS,
    },
];

const EXTENSION_TYPES: &[ComponentTypeSpec] = &[
    ComponentTypeSpec {
        type_name: "file_storage",
        description: "Persists receiver read-position/bookmarks to disk so a restart resumes correctly. Referenced by a receiver's storage field.",
        fields: FILE_STORAGE_FIELDS,
    },
    ComponentTypeSpec {
        type_name: "health_check",
        description: "Serves a simple HTTP health-check endpoint for this collector process.",
        fields: HEALTH_CHECK_FIELDS,
    },
    ComponentTypeSpec {
        type_name: "statuscfg",
        description: "Serves /status and /config for sgcia dashboard/edit to poll -- the local extension that replaces the old built-in status API.",
        fields: STATUSCFG_FIELDS,
    },
];

const OPERATOR_TYPES: &[ComponentTypeSpec] = &[
    ComponentTypeSpec {
        type_name: "regex_parser",
        description: "Extracts named fields out of the log text using a regular expression.",
        fields: REGEX_PARSER_FIELDS,
    },
    ComponentTypeSpec {
        type_name: "json_parser",
        description: "Parses a field's text as JSON and merges the result in.",
        fields: JSON_PARSER_FIELDS,
    },
    ComponentTypeSpec {
        type_name: "key_value_parser",
        description: "Parses key=value pairs out of a field's text -- e.g. user=neo action=login.",
        fields: KEY_VALUE_PARSER_FIELDS,
    },
    ComponentTypeSpec {
        type_name: "severity_parser",
        description: "Sets an event's severity by interpreting a value already in the event.",
        fields: SEVERITY_PARSER_FIELDS,
    },
    ComponentTypeSpec {
        type_name: "time_parser",
        description: "Sets an event's official timestamp by parsing a value already in the event.",
        fields: TIME_PARSER_FIELDS,
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
        description: "Moves (or renames) a field's value to a new location, removing it from the original.",
        fields: FROM_TO_FIELDS,
    },
];

pub fn types_for(category: ComponentCategory) -> &'static [ComponentTypeSpec] {
    match category {
        ComponentCategory::Receiver => RECEIVER_TYPES,
        ComponentCategory::Exporter => EXPORTER_TYPES,
        ComponentCategory::Extension => EXTENSION_TYPES,
    }
}

pub fn operator_types() -> &'static [ComponentTypeSpec] {
    OPERATOR_TYPES
}

pub fn operator_type(type_name: &str) -> Option<&'static ComponentTypeSpec> {
    OPERATOR_TYPES.iter().find(|s| s.type_name == type_name)
}

/// Reads a dot-separated path out of a JSON value, e.g. `"udp.listen_address"`.
pub fn get_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for part in path.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

/// Writes a dot-separated path into a JSON object, creating intermediate
/// objects as needed -- e.g. `set_path(map, "udp.listen_address", ...)`
/// creates `{"udp": {"listen_address": ...}}` if `udp` isn't there yet.
pub fn set_path(map: &mut serde_json::Map<String, serde_json::Value>, path: &str, value: serde_json::Value) {
    match path.split_once('.') {
        None => {
            map.insert(path.to_string(), value);
        }
        Some((first, rest)) => {
            let entry = map
                .entry(first.to_string())
                .or_insert_with(|| serde_json::json!({}));
            if !entry.is_object() {
                *entry = serde_json::json!({});
            }
            set_path(entry.as_object_mut().expect("just ensured object"), rest, value);
        }
    }
}

/// Builds a minimal JSON value from a spec's required fields, using each
/// field's `default` where given and a harmless placeholder otherwise --
/// used both by "add component" (to seed a new component's initial
/// value) and by the drift test below.
pub fn minimal_value(spec: &ComponentTypeSpec) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for field in spec.fields {
        if !field.required {
            continue;
        }
        let value = match (field.kind, field.default) {
            (_, Some(default)) => serde_json::json!(default),
            (FieldKind::StringList, None) => serde_json::json!(["placeholder"]),
            (FieldKind::Bool, None) => serde_json::json!(false),
            (FieldKind::OperatorList, None) => serde_json::json!([]),
            (FieldKind::Enum(options), None) => serde_json::json!(options.first().copied().unwrap_or("")),
            (FieldKind::Str | FieldKind::Duration, None) => serde_json::json!("placeholder"),
        };
        set_path(&mut map, field.key, value);
    }
    serde_json::Value::Object(map)
}

/// Component IDs follow the otel-contrib convention `type[/name]` (e.g.
/// `syslog/udp`, `file_log/app`) -- the part before `/` is always the
/// real component type, for every category (OTel infers type from the id
/// prefix alone, never from an internal `type:` key on receivers or
/// exporters -- unlike the retired Rust engine's exporters, which needed
/// one).
pub fn component_type(id: &str) -> &str {
    id.split('/').next().unwrap_or(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_field_and_type_has_help_text() {
        for category in [
            ComponentCategory::Receiver,
            ComponentCategory::Exporter,
            ComponentCategory::Extension,
        ] {
            for spec in types_for(category) {
                assert!(!spec.description.is_empty(), "{} has no description", spec.type_name);
                for field in spec.fields {
                    assert!(!field.help.is_empty(), "{}.{} has no help text", spec.type_name, field.key);
                }
            }
        }
        for spec in operator_types() {
            assert!(!spec.description.is_empty(), "{} has no description", spec.type_name);
            for field in spec.fields {
                assert!(!field.help.is_empty(), "{}.{} has no help text", spec.type_name, field.key);
            }
        }
    }

    #[test]
    fn set_path_and_get_path_round_trip_nested_keys() {
        let mut map = serde_json::Map::new();
        set_path(&mut map, "udp.listen_address", serde_json::json!("0.0.0.0:514"));
        set_path(&mut map, "tcp.listen_address", serde_json::json!("0.0.0.0:601"));
        let value = serde_json::Value::Object(map);
        assert_eq!(get_path(&value, "udp.listen_address").unwrap(), "0.0.0.0:514");
        assert_eq!(get_path(&value, "tcp.listen_address").unwrap(), "0.0.0.0:601");
        assert!(get_path(&value, "udp.missing").is_none());
    }

    #[test]
    fn component_type_takes_the_prefix_before_slash() {
        assert_eq!(component_type("syslog/udp"), "syslog");
        assert_eq!(component_type("file_log/app"), "file_log");
        assert_eq!(component_type("standalone"), "standalone");
    }

    #[test]
    fn minimal_value_nests_required_fields_by_path() {
        let spec = types_for(ComponentCategory::Extension)
            .iter()
            .find(|s| s.type_name == "file_storage")
            .unwrap();
        let value = minimal_value(spec);
        assert!(value.get("directory").is_some());
    }

    /// Every registered receiver/exporter/extension/operator type's
    /// `minimal_value()` output must actually validate against the real
    /// `sgcia-otelcol` binary once wrapped in a minimal full config --
    /// catches registry/reality drift (a field renamed upstream, a new
    /// required field, etc.) that nothing else here would catch. Skipped
    /// (not failed) if the binary hasn't been built yet, matching the
    /// pattern in `editor::model`'s own integration test.
    #[test]
    fn every_registered_type_round_trips_through_the_real_validator() {
        let bin = test_otelcol_binary_path();
        if !bin.exists() {
            eprintln!("skipping: {} not built (see otelcol/README)", bin.display());
            return;
        }

        let placeholder_receiver = || {
            let mut m = serde_json::Map::new();
            m.insert("include".to_string(), serde_json::json!(["/tmp/placeholder.log"]));
            serde_json::Value::Object(m)
        };

        for spec in types_for(ComponentCategory::Receiver) {
            if spec.type_name == "windows_event_log" {
                continue; // only starts a pipeline successfully on Windows
            }
            let id = format!("{}/test", spec.type_name);
            let config = wrap_in_full_config(
                [(id.clone(), seeded_value(spec, false))],
                [("debug".to_string(), serde_json::json!({}))],
                &[],
                [id],
                ["debug".to_string()],
            );
            assert_validates(&bin, &config, spec.type_name);
        }

        for spec in types_for(ComponentCategory::Exporter) {
            let id = format!("{}/test", spec.type_name);
            let config = wrap_in_full_config(
                [("file_log/test".to_string(), placeholder_receiver())],
                [(id.clone(), seeded_value(spec, false))],
                &[],
                ["file_log/test".to_string()],
                [id],
            );
            assert_validates(&bin, &config, spec.type_name);
        }

        for spec in types_for(ComponentCategory::Extension) {
            let config = wrap_in_full_config(
                [("file_log/test".to_string(), placeholder_receiver())],
                [("debug".to_string(), serde_json::json!({}))],
                &[(spec.type_name.to_string(), seeded_value(spec, false))],
                ["file_log/test".to_string()],
                ["debug".to_string()],
            );
            assert_validates(&bin, &config, spec.type_name);
        }

        for spec in operator_types() {
            let op_value = seeded_value(spec, true);
            let mut receiver = placeholder_receiver();
            receiver["operators"] = serde_json::json!([op_value]);
            let config = wrap_in_full_config(
                [("file_log/test".to_string(), receiver)],
                [("debug".to_string(), serde_json::json!({}))],
                &[],
                ["file_log/test".to_string()],
                ["debug".to_string()],
            );
            assert_validates(&bin, &config, spec.type_name);
        }
    }

    /// Runs a spec's `minimal_value()` seed through the exact same
    /// `FormState::new(...).to_value()` round trip the real editor uses
    /// when you add a new component and submit it untouched -- this
    /// picks up every field's *default*, not just the required ones
    /// `minimal_value()` alone seeds, matching what a real user actually
    /// produces (e.g. `file_storage`'s `create_directory: true` default,
    /// which isn't `required` but is always present in a real save).
    fn seeded_value(spec: &ComponentTypeSpec, write_type_key: bool) -> serde_json::Value {
        let seed = minimal_value(spec);
        crate::editor::app::FormState::new(Some(spec.type_name), write_type_key, spec.fields, &seed)
            .to_value()
    }

    /// Builds a minimal but complete otelcol config: the given receivers/
    /// exporters/extensions, wired into a single `logs/test` pipeline
    /// referencing the given receiver/exporter ids (extensions, if any,
    /// are always activated via `service.extensions`).
    fn wrap_in_full_config(
        receivers: impl IntoIterator<Item = (String, serde_json::Value)>,
        exporters: impl IntoIterator<Item = (String, serde_json::Value)>,
        extensions: &[(String, serde_json::Value)],
        pipeline_receivers: impl IntoIterator<Item = String>,
        pipeline_exporters: impl IntoIterator<Item = String>,
    ) -> serde_json::Value {
        let mut root = serde_json::Map::new();
        root.insert(
            "receivers".to_string(),
            serde_json::Value::Object(receivers.into_iter().collect()),
        );
        root.insert(
            "exporters".to_string(),
            serde_json::Value::Object(exporters.into_iter().collect()),
        );
        if !extensions.is_empty() {
            root.insert(
                "extensions".to_string(),
                serde_json::Value::Object(extensions.iter().cloned().collect()),
            );
        }
        let mut pipeline = serde_json::Map::new();
        pipeline.insert(
            "receivers".to_string(),
            serde_json::Value::Array(pipeline_receivers.into_iter().map(|s| serde_json::json!(s)).collect()),
        );
        pipeline.insert(
            "exporters".to_string(),
            serde_json::Value::Array(pipeline_exporters.into_iter().map(|s| serde_json::json!(s)).collect()),
        );
        let mut pipelines = serde_json::Map::new();
        pipelines.insert("logs/test".to_string(), serde_json::Value::Object(pipeline));
        let mut service = serde_json::Map::new();
        if !extensions.is_empty() {
            service.insert(
                "extensions".to_string(),
                serde_json::Value::Array(
                    extensions.iter().map(|(id, _)| serde_json::json!(id)).collect(),
                ),
            );
        }
        service.insert("pipelines".to_string(), serde_json::Value::Object(pipelines));
        root.insert("service".to_string(), serde_json::Value::Object(service));
        serde_json::Value::Object(root)
    }

    fn test_otelcol_binary_path() -> std::path::PathBuf {
        if let Ok(p) = std::env::var("SGCIA_OTELCOL_BIN") {
            return std::path::PathBuf::from(p);
        }
        // `cargo test` runs test binaries with cwd set to this crate's own
        // manifest directory (crates/collector), not the workspace root,
        // so a plain relative "otelcol/dist/..." never resolves here --
        // anchor to CARGO_MANIFEST_DIR (set at compile time) instead.
        let dev_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../otelcol/dist/sgcia-otelcol");
        if dev_path.exists() {
            return dev_path;
        }
        std::path::PathBuf::from("sgcia-otelcol")
    }

    fn assert_validates(bin: &std::path::Path, config: &serde_json::Value, type_name: &str) {
        use std::io::Write;
        let yaml_text = serde_yaml_ng::to_string(config).unwrap();
        let mut tmp = tempfile::Builder::new().suffix(".yaml").tempfile().unwrap();
        tmp.write_all(yaml_text.as_bytes()).unwrap();
        tmp.flush().unwrap();

        let output = std::process::Command::new(bin)
            .arg("validate")
            .arg("--config")
            .arg(format!("file:{}", tmp.path().display()))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{type_name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
