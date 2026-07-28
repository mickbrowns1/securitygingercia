use serde_json::{json, Map, Value};
use sg_core::Event;
use sg_exporter_core::{EnvelopeBuilder, EnvelopeError};

/// Standard Splunk HEC envelope: `event`/`source`/`sourcetype`/`index`/
/// `host`/`time` as sibling top-level keys -- unlike the S1 DataPipeline
/// exporter, there's no need to nest everything under `fields{}`.
pub struct SplunkHecEnvelopeBuilder {
    pub source: String,
    /// Dotted attribute path to read a per-event sourcetype from (e.g.
    /// "attributes.sourcetype"); falls back to `default_sourcetype` when
    /// absent.
    pub sourcetype_field: Option<String>,
    pub default_sourcetype: Option<String>,
    pub index: Option<String>,
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

impl EnvelopeBuilder for SplunkHecEnvelopeBuilder {
    fn build(&self, event: &Event) -> Result<Value, EnvelopeError> {
        let mut obj = Map::new();
        obj.insert("time".to_string(), json!(event.timestamp.timestamp()));
        obj.insert("host".to_string(), resolve_host(event));
        obj.insert("source".to_string(), json!(self.source));

        let sourcetype = self
            .sourcetype_field
            .as_deref()
            .and_then(|p| resolve_attr(event, p))
            .cloned()
            .or_else(|| self.default_sourcetype.as_ref().map(|s| json!(s)));
        if let Some(st) = sourcetype {
            obj.insert("sourcetype".to_string(), st);
        }
        if let Some(idx) = &self.index {
            obj.insert("index".to_string(), json!(idx));
        }
        obj.insert("event".to_string(), json!(event.render_body()));

        Ok(Value::Object(obj))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_sourcetype_field_when_present_on_event() {
        let b = SplunkHecEnvelopeBuilder {
            source: "sgcia".to_string(),
            sourcetype_field: Some("attributes.sourcetype".to_string()),
            default_sourcetype: Some("fallback".to_string()),
            index: Some("main".to_string()),
        };
        let mut event = Event::new(bytes::Bytes::from_static(b"hello"));
        event
            .attributes
            .insert("sourcetype".to_string(), json!("cisco_asa"));

        let envelope = b.build(&event).unwrap();
        assert_eq!(envelope["sourcetype"], "cisco_asa");
        assert_eq!(envelope["source"], "sgcia");
        assert_eq!(envelope["index"], "main");
        assert_eq!(envelope["event"], "hello");
    }

    #[test]
    fn falls_back_to_default_sourcetype_when_field_absent() {
        let b = SplunkHecEnvelopeBuilder {
            source: "sgcia".to_string(),
            sourcetype_field: Some("attributes.sourcetype".to_string()),
            default_sourcetype: Some("fallback".to_string()),
            index: None,
        };
        let event = Event::new(bytes::Bytes::from_static(b"hello"));
        let envelope = b.build(&event).unwrap();
        assert_eq!(envelope["sourcetype"], "fallback");
        assert!(envelope.as_object().unwrap().get("index").is_none());
    }
}
