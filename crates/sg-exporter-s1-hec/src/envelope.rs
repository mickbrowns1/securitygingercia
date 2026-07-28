use serde_json::{json, Map, Value};
use sg_core::Event;
use sg_exporter_core::{EnvelopeBuilder, EnvelopeError};
use std::collections::HashMap;

/// Builds the exact envelope shape SentinelOne DataPipeline expects, as
/// already validated on the wire by the sibling DPM-Syslog-NG project:
/// top-level keys are *only* `time`/`host`/`event`/`fields` -- anything
/// else at the top level gets prefixed with `splunk_` by DataPipeline,
/// which is why every other piece of metadata is nested under `fields`.
pub struct S1HecEnvelopeBuilder {
    /// Convention: `<product>_ts_parser` -- DataPipeline auto-selects a
    /// parser matching this name exactly.
    pub sourcetype: String,
    pub datasource: Option<String>,
    /// Dotted attribute path (e.g. "msgid" or "attributes.msgid") to pull
    /// the per-source routing key from, if the event has one.
    pub msgid_field: Option<String>,
    pub static_fields: HashMap<String, Value>,
}

fn resolve_attr<'a>(event: &'a Event, path: &str) -> Option<&'a Value> {
    let path = path.strip_prefix("attributes.").unwrap_or(path);
    event.attributes.get(path)
}

fn resolve_host(event: &Event) -> Value {
    event
        .resource
        .get("host")
        .or_else(|| event.attributes.get("hostname"))
        .cloned()
        .unwrap_or_else(|| json!("unknown"))
}

impl EnvelopeBuilder for S1HecEnvelopeBuilder {
    fn build(&self, event: &Event) -> Result<Value, EnvelopeError> {
        let mut fields = Map::new();
        fields.insert("sourcetype".to_string(), json!(self.sourcetype));
        if let Some(ds) = &self.datasource {
            fields.insert("datasource".to_string(), json!(ds));
        }
        if let Some(path) = &self.msgid_field {
            if let Some(v) = resolve_attr(event, path) {
                fields.insert("msgid".to_string(), v.clone());
            }
        }
        if let Some(sev) = &event.severity {
            fields.insert("syslog_severity".to_string(), json!(sev.number));
        }
        for (k, v) in &self.static_fields {
            fields.insert(k.clone(), v.clone());
        }

        Ok(json!({
            "time": event.timestamp.timestamp(),
            "host": resolve_host(event),
            "event": event.render_body(),
            "fields": fields,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sg_core::Severity;
    use std::collections::HashSet;

    fn builder() -> S1HecEnvelopeBuilder {
        S1HecEnvelopeBuilder {
            sourcetype: "cisco_asa_ts_parser".to_string(),
            datasource: Some("cisco_asa".to_string()),
            msgid_field: Some("attributes.msgid".to_string()),
            static_fields: HashMap::from([("tags".to_string(), json!("prod-datacenter-1"))]),
        }
    }

    #[test]
    fn top_level_keys_are_exactly_time_host_event_fields() {
        let b = builder();
        let mut event = Event::new(bytes::Bytes::from_static(b"hello"));
        event.attributes.insert("msgid".to_string(), json!("302013"));
        event.severity = Some(Severity {
            number: 3,
            text: "error".to_string(),
        });

        let envelope = b.build(&event).unwrap();
        let obj = envelope.as_object().unwrap();
        let keys: HashSet<&str> = obj.keys().map(|s| s.as_str()).collect();
        assert_eq!(keys, HashSet::from(["time", "host", "event", "fields"]));

        let fields = obj["fields"].as_object().unwrap();
        assert_eq!(fields["sourcetype"], "cisco_asa_ts_parser");
        assert_eq!(fields["datasource"], "cisco_asa");
        assert_eq!(fields["msgid"], "302013");
        assert_eq!(fields["syslog_severity"], 3);
        assert_eq!(fields["tags"], "prod-datacenter-1");
    }

    #[test]
    fn falls_back_to_hostname_attribute_when_no_resource_host() {
        let b = builder();
        let mut event = Event::new(bytes::Bytes::from_static(b"hello"));
        event
            .attributes
            .insert("hostname".to_string(), json!("mymachine"));
        let envelope = b.build(&event).unwrap();
        assert_eq!(envelope["host"], "mymachine");
    }
}
