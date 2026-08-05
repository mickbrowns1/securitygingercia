//! Curated "source templates" for `sgcia edit`'s Receivers tab -- each one
//! produces a ready-to-use receiver (a listen protocol/address, or a file
//! glob, plus a parsing `operators:` chain already filled in) from a
//! couple of typed parameters, instead of making every user hand-write
//! their own `regex_parser` from scratch for common sources like Cisco
//! ASA or nginx access logs.
//!
//! Deliberately scoped to this distribution's three actual receiver
//! types (`syslog`, `file_log`, `windows_event_log`, see
//! `schema_registry`) -- no new OTel components. No templating engine
//! either: a template's `params` are just `FieldSpec`s, so `FormState`
//! (`app.rs`) already knows how to render/edit them, and its `build` fn
//! is a plain function from a params bag to the receiver's full `Value`
//! -- indistinguishable from a receiver built by hand once inserted into
//! `EditorDoc`.

use crate::editor::schema_registry::{self, FieldKind, FieldSpec};
use serde_json::{json, Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateCategory {
    NetworkSecurity,
    Generic,
    Windows,
    Database,
    Messaging,
}

impl TemplateCategory {
    pub fn label(self) -> &'static str {
        match self {
            TemplateCategory::NetworkSecurity => "Network & security devices",
            TemplateCategory::Database => "Databases",
            TemplateCategory::Messaging => "Messaging & big data",
            TemplateCategory::Generic => "Generic",
            TemplateCategory::Windows => "Windows / Active Directory",
        }
    }
}

pub struct SourceTemplate {
    /// Stable id, e.g. `"cisco_asa"` -- never shown to the user, only
    /// used to look a template back up by key.
    pub key: &'static str,
    pub title: &'static str,
    pub category: TemplateCategory,
    pub description: &'static str,
    /// Which registered receiver type (`schema_registry::types_for`)
    /// this template's `build` output matches the shape of --
    /// `"syslog"`, `"file_log"`, or `"windows_event_log"`.
    pub receiver_type: &'static str,
    /// Suggested id for the new receiver, e.g. `"syslog/cisco_asa"` --
    /// pre-fills the naming prompt; the user can still edit it.
    pub default_id: &'static str,
    /// Never `FieldKind::OperatorList` -- a template's own `build` fn
    /// always fills in `operators` itself.
    pub params: &'static [FieldSpec],
    pub build: fn(&Value) -> Value,
}

const fn param(
    key: &'static str,
    kind: FieldKind,
    required: bool,
    default: Option<&'static str>,
    help: &'static str,
) -> FieldSpec {
    FieldSpec { key, kind, required, default, help }
}

fn param_str<'a>(params: &'a Value, key: &str, fallback: &'a str) -> &'a str {
    schema_registry::get_path(params, key).and_then(Value::as_str).unwrap_or(fallback)
}

/// Shared by every syslog-based template: sets `udp.listen_address` or
/// `tcp.listen_address` (+ `enable_octet_counting` for tcp) from the
/// template's own `transport`/`listen_address` params.
fn set_transport(map: &mut Map<String, Value>, params: &Value, default_addr: &str) {
    let addr = param_str(params, "listen_address", default_addr);
    match param_str(params, "transport", "udp") {
        "tcp" => {
            schema_registry::set_path(map, "tcp.listen_address", json!(addr));
            schema_registry::set_path(map, "enable_octet_counting", json!(true));
        }
        _ => schema_registry::set_path(map, "udp.listen_address", json!(addr)),
    }
}

fn add_op(field: &str, value: &str) -> Value {
    json!({"type": "add", "field": field, "value": value})
}

fn move_op(from: &str, to: &str) -> Value {
    json!({"type": "move", "from": from, "to": to})
}

const TRANSPORT_ENUM: FieldKind = FieldKind::Enum(&["udp", "tcp"]);

// --- Network & security devices (all `syslog`) ---

const CISCO_ASA_PARAMS: &[FieldSpec] = &[
    param("transport", TRANSPORT_ENUM, true, Some("udp"), "udp or tcp, matching how the ASA is configured to send logs."),
    param(
        "listen_address",
        FieldKind::Str,
        true,
        Some("0.0.0.0:514"),
        "Address to listen on -- e.g. 0.0.0.0:514. Just a suggested starting port; adjust if this collector already listens on it elsewhere.",
    ),
];

