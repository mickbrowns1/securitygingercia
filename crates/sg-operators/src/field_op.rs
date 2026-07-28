use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use sg_core::{Event, FieldRef, Operator, OperatorError};

enum FieldOpKind {
    Add { field: FieldRef, value: Value },
    Remove { field: FieldRef },
    Copy { from: FieldRef, to: FieldRef },
    /// Also used for `rename` -- both get the source value, clear the
    /// source, and set it at the destination.
    Move { from: FieldRef, to: FieldRef },
}

pub struct FieldOpOperator {
    id: String,
    kind: FieldOpKind,
}

#[derive(Deserialize)]
struct AddDef {
    field: String,
    value: Value,
}
#[derive(Deserialize)]
struct RemoveDef {
    field: String,
}
#[derive(Deserialize)]
struct FromToDef {
    from: String,
    to: String,
}

impl FieldOpOperator {
    pub fn from_value(id: &str, type_str: &str, value: &Value) -> Result<Self, String> {
        let kind = match type_str {
            "add" => {
                let d: AddDef =
                    serde_json::from_value(value.clone()).map_err(|e| format!("{id}: {e}"))?;
                FieldOpKind::Add {
                    field: FieldRef::parse(&d.field),
                    value: d.value,
                }
            }
            "remove" => {
                let d: RemoveDef =
                    serde_json::from_value(value.clone()).map_err(|e| format!("{id}: {e}"))?;
                FieldOpKind::Remove {
                    field: FieldRef::parse(&d.field),
                }
            }
            "copy" => {
                let d: FromToDef =
                    serde_json::from_value(value.clone()).map_err(|e| format!("{id}: {e}"))?;
                FieldOpKind::Copy {
                    from: FieldRef::parse(&d.from),
                    to: FieldRef::parse(&d.to),
                }
            }
            "move" | "rename" => {
                let d: FromToDef =
                    serde_json::from_value(value.clone()).map_err(|e| format!("{id}: {e}"))?;
                FieldOpKind::Move {
                    from: FieldRef::parse(&d.from),
                    to: FieldRef::parse(&d.to),
                }
            }
            other => return Err(format!("{id}: unknown field op type '{other}'")),
        };
        Ok(Self {
            id: id.to_string(),
            kind,
        })
    }
}

#[async_trait]
impl Operator for FieldOpOperator {
    fn id(&self) -> &str {
        &self.id
    }

    async fn process(&self, mut event: Event) -> Result<Event, (Event, OperatorError)> {
        match &self.kind {
            FieldOpKind::Add { field, value } => {
                field.set(&mut event, value.clone());
                Ok(event)
            }
            FieldOpKind::Remove { field } => {
                field.remove(&mut event);
                Ok(event)
            }
            FieldOpKind::Copy { from, to } => match from.get(&event).cloned() {
                Some(v) => {
                    to.set(&mut event, v);
                    Ok(event)
                }
                None => Err((
                    event,
                    OperatorError {
                        operator_id: self.id.clone(),
                        message: "copy source field not present".to_string(),
                    },
                )),
            },
            FieldOpKind::Move { from, to } => match from.get(&event).cloned() {
                Some(v) => {
                    from.remove(&mut event);
                    to.set(&mut event, v);
                    Ok(event)
                }
                None => Err((
                    event,
                    OperatorError {
                        operator_id: self.id.clone(),
                        message: "move source field not present".to_string(),
                    },
                )),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn blank_event() -> Event {
        Event::new(bytes::Bytes::from_static(b"x"))
    }

    #[tokio::test]
    async fn add_sets_a_static_field() {
        let cfg = json!({"field": "attributes.datasource", "value": "cisco_asa"});
        let op = FieldOpOperator::from_value("add_ds", "add", &cfg).unwrap();
        let event = op.process(blank_event()).await.unwrap();
        assert_eq!(event.attributes.get("datasource").unwrap(), "cisco_asa");
    }

    #[tokio::test]
    async fn remove_deletes_a_field() {
        let mut event = blank_event();
        event.attributes.insert("secret".to_string(), json!("x"));
        let cfg = json!({"field": "attributes.secret"});
        let op = FieldOpOperator::from_value("rm", "remove", &cfg).unwrap();
        let event = op.process(event).await.unwrap();
        assert!(!event.attributes.contains_key("secret"));
    }

    #[tokio::test]
    async fn copy_duplicates_without_removing_source() {
        let mut event = blank_event();
        event.attributes.insert("a".to_string(), json!("v"));
        let cfg = json!({"from": "attributes.a", "to": "attributes.b"});
        let op = FieldOpOperator::from_value("cp", "copy", &cfg).unwrap();
        let event = op.process(event).await.unwrap();
        assert_eq!(event.attributes.get("a").unwrap(), "v");
        assert_eq!(event.attributes.get("b").unwrap(), "v");
    }

    #[tokio::test]
    async fn move_relocates_and_removes_source() {
        let mut event = blank_event();
        event.attributes.insert("message".to_string(), json!("hi"));
        let cfg = json!({"from": "attributes.message", "to": "body"});
        let op = FieldOpOperator::from_value("mv", "move", &cfg).unwrap();
        let event = op.process(event).await.unwrap();
        assert!(!event.attributes.contains_key("message"));
        assert_eq!(event.body, json!("hi"));
    }

    #[tokio::test]
    async fn move_missing_source_is_an_error() {
        let cfg = json!({"from": "attributes.nope", "to": "body"});
        let op = FieldOpOperator::from_value("mv", "move", &cfg).unwrap();
        assert!(op.process(blank_event()).await.is_err());
    }
}
