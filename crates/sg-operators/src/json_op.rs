use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use sg_core::{Event, FieldRef, Operator, OperatorError};

#[derive(Debug, Deserialize)]
struct JsonOpDef {
    parse_from: String,
    parse_to: String,
}

pub struct JsonOperator {
    id: String,
    parse_from: FieldRef,
    parse_to: FieldRef,
}

impl JsonOperator {
    pub fn from_value(id: &str, value: &Value) -> Result<Self, String> {
        let def: JsonOpDef =
            serde_json::from_value(value.clone()).map_err(|e| format!("{id}: {e}"))?;
        Ok(Self {
            id: id.to_string(),
            parse_from: FieldRef::parse(&def.parse_from),
            parse_to: FieldRef::parse(&def.parse_to),
        })
    }
}

#[async_trait]
impl Operator for JsonOperator {
    fn id(&self) -> &str {
        &self.id
    }

    async fn process(&self, mut event: Event) -> Result<Event, (Event, OperatorError)> {
        let input = match self.parse_from.get(&event) {
            Some(Value::String(s)) => s.clone(),
            _ => {
                return Err((
                    event,
                    OperatorError {
                        operator_id: self.id.clone(),
                        message: "parse_from field is not a string".to_string(),
                    },
                ))
            }
        };

        let parsed: Value = match serde_json::from_str(&input) {
            Ok(v) => v,
            Err(e) => {
                return Err((
                    event,
                    OperatorError {
                        operator_id: self.id.clone(),
                        message: format!("invalid JSON: {e}"),
                    },
                ))
            }
        };

        self.parse_to.merge(&mut event, parsed);
        Ok(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn merges_parsed_object_into_attributes() {
        let cfg = json!({"parse_from": "body", "parse_to": "attributes"});
        let op = JsonOperator::from_value("j", &cfg).unwrap();

        let event = Event::new(bytes::Bytes::from_static(
            br#"{"user":"neo","result":"success"}"#,
        ));
        let event = op.process(event).await.unwrap();

        assert_eq!(event.attributes.get("user").unwrap(), "neo");
        assert_eq!(event.attributes.get("result").unwrap(), "success");
    }

    #[tokio::test]
    async fn replaces_body_when_parse_to_is_body() {
        let cfg = json!({"parse_from": "body", "parse_to": "body"});
        let op = JsonOperator::from_value("j", &cfg).unwrap();

        let event = Event::new(bytes::Bytes::from_static(br#"{"a":1}"#));
        let event = op.process(event).await.unwrap();

        assert_eq!(event.body, json!({"a": 1}));
    }

    #[tokio::test]
    async fn invalid_json_is_an_error() {
        let cfg = json!({"parse_from": "body", "parse_to": "attributes"});
        let op = JsonOperator::from_value("j", &cfg).unwrap();
        let event = Event::new(bytes::Bytes::from_static(b"not json at all"));
        assert!(op.process(event).await.is_err());
    }
}