fn build_cisco_asa(params: &Value) -> Value {
    let mut map = Map::new();
    schema_registry::set_path(&mut map, "protocol", json!("rfc3164"));
    set_transport(&mut map, params, "0.0.0.0:514");
    map.insert(
        "operators".to_string(),
        json!([
            {
                "type": "regex_parser",
                "parse_from": "body",
                "parse_to": "attributes",
                "regex": r"^%ASA-(?P<severity>\d)-(?P<msgid>\d+):\s*(?P<message>.*)$",
                "on_error": "send_quiet",
            },
            {"type": "severity_parser", "parse_from": "attributes.severity", "preset": "default", "on_error": "send_quiet"},
            add_op("attributes.datasource", "cisco_asa"),
            add_op("attributes.sourcetype", "cisco_asa_ts_parser"),
            move_op("attributes.message", "body"),
        ]),
    );
    Value::Object(map)
}

const CISCO_CATALYST_PARAMS: &[FieldSpec] = &[
    param("transport", TRANSPORT_ENUM, true, Some("udp"), "udp or tcp, matching how the switch is configured to send logs."),
    param("listen_address", FieldKind::Str, true, Some("0.0.0.0:515"), "Address to listen on -- e.g. 0.0.0.0:515."),
];

fn build_cisco_catalyst(params: &Value) -> Value {
    let mut map = Map::new();
    schema_registry::set_path(&mut map, "protocol", json!("rfc3164"));
    set_transport(&mut map, params, "0.0.0.0:515");
    map.insert(
        "operators".to_string(),
        json!([
            {
                // IOS's %FACILITY-SEVERITY-MNEMONIC: message grammar --
                // note the alphabetic mnemonic, unlike ASA's numeric msgid.
                "type": "regex_parser",
                "parse_from": "body",
                "parse_to": "attributes",
                "regex": r"^%(?P<facility>[A-Z0-9_]+)-(?P<severity>\d)-(?P<mnemonic>[A-Z0-9_]+):\s*(?P<message>.*)$",
                "on_error": "send_quiet",
            },
            {"type": "severity_parser", "parse_from": "attributes.severity", "preset": "default", "on_error": "send_quiet"},
            add_op("attributes.datasource", "cisco_ios"),
            add_op("attributes.sourcetype", "cisco_catalyst_ts_parser"),
            move_op("attributes.message", "body"),
        ]),
    );
    Value::Object(map)
}

const CISCO_MERAKI_PARAMS: &[FieldSpec] = &[
    param("transport", TRANSPORT_ENUM, true, Some("udp"), "udp or tcp, matching the syslog server target configured in the Meraki dashboard."),
    param("listen_address", FieldKind::Str, true, Some("0.0.0.0:516"), "Address to listen on -- e.g. 0.0.0.0:516."),
];

fn build_cisco_meraki(params: &Value) -> Value {
    let mut map = Map::new();
    // Meraki's own message body isn't a fixed syslog envelope (protocol
    // "none" skips rfc3164/5424 decoding, keeping just the PRI header
    // parsed off) -- its exact shape varies by device/log type
    // (flows/events/urls/...), so this covers the common
    // "<timestamp> <device> <log_type> key=value ..." export shape as a
    // starting point rather than an exact universal parser.
    schema_registry::set_path(&mut map, "protocol", json!("none"));
    set_transport(&mut map, params, "0.0.0.0:516");
    map.insert(
        "operators".to_string(),
        json!([
            {
                "type": "regex_parser",
                "parse_from": "body",
                "parse_to": "attributes",
                "regex": r"^(?P<timestamp>\S+\s+\S+)\s+(?P<device>\S+)\s+(?P<log_type>\S+)\s+(?P<message>.*)$",
                "on_error": "send_quiet",
            },
            {
                "type": "key_value_parser",
                "parse_from": "attributes.message",
                "parse_to": "attributes",
                "on_error": "send_quiet",
            },
            add_op("attributes.datasource", "cisco_meraki"),
            add_op("attributes.sourcetype", "cisco_meraki_ts_parser"),
            move_op("attributes.message", "body"),
        ]),
    );
    Value::Object(map)
}

const UBIQUITI_PARAMS: &[FieldSpec] = &[
    param("transport", TRANSPORT_ENUM, true, Some("udp"), "udp or tcp, matching the UniFi controller/device's remote syslog config."),
    param("listen_address", FieldKind::Str, true, Some("0.0.0.0:517"), "Address to listen on -- e.g. 0.0.0.0:517."),
];

fn build_ubiquiti(params: &Value) -> Value {
    let mut map = Map::new();
    schema_registry::set_path(&mut map, "protocol", json!("rfc3164"));
    set_transport(&mut map, params, "0.0.0.0:517");
    // UniFi/Ubiquiti devices (APs, switches, gateways) don't share one
    // fixed message grammar the way ASA/IOS do -- rfc3164 decoding
    // already leaves the plain message text in body on its own, so no
    // regex_parser (and no move, since there's no attributes.message to
    // move) is needed here, just tagging.
    map.insert("operators".to_string(), json!([add_op("attributes.datasource", "ubiquiti"), add_op("attributes.sourcetype", "ubiquiti_ts_parser")]));
    Value::Object(map)
}

