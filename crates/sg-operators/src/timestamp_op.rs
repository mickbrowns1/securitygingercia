use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;
use serde_json::Value;
use sg_core::{Event, FieldRef, Operator, OperatorError};

#[derive(Debug, Deserialize)]
struct TimestampOpDef {
    parse_from: String,
    layout: String,
}

pub struct TimestampOperator {
    id: String,
    parse_from: FieldRef,
    layout: String,
}

impl TimestampOperator {
    pub fn from_value(id: &str, value: &Value) -> Result<Self, String> {
        let def: TimestampOpDef =
            serde_json::from_value(value.clone()).map_err(|e| format!("{id}: {e}"))?;
        Ok(Self {
            id: id.to_string(),
            parse_from: FieldRef::parse(&def.parse_from),
            layout: def.layout,
        })
    }

    fn parse(&self, value: &Value) -> Option<DateTime<Utc>> {
        match self.layout.as_str() {
            "rfc3339" => {
                let s = value.as_str()?;
                DateTime::parse_from_rfc3339(s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }
            "epoch" => {
                let secs = as_i64(value)?;
                Utc.timestamp_opt(secs, 0).single()
            }
            "epoch_ms" => {
                let millis = as_i64(value)?;
                Utc.timestamp_millis_opt(millis).single()
            }
            fmt => {
                let s = value.as_str()?;
                chrono::NaiveDateTime::parse_from_str(s, fmt)
                    .ok()
                    .map(|naive| Utc.from_utc_datetime(&naive))
            }
        }
    }
}

fn as_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse::<i64>().ok(),
        _ => None,
    }
}

#[async_trait]
impl Operator for TimestampOperator {
    fn id(&self) -> &str {
        &self.id
    }

    async fn process(&self, mut event: Event) -> Result<Event, (Event, OperatorError)> {
        let Some(raw) = self.parse_from.get(&event).cloned() else {
            return Err((
                event,
                OperatorError {
                    operator_id: self.id.clone(),
                    message: "parse_from field not present".to_string(),
                },
            ));
        };

        match self.parse(&raw) {
            Some(ts) => {
                event.timestamp = ts;
                Ok(event)
            }
            None => Err((
                event,
                OperatorError {
                    operator_id: self.id.clone(),
                    message: format!("could not parse timestamp with layout '{}'", self.layout),
                },
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn parses_rfc3339() {
        let cfg = json!({"parse_from": "attributes.ts", "layout": "rfc3339"});
        let op = TimestampOperator::from_value("ts", &cfg).unwrap();

        let mut event = Event::new(bytes::Bytes::from_static(b"x"));
        event
            .attributes
            .insert("ts".to_string(), json!("2026-07-28T12:00:00Z"));
        let event = op.process(event).await.unwrap();

        assert_eq!(event.timestamp.to_rfc3339(), "2026-07-28T12:00:00+00:00");
    }

    #[tokio::test]
    async fn parses_epoch_seconds() {
        let cfg = json!({"parse_from": "attributes.ts", "layout": "epoch"});
        let op = TimestampOperator::from_value("ts", &cfg).unwrap();

        let mut event = Event::new(bytes::Bytes::from_static(b"x"));
        event.attributes.insert("ts".to_string(), json!(1_800_000_000));
        let event = op.process(event).await.unwrap();

        assert_eq!(event.timestamp.timestamp(), 1_800_000_000);
    }

    #[tokio::test]
    async fn unparseable_value_is_an_error() {
        let cfg = json!({"parse_from": "attributes.ts", "layout": "rfc3339"});
        let op = TimestampOperator::from_value("ts", &cfg).unwrap();

        let mut event = Event::new(bytes::Bytes::from_static(b"x"));
        event.attributes.insert("ts".to_string(), json!("not a date"));
        assert!(op.process(event).await.is_err());
    }
}
