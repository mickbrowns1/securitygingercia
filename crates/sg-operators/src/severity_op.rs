use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use sg_core::{Event, FieldRef, Operator, OperatorError, Severity};

const SYSLOG_SEVERITY_TEXT: [&str; 8] = [
    "emergency", "alert", "critical", "error", "warning", "notice", "informational", "debug",
];

#[derive(Debug, Deserialize)]
struct SeverityOpDef {
    parse_from: String,
    preset: String,
}

pub struct SeverityOperator {
    id: String,
    parse_from: FieldRef,
    preset: String,
}

impl SeverityOperator {
    pub fn from_value(id: &str, value: &Value) -> Result<Self, String> {
        let def: SeverityOpDef =
            serde_json::from_value(value.clone()).map_err(|e| format!("{id}: {e}"))?;
        if def.preset != "syslog" {
            return Err(format!(
                "{id}: unknown severity preset '{}' (only 'syslog' is supported)",
                def.preset
            ));
        }
        Ok(Self {
            id: id.to_string(),
            parse_from: FieldRef::parse(&def.parse_from),
            preset: def.preset,
        })
    }
}

#[async_trait]
impl Operator for SeverityOperator {
    fn id(&self) -> &str {
        &self.id
    }

    async fn process(&self, mut event: Event) -> Result<Event, (Event, OperatorError)> {
        let number = match self.parse_from.get(&event) {
            Some(Value::String(s)) => s.parse::<i64>().ok(),
            Some(Value::Number(n)) => n.as_i64(),
            _ => None,
        };

        let number = match number {
            Some(n) if self.preset == "syslog" && (0..=7).contains(&n) => n,
            _ => {
                return Err((
                    event,
                    OperatorError {
                        operator_id: self.id.clone(),
                        message: "parse_from did not contain a valid syslog severity (0-7)"
                            .to_string(),
                    },
                ))
            }
        };

        event.severity = Some(Severity {
            number: number as i32,
            text: SYSLOG_SEVERITY_TEXT[number as usize].to_string(),
        });
        Ok(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn maps_syslog_severity_number_to_text() {
        let cfg = json!({"parse_from": "attributes.severity", "preset": "syslog"});
        let op = SeverityOperator::from_value("sev", &cfg).unwrap();

        let mut event = Event::new(bytes::Bytes::from_static(b"x"));
        event
            .attributes
            .insert("severity".to_string(), json!("3"));
        let event = op.process(event).await.unwrap();

        let sev = event.severity.unwrap();
        assert_eq!(sev.number, 3);
        assert_eq!(sev.text, "error");
    }

    #[tokio::test]
    async fn out_of_range_is_an_error() {
        let cfg = json!({"parse_from": "attributes.severity", "preset": "syslog"});
        let op = SeverityOperator::from_value("sev", &cfg).unwrap();

        let mut event = Event::new(bytes::Bytes::from_static(b"x"));
        event
            .attributes
            .insert("severity".to_string(), json!("99"));
        assert!(op.process(event).await.is_err());
    }
}