const HAPROXY_PARAMS: &[FieldSpec] = &[
    param("transport", TRANSPORT_ENUM, true, Some("udp"), "udp or tcp, matching HAProxy's log target config."),
    param("listen_address", FieldKind::Str, true, Some("0.0.0.0:518"), "Address to listen on -- e.g. 0.0.0.0:518."),
];

fn build_haproxy(params: &Value) -> Value {
    let mut map = Map::new();
    schema_registry::set_path(&mut map, "protocol", json!("rfc3164"));
    set_transport(&mut map, params, "0.0.0.0:518");
    map.insert(
        "operators".to_string(),
        json!([
            {
                // HAProxy's default "httplog" format -- covers client
                // address, frontend/backend, status, bytes, and the raw
                // HTTP request line. HAProxy's format is configurable, so
                // this matches the common default rather than every case.
                "type": "regex_parser",
                "parse_from": "body",
                "parse_to": "attributes",
                "regex": r#"^(?P<client_ip>[\d.:a-fA-F]+):(?P<client_port>\d+) \[(?P<accept_date>[^\]]+)\] (?P<frontend>\S+) (?P<backend>\S+)/(?P<server>\S+) \S+ (?P<status_code>\d+) (?P<bytes_read>\d+) \S+ \S+ \S+ \S+ \S+ "(?P<http_request>[^"]*)"$"#,
                "on_error": "send_quiet",
            },
            add_op("attributes.datasource", "haproxy"),
            add_op("attributes.sourcetype", "haproxy_ts_parser"),
        ]),
    );
    Value::Object(map)
}

const CEF_PARAMS: &[FieldSpec] = &[
    param("transport", TRANSPORT_ENUM, true, Some("udp"), "udp or tcp, matching the CEF-emitting device's syslog config."),
    param("listen_address", FieldKind::Str, true, Some("0.0.0.0:519"), "Address to listen on -- e.g. 0.0.0.0:519."),
];

fn build_cef(params: &Value) -> Value {
    let mut map = Map::new();
    // CEF (Common Event Format) is a syslog *payload* format, not a
    // transport of its own -- protocol "none" just strips a leading PRI
    // header if present and leaves the CEF text itself in body.
    schema_registry::set_path(&mut map, "protocol", json!("none"));
    set_transport(&mut map, params, "0.0.0.0:519");
    map.insert(
        "operators".to_string(),
        json!([
            {
                "type": "regex_parser",
                "parse_from": "body",
                "parse_to": "attributes",
                "regex": r"^CEF:(?P<cef_version>\d)\|(?P<device_vendor>[^|]*)\|(?P<device_product>[^|]*)\|(?P<device_version>[^|]*)\|(?P<signature_id>[^|]*)\|(?P<name>[^|]*)\|(?P<cef_severity>[^|]*)\|(?P<extension>.*)$",
                "on_error": "send_quiet",
            },
            {
                // The CEF extension is its own key=value block, and CEF
                // severity (0-10) doesn't match the OTel/syslog severity
                // scale, so it's kept as a plain attribute rather than
                // run through severity_parser's "default" preset.
                "type": "key_value_parser",
                "parse_from": "attributes.extension",
                "parse_to": "attributes",
                "on_error": "send_quiet",
            },
            add_op("attributes.datasource", "cef"),
            add_op("attributes.sourcetype", "cef_ts_parser"),
        ]),
    );
    Value::Object(map)
}

// --- Generic (mostly `file_log`, plus one plain `syslog`) ---

const GENERIC_SYSLOG_PARAMS: &[FieldSpec] = &[
    param("transport", TRANSPORT_ENUM, true, Some("udp"), "udp or tcp, matching the sender's remote-syslog config."),
    param("listen_address", FieldKind::Str, true, Some("0.0.0.0:1514"), "Address to listen on -- e.g. 0.0.0.0:1514."),
];

fn build_generic_syslog(params: &Value) -> Value {
    let mut map = Map::new();
    // RFC 5424 (2009) is what most modern senders speak (current
    // util-linux logger, rsyslog's own default remote-forwarding
    // template) -- its envelope is already structured, so no
    // regex_parser is needed, just the move this receiver's RFC 5424
    // decoding always requires (see otelcol/config/example.yaml).
    schema_registry::set_path(&mut map, "protocol", json!("rfc5424"));
    set_transport(&mut map, params, "0.0.0.0:1514");
    map.insert(
        "operators".to_string(),
        json!([add_op("attributes.datasource", "generic_syslog"), add_op("attributes.sourcetype", "generic_rsyslog"), move_op("attributes.message", "body")]),
    );
    Value::Object(map)
}

