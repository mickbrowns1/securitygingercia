use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;
use serde_json::Value;
use sg_core::{Event, FieldRef, Operator, OperatorError};

#[derive(Debug, Deserialize)]
struct RegexOpDef {
    pattern: String,
    parse_from: String,
    parse_to: String,
}

pub struct RegexOperator {
    id: String,
    regex: Regex,
    parse_from: FieldRef,
    parse_to: FieldRef,
}

impl RegexOperator {
    pub fn from_value(id: &str, value: &Value) -> Result<Self, String> {
        let def: RegexOpDef =
            serde_json::from_value(value.clone()).map_err(|e| format!("{id}: {e}"))?;
        let regex =
            Regex::new(&def.pattern).map_err(|e| format!("{id}: invalid regex pattern: {e}"))?;
        Ok(Self {
            id: id.to_string(),
            regex,
            parse_from: FieldRef::parse(&def.parse_from),
            parse_to: FieldRef::parse(&def.parse_to),
        })
    }
}

#[async_trait]
impl Operator for RegexOperator {
    fn id(&self) -> &str {
        &self.id
    }

    async fn process(&self, mut event: Event) -> Result<Event, (Event, OperatorError)> {
        let input = match self.parse_from.get(&event) {
            Some(Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => {
                return Err((
                    event,
                    OperatorError {
                        operator_id: self.id.clone(),
                        message: "parse_from field not present".to_string(),
                    },
                ))
            }
        };

        let Some(caps) = self.regex.captures(&input) else {
            return Err((
                event,
                OperatorError {
                    operator_id: self.id.clone(),
                    message: "pattern did not match".to_string(),
                },
            ));
        };

        let mut extracted = serde_json::Map::new();
        for name in self.regex.capture_names().flatten() {
            if let Some(m) = caps.name(name) {
                extracted.insert(name.to_string(), Value::String(m.as_str().to_string()));
            }
        }

        self.parse_to.merge(&mut event, Value::Object(extracted));
        Ok(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn extracts_named_captures_into_attributes() {
        let cfg = json!({
            "pattern": r"^%ASA-(?P<severity>\d)-(?P<msgid>\d+):\s*(?P<message>.*)$",
            "parse_from": "body",
            "parse_to": "attributes",
        });
        let op = RegexOperator::from_value("extract_asa_fields", &cfg).unwrap();

        let event = Event::new(bytes::Bytes::from_static(
            b"%ASA-6-302013: Built inbound TCP connection",
        ));
        let event = op.process(event).await.unwrap();

        assert_eq!(event.attributes.get("severity").unwrap(), "6");
        assert_eq!(event.attributes.get("msgid").unwrap(), "302013");
        assert_eq!(
            event.attributes.get("message").unwrap(),
            "Built inbound TCP connection"
        );
    }

    #[tokio::test]
    async fn non_matching_input_is_an_error() {
        let cfg = json!({
            "pattern": r"^ONLY-THIS-MATCHES$",
            "parse_from": "body",
            "parse_to": "attributes",
        });
        let op = RegexOperator::from_value("r", &cfg).unwrap();
        let event = Event::new(bytes::Bytes::from_static(b"something else entirely"));
        assert!(op.process(event).await.is_err());
    }
}
