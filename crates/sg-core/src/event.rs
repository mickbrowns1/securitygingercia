use chrono::{DateTime, Utc};

/// Structured metadata attached to an event: extracted/derived fields
/// (maps onto SentinelOne DataPipeline's `fields{}`) or static per-source
/// info (host, file path, channel).
pub type Attributes = serde_json::Map<String, serde_json::Value>;

#[derive(Debug, Clone)]
pub struct Severity {
    pub number: i32,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct EventMeta {
    /// Non-fatal operator failures accumulated as the event moves through
    /// an OperatorChain under `OnError::Pass`. Never causes data loss on
    /// its own.
    pub errors: Vec<String>,
    pub dead_letter: bool,
}

/// A single log record as it flows receiver -> operator chain -> exporter.
#[derive(Debug, Clone)]
pub struct Event {
    /// Immutable original bytes as received. Never mutated by operators;
    /// kept around for dead-lettering and debugging.
    pub raw: bytes::Bytes,

    /// The "current best" representation of the payload. Starts as the
    /// lossy-UTF8 string of `raw`; operators may replace it with a
    /// structured `Value::Object` (e.g. the JSON operator with
    /// `parse_to: body`). Maps directly onto the HEC `event` field.
    pub body: serde_json::Value,

    /// Event timestamp, defaulting to `observed_timestamp` until a
    /// timestamp operator overwrites it from parsed content.
    pub timestamp: DateTime<Utc>,

    /// Time the receiver ingested this record. Never overwritten.
    pub observed_timestamp: DateTime<Utc>,

    /// Extracted/derived fields. Receivers seed initial values (host,
    /// source ip, syslog facility/severity); operators add/remove/rename.
    pub attributes: Attributes,

    /// Fairly static per-source info: host, receiver name, file path,
    /// Windows Event Log channel, etc.
    pub resource: Attributes,

    pub severity: Option<Severity>,
    pub meta: EventMeta,
}

impl Event {
    pub fn new(raw: bytes::Bytes) -> Self {
        let now = Utc::now();
        let body = serde_json::Value::String(String::from_utf8_lossy(&raw).into_owned());
        Self {
            raw,
            body,
            timestamp: now,
            observed_timestamp: now,
            attributes: Attributes::new(),
            resource: Attributes::new(),
            severity: None,
            meta: EventMeta::default(),
        }
    }

    /// Render `body` the way it should appear in an HEC `event` field:
    /// a plain string as-is, or a compact JSON string if it has been
    /// replaced with a structured value by an operator.
    pub fn render_body(&self) -> String {
        match &self.body {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        }
    }
}