const FILE_LOG_PARAMS: &[FieldSpec] = &[
    param("include", FieldKind::StringList, true, None, "File glob(s) to tail -- e.g. /var/log/myapp/*.log."),
    param("sourcetype", FieldKind::Str, true, None, "Sourcetype tag to add to every event from this file -- e.g. myapp."),
];

fn build_file_log(params: &Value) -> Value {
    let mut map = Map::new();
    let include = schema_registry::get_path(params, "include").cloned().unwrap_or_else(|| json!(["/var/log/*.log"]));
    map.insert("include".to_string(), include);
    schema_registry::set_path(&mut map, "start_at", json!("end"));
    let sourcetype = param_str(params, "sourcetype", "generic");
    map.insert("operators".to_string(), json!([add_op("attributes.sourcetype", sourcetype)]));
    Value::Object(map)
}

const JSON_LOGS_PARAMS: &[FieldSpec] = &[
    param("include", FieldKind::StringList, true, None, "File glob(s) to tail -- e.g. /var/log/myapp/*.jsonl."),
    param("sourcetype", FieldKind::Str, false, Some("json"), "Sourcetype tag to add to every event from this file."),
];

fn build_json_logs(params: &Value) -> Value {
    let mut map = Map::new();
    let include = schema_registry::get_path(params, "include").cloned().unwrap_or_else(|| json!(["/var/log/*.jsonl"]));
    map.insert("include".to_string(), include);
    schema_registry::set_path(&mut map, "start_at", json!("end"));
    let sourcetype = param_str(params, "sourcetype", "json");
    map.insert(
        "operators".to_string(),
        json!([
            {"type": "json_parser", "parse_from": "body", "parse_to": "attributes", "on_error": "send_quiet"},
            add_op("attributes.sourcetype", sourcetype),
        ]),
    );
    Value::Object(map)
}

const W3C_LOGS_PARAMS: &[FieldSpec] = &[param(
    "include",
    FieldKind::StringList,
    true,
    Some(r"C:\inetpub\logs\LogFiles\W3SVC1\*.log"),
    r"IIS log file glob(s) -- e.g. C:\inetpub\logs\LogFiles\W3SVC1\*.log.",
)];

fn build_w3c_logs(params: &Value) -> Value {
    let mut map = Map::new();
    let include = schema_registry::get_path(params, "include")
        .cloned()
        .unwrap_or_else(|| json!([r"C:\inetpub\logs\LogFiles\W3SVC1\*.log"]));
    map.insert("include".to_string(), include);
    schema_registry::set_path(&mut map, "start_at", json!("end"));
    map.insert(
        "operators".to_string(),
        json!([
            {
                // IIS's default W3C extended field selection, in its
                // default order. IIS log files also contain #Fields:/
                // #Software: comment lines -- on_error: send_quiet just
                // forwards those unparsed rather than failing loudly.
                "type": "regex_parser",
                "parse_from": "body",
                "parse_to": "attributes",
                "regex": r"^(?P<date>\S+) (?P<time>\S+) (?P<s_ip>\S+) (?P<method>\S+) (?P<uri_stem>\S+) (?P<uri_query>\S+) (?P<port>\S+) (?P<username>\S+) (?P<client_ip>\S+) (?P<user_agent>\S+) (?P<referer>\S+) (?P<status>\d+) (?P<substatus>\d+) (?P<win32_status>\d+) (?P<time_taken>\d+)$",
                "on_error": "send_quiet",
            },
            add_op("attributes.sourcetype", "iis_w3c"),
        ]),
    );
    Value::Object(map)
}

const NGINX_PARAMS: &[FieldSpec] =
    &[param("include", FieldKind::StringList, true, Some("/var/log/nginx/access.log"), "nginx access log file glob(s) -- e.g. /var/log/nginx/access.log.")];

fn build_nginx(params: &Value) -> Value {
    let mut map = Map::new();
    let include = schema_registry::get_path(params, "include").cloned().unwrap_or_else(|| json!(["/var/log/nginx/access.log"]));
    map.insert("include".to_string(), include);
    schema_registry::set_path(&mut map, "start_at", json!("end"));
    map.insert(
        "operators".to_string(),
        json!([
            {
                // nginx's default "combined" log_format.
                "type": "regex_parser",
                "parse_from": "body",
                "parse_to": "attributes",
                "regex": r#"^(?P<remote_addr>\S+) - (?P<remote_user>\S+) \[(?P<time_local>[^\]]+)\] "(?P<request>[^"]*)" (?P<status>\d+) (?P<bytes_sent>\d+) "(?P<referer>[^"]*)" "(?P<user_agent>[^"]*)"$"#,
                "on_error": "send_quiet",
            },
            add_op("attributes.sourcetype", "nginx_access"),
        ]),
    );
    Value::Object(map)
}

// --- Windows / Active Directory ---

const WINDOWS_DHCP_PARAMS: &[FieldSpec] = &[param(
    "include",
    FieldKind::StringList,
    true,
    Some(r"C:\Windows\System32\dhcp\DhcpSrvLog-*.log"),
    r"DHCP server audit log glob(s) -- e.g. C:\Windows\System32\dhcp\DhcpSrvLog-*.log.",
)];

fn build_windows_dhcp(params: &Value) -> Value {
    let mut map = Map::new();
    let include = schema_registry::get_path(params, "include")
        .cloned()
        .unwrap_or_else(|| json!([r"C:\Windows\System32\dhcp\DhcpSrvLog-*.log"]));
    map.insert("include".to_string(), include);
    schema_registry::set_path(&mut map, "start_at", json!("end"));
    map.insert(
        "operators".to_string(),
        json!([
            {
                // Microsoft's documented DHCP audit-log CSV schema --
                // this captures the first several (most security-relevant)
                // columns and keeps the rest as one trailing field, rather
                // than every column in the fixed 19-column schema.
                "type": "regex_parser",
                "parse_from": "body",
                "parse_to": "attributes",
                "regex": r"^(?P<event_id>\d+),(?P<date>[^,]*),(?P<time>[^,]*),(?P<description>[^,]*),(?P<ip_address>[^,]*),(?P<host_name>[^,]*),(?P<mac_address>[^,]*),(?P<rest>.*)$",
                "on_error": "send_quiet",
            },
            add_op("attributes.sourcetype", "windows_dhcp"),
        ]),
    );
    Value::Object(map)
}

const ACTIVE_DIRECTORY_PARAMS: &[FieldSpec] = &[param(
    "channel",
    FieldKind::Str,
    true,
    Some("Security"),
    "Windows Event Log channel to read -- Security covers logon/account/AD events; some environments also use Directory Service.",
)];

fn build_active_directory(params: &Value) -> Value {
    let mut map = Map::new();
    let channel = param_str(params, "channel", "Security");
    map.insert("channel".to_string(), json!(channel));
    map.insert("query".to_string(), json!("*"));
    map.insert("operators".to_string(), json!([add_op("attributes.sourcetype", "windows_security"), add_op("attributes.datasource", "active_directory")]));
    Value::Object(map)
}

// --- Databases (all `file_log`) ---

const MYSQL_PARAMS: &[FieldSpec] =
    &[param("include", FieldKind::StringList, true, Some("/var/log/mysql/error.log"), "MySQL error log glob(s) -- e.g. /var/log/mysql/error.log.")];

fn build_mysql(params: &Value) -> Value {
    let mut map = Map::new();
    let include = schema_registry::get_path(params, "include").cloned().unwrap_or_else(|| json!(["/var/log/mysql/error.log"]));
    map.insert("include".to_string(), include);
    schema_registry::set_path(&mut map, "start_at", json!("end"));
    map.insert(
        "operators".to_string(),
        json!([
            {
                // MySQL 8+'s default error log format:
                // <timestamp> <thread_id> [<level>] [<error_code>] [<subsystem>] message
                "type": "regex_parser",
                "parse_from": "body",
                "parse_to": "attributes",
                "regex": r"^(?P<timestamp>\S+)\s+(?P<thread_id>\d+)\s+\[(?P<level>\w+)\]\s+\[(?P<error_code>[\w-]+)\]\s+\[(?P<subsystem>[\w.]+)\]\s+(?P<message>.*)$",
                "on_error": "send_quiet",
            },
            add_op("attributes.sourcetype", "mysql_error"),
        ]),
    );
    Value::Object(map)
}

const POSTGRESQL_PARAMS: &[FieldSpec] = &[param(
    "include",
    FieldKind::StringList,
    true,
    Some("/var/log/postgresql/postgresql-*.log"),
    "PostgreSQL log file glob(s) -- e.g. /var/log/postgresql/postgresql-*.log.",
)];

fn build_postgresql(params: &Value) -> Value {
    let mut map = Map::new();
    let include = schema_registry::get_path(params, "include").cloned().unwrap_or_else(|| json!(["/var/log/postgresql/postgresql-*.log"]));
    map.insert("include".to_string(), include);
    schema_registry::set_path(&mut map, "start_at", json!("end"));
    map.insert(
        "operators".to_string(),
        json!([
            {
                // Matches Postgres's common log_line_prefix default of
                // "%m [%p] " (millisecond timestamp, then pid), followed
                // by the level. log_line_prefix is configurable, so this
                // covers the common default rather than every setup.
                "type": "regex_parser",
                "parse_from": "body",
                "parse_to": "attributes",
                "regex": r"^(?P<timestamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d+ \w+) \[(?P<pid>\d+)\] (?P<level>\w+):\s+(?P<message>.*)$",
                "on_error": "send_quiet",
            },
            add_op("attributes.sourcetype", "postgresql"),
        ]),
    );
    Value::Object(map)
}

const SQL_SERVER_PARAMS: &[FieldSpec] = &[param(
    "include",
    FieldKind::StringList,
    true,
    Some("/var/opt/mssql/log/errorlog"),
    "SQL Server error log glob(s) -- e.g. /var/opt/mssql/log/errorlog (Linux/container default) or the ERRORLOG file under Windows's MSSQL log directory.",
)];

fn build_sql_server(params: &Value) -> Value {
    let mut map = Map::new();
    let include = schema_registry::get_path(params, "include").cloned().unwrap_or_else(|| json!(["/var/opt/mssql/log/errorlog"]));
    map.insert("include".to_string(), include);
    schema_registry::set_path(&mut map, "start_at", json!("end"));
    map.insert(
        "operators".to_string(),
        json!([
            {
                // SQL Server's error log: "<date> <time>.<ms> <spid>  message"
                "type": "regex_parser",
                "parse_from": "body",
                "parse_to": "attributes",
                "regex": r"^(?P<date>\d{4}-\d{2}-\d{2}) (?P<time>\d{2}:\d{2}:\d{2}\.\d+) (?P<spid>\S+)\s+(?P<message>.*)$",
                "on_error": "send_quiet",
            },
            add_op("attributes.sourcetype", "sql_server"),
        ]),
    );
    Value::Object(map)
}

// --- Messaging & big data (all `file_log`) ---

const KAFKA_PARAMS: &[FieldSpec] =
    &[param("include", FieldKind::StringList, true, Some("/var/log/kafka/server.log"), "Kafka broker log glob(s) -- e.g. /var/log/kafka/server.log.")];

fn build_kafka(params: &Value) -> Value {
    let mut map = Map::new();
    let include = schema_registry::get_path(params, "include").cloned().unwrap_or_else(|| json!(["/var/log/kafka/server.log"]));
    map.insert("include".to_string(), include);
    schema_registry::set_path(&mut map, "start_at", json!("end"));
    map.insert(
        "operators".to_string(),
        json!([
            {
                // Kafka's default log4j pattern:
                // [<timestamp>] <level> message (<logger>)
                "type": "regex_parser",
                "parse_from": "body",
                "parse_to": "attributes",
                "regex": r"^\[(?P<timestamp>[\d-]+ [\d:,]+)\]\s+(?P<level>\w+)\s+(?P<message>.*?)\s+\((?P<logger>[\w.$]+)\)$",
                "on_error": "send_quiet",
            },
            add_op("attributes.sourcetype", "kafka"),
        ]),
    );
    Value::Object(map)
}

const HADOOP_PARAMS: &[FieldSpec] =
    &[param("include", FieldKind::StringList, true, Some("/var/log/hadoop/*.log"), "Hadoop daemon log glob(s) -- e.g. /var/log/hadoop/*.log.")];

fn build_hadoop(params: &Value) -> Value {
    let mut map = Map::new();
    let include = schema_registry::get_path(params, "include").cloned().unwrap_or_else(|| json!(["/var/log/hadoop/*.log"]));
    map.insert("include".to_string(), include);
    schema_registry::set_path(&mut map, "start_at", json!("end"));
    map.insert(
        "operators".to_string(),
        json!([
            {
                // Hadoop's default log4j pattern:
                // <timestamp> <level> [<thread>] <logger>: message
                "type": "regex_parser",
                "parse_from": "body",
                "parse_to": "attributes",
                "regex": r"^(?P<timestamp>[\d-]+ [\d:,]+)\s+(?P<level>\w+)\s+\[(?P<thread>[^\]]+)\]\s+(?P<logger>\S+):\s+(?P<message>.*)$",
                "on_error": "send_quiet",
            },
            add_op("attributes.sourcetype", "hadoop"),
        ]),
    );
    Value::Object(map)
}

pub const SOURCE_TEMPLATES: &[SourceTemplate] = &[
    SourceTemplate {
        key: "cisco_asa",
        title: "Cisco ASA (syslog)",
        category: TemplateCategory::NetworkSecurity,
        description: "Cisco ASA firewall, RFC 3164 syslog with the %ASA-severity-msgid: message grammar.",
        receiver_type: "syslog",
        default_id: "syslog/cisco_asa",
        params: CISCO_ASA_PARAMS,
        build: build_cisco_asa,
    },
    SourceTemplate {
        key: "cisco_catalyst",
        title: "Cisco Catalyst / IOS (syslog)",
        category: TemplateCategory::NetworkSecurity,
        description: "Cisco IOS switches/routers, RFC 3164 syslog with the %FACILITY-severity-MNEMONIC: message grammar.",
        receiver_type: "syslog",
        default_id: "syslog/cisco_catalyst",
        params: CISCO_CATALYST_PARAMS,
        build: build_cisco_catalyst,
    },
    SourceTemplate {
        key: "cisco_meraki",
        title: "Cisco Meraki (syslog)",
        category: TemplateCategory::NetworkSecurity,
        description: "Meraki dashboard syslog export (flows/events/urls). Best-effort parsing -- Meraki's exact message shape varies by log type.",
        receiver_type: "syslog",
        default_id: "syslog/cisco_meraki",
        params: CISCO_MERAKI_PARAMS,
        build: build_cisco_meraki,
    },
    SourceTemplate {
        key: "ubiquiti",
        title: "Ubiquiti / UniFi (syslog)",
        category: TemplateCategory::NetworkSecurity,
        description: "UniFi APs/switches/gateways, RFC 3164 syslog. No vendor-specific parsing -- message format varies by device type.",
        receiver_type: "syslog",
        default_id: "syslog/ubiquiti",
        params: UBIQUITI_PARAMS,
        build: build_ubiquiti,
    },
    SourceTemplate {
        key: "haproxy",
        title: "HAProxy (syslog)",
        category: TemplateCategory::NetworkSecurity,
        description: "HAProxy's default httplog format. HAProxy's log format is configurable, so this covers the common default.",
        receiver_type: "syslog",
        default_id: "syslog/haproxy",
        params: HAPROXY_PARAMS,
        build: build_haproxy,
    },
    SourceTemplate {
        key: "cef",
        title: "CEF -- Common Event Format (syslog)",
        category: TemplateCategory::NetworkSecurity,
        description: "Any device emitting CEF over syslog (ArcSight-style CEF:0|Vendor|Product|...|Extension).",
        receiver_type: "syslog",
        default_id: "syslog/cef",
        params: CEF_PARAMS,
        build: build_cef,
    },
    SourceTemplate {
        key: "generic_syslog",
        title: "Generic syslog (RFC 5424)",
        category: TemplateCategory::Generic,
        description: "Modern Linux hosts / rsyslog's default remote-forwarding template -- structured envelope, no vendor-specific regex needed.",
        receiver_type: "syslog",
        default_id: "syslog/generic",
        params: GENERIC_SYSLOG_PARAMS,
        build: build_generic_syslog,
    },
    SourceTemplate {
        key: "file_log",
        title: "Generic file tailing",
        category: TemplateCategory::Generic,
        description: "Tail any log file with no parsing beyond tagging a sourcetype -- a starting point for a format none of the other templates cover.",
        receiver_type: "file_log",
        default_id: "file_log/app",
        params: FILE_LOG_PARAMS,
        build: build_file_log,
    },
    SourceTemplate {
        key: "json_logs",
        title: "JSON logs",
        category: TemplateCategory::Generic,
        description: "One JSON object per line -- parsed and merged into attributes.",
        receiver_type: "file_log",
        default_id: "file_log/json",
        params: JSON_LOGS_PARAMS,
        build: build_json_logs,
    },
    SourceTemplate {
        key: "w3c_logs",
        title: "W3C / IIS extended log",
        category: TemplateCategory::Generic,
        description: "IIS's default W3C extended log field selection.",
        receiver_type: "file_log",
        default_id: "file_log/iis",
        params: W3C_LOGS_PARAMS,
        build: build_w3c_logs,
    },
    SourceTemplate {
        key: "nginx",
        title: "nginx access log",
        category: TemplateCategory::Generic,
        description: "nginx's default combined log_format.",
        receiver_type: "file_log",
        default_id: "file_log/nginx",
        params: NGINX_PARAMS,
        build: build_nginx,
    },
    SourceTemplate {
        key: "windows_dhcp",
        title: "Windows DHCP server log",
        category: TemplateCategory::Windows,
        description: "Microsoft DHCP server audit log (flat CSV file, not the Event Log) -- captures the security-relevant lease/host fields.",
        receiver_type: "file_log",
        default_id: "file_log/windows_dhcp",
        params: WINDOWS_DHCP_PARAMS,
        build: build_windows_dhcp,
    },
    SourceTemplate {
        key: "active_directory",
        title: "Active Directory (Windows Event Log)",
        category: TemplateCategory::Windows,
        description: "Domain controller Security channel events (logon/account/AD activity). Windows-only at runtime, like any windows_event_log receiver.",
        receiver_type: "windows_event_log",
        default_id: "windows_event_log/active_directory",
        params: ACTIVE_DIRECTORY_PARAMS,
        build: build_active_directory,
    },
    SourceTemplate {
        key: "mysql",
        title: "MySQL error log",
        category: TemplateCategory::Database,
        description: "MySQL 8+'s default error log format ([level] [error_code] [subsystem] message).",
        receiver_type: "file_log",
        default_id: "file_log/mysql",
        params: MYSQL_PARAMS,
        build: build_mysql,
    },
    SourceTemplate {
        key: "postgresql",
        title: "PostgreSQL log",
        category: TemplateCategory::Database,
        description: "Postgres's common log_line_prefix default (%m [%p] ). log_line_prefix is configurable, so this covers the common default.",
        receiver_type: "file_log",
        default_id: "file_log/postgresql",
        params: POSTGRESQL_PARAMS,
        build: build_postgresql,
    },
    SourceTemplate {
        key: "sql_server",
        title: "SQL Server error log",
        category: TemplateCategory::Database,
        description: "SQL Server's error log format (date time.ms spid message) -- works for both Linux/container and Windows installs.",
        receiver_type: "file_log",
        default_id: "file_log/sql_server",
        params: SQL_SERVER_PARAMS,
        build: build_sql_server,
    },
    SourceTemplate {
        key: "kafka",
        title: "Kafka broker log",
        category: TemplateCategory::Messaging,
        description: "Kafka's default log4j pattern ([timestamp] level message (logger)).",
        receiver_type: "file_log",
        default_id: "file_log/kafka",
        params: KAFKA_PARAMS,
        build: build_kafka,
    },
    SourceTemplate {
        key: "hadoop",
        title: "Hadoop daemon log",
        category: TemplateCategory::Messaging,
        description: "Hadoop's default log4j pattern (timestamp level [thread] logger: message).",
        receiver_type: "file_log",
        default_id: "file_log/hadoop",
        params: HADOOP_PARAMS,
        build: build_hadoop,
    },
];

pub fn find(key: &str) -> Option<&'static SourceTemplate> {
    SOURCE_TEMPLATES.iter().find(|t| t.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::app::FormState;
    use crate::editor::schema_registry::test_support::{assert_validates, test_otelcol_binary_path, wrap_in_full_config};

    #[test]
    fn every_template_has_a_description_and_matches_a_registered_receiver_type() {
        for template in SOURCE_TEMPLATES {
            assert!(!template.description.is_empty(), "{} has no description", template.key);
            assert!(
                ["syslog", "file_log", "windows_event_log"].contains(&template.receiver_type),
                "{} has an unrecognized receiver_type {}",
                template.key,
                template.receiver_type
            );
            for p in template.params {
                assert!(!matches!(p.kind, FieldKind::OperatorList), "{}'s param {} must not be an OperatorList", template.key, p.key);
                assert!(!p.help.is_empty(), "{}.{} has no help text", template.key, p.key);
            }
        }
    }

    #[test]
    fn keys_and_default_ids_are_unique() {
        let mut keys: Vec<&str> = SOURCE_TEMPLATES.iter().map(|t| t.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), SOURCE_TEMPLATES.len(), "duplicate template key");

        let mut ids: Vec<&str> = SOURCE_TEMPLATES.iter().map(|t| t.default_id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), SOURCE_TEMPLATES.len(), "duplicate default_id");
    }

    #[test]
    fn find_looks_up_by_key() {
        assert!(find("cisco_asa").is_some());
        assert!(find("not-a-real-template").is_none());
    }

    /// Every template's `build` output, seeded the same way the real
    /// editor seeds a fresh params form (defaults only, nothing typed),
    /// must validate against the real `sgcia-otelcol` binary once wrapped
    /// in a minimal full config -- catches a bad regex or any drift up
    /// front. Skipped (not failed) if the binary hasn't been built yet,
    /// matching schema_registry's own drift test.
    #[test]
    fn every_template_builds_a_config_that_round_trips_through_the_real_validator() {
        let bin = test_otelcol_binary_path();
        if !bin.exists() {
            eprintln!("skipping: {} not built (see otelcol/README)", bin.display());
            return;
        }

        for template in SOURCE_TEMPLATES {
            if template.receiver_type == "windows_event_log" {
                continue; // only starts a pipeline successfully on Windows
            }
            let params = FormState::new(None, false, template.params, &json!({})).to_value();
            let receiver = (template.build)(&params);
            let id = format!("{}/template-test", template.receiver_type);
            let config = wrap_in_full_config(
                [(id.clone(), receiver)],
                [("debug".to_string(), json!({}))],
                &[],
                [id],
                ["debug".to_string()],
            );
            assert_validates(&bin, &config, template.key);
        }
    }
}
