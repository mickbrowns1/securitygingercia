use crate::field_op::FieldOpOperator;
use crate::json_op::JsonOperator;
use crate::kv_op::KvOperator;
use crate::regex_op::RegexOperator;
use crate::severity_op::SeverityOperator;
use crate::timestamp_op::TimestampOperator;
use serde_json::Value;
use sg_core::{OnError, Operator, OperatorChain};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("operator '{0}' has no 'type' field")]
    MissingType(String),
    #[error("operator '{0}' has unknown type '{1}'")]
    UnknownType(String, String),
    #[error("operator '{0}': {1}")]
    Invalid(String, String),
    #[error("pipeline references unknown operator '{0}'")]
    UnknownOperatorRef(String),
}

fn parse_on_error(value: &Value) -> OnError {
    match value.get("on_error").and_then(|v| v.as_str()) {
        Some("drop") => OnError::Drop,
        Some("dead_letter") => OnError::DeadLetter,
        // "pass" or unset -- pass is the safe default: never silently
        // lose an event just because one operator's parse failed.
        _ => OnError::Pass,
    }
}

/// Builds a single named operator from its raw config value.
pub fn build_one(name: &str, value: &Value) -> Result<(Arc<dyn Operator>, OnError), BuildError> {
    let type_str = value
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BuildError::MissingType(name.to_string()))?;
    let on_error = parse_on_error(value);

    let op: Arc<dyn Operator> = match type_str {
        "regex" => Arc::new(
            RegexOperator::from_value(name, value).map_err(|e| BuildError::Invalid(name.to_string(), e))?,
        ),
        "json" => Arc::new(
            JsonOperator::from_value(name, value).map_err(|e| BuildError::Invalid(name.to_string(), e))?,
        ),
        "kv" => Arc::new(
            KvOperator::from_value(name, value).map_err(|e| BuildError::Invalid(name.to_string(), e))?,
        ),
        "severity" => Arc::new(
            SeverityOperator::from_value(name, value)
                .map_err(|e| BuildError::Invalid(name.to_string(), e))?,
        ),
        "timestamp" => Arc::new(
            TimestampOperator::from_value(name, value)
                .map_err(|e| BuildError::Invalid(name.to_string(), e))?,
        ),
        "add" | "remove" | "copy" | "move" | "rename" => Arc::new(
            FieldOpOperator::from_value(name, type_str, value)
                .map_err(|e| BuildError::Invalid(name.to_string(), e))?,
        ),
        other => return Err(BuildError::UnknownType(name.to_string(), other.to_string())),
    };

    Ok((op, on_error))
}

/// Builds an `OperatorChain` from a pipeline's ordered list of operator
/// names, resolved against the config's `operators:` map.
pub fn build_chain(
    operator_defs: &HashMap<String, Value>,
    chain_names: &[String],
) -> Result<OperatorChain, BuildError> {
    let mut steps = Vec::with_capacity(chain_names.len());
    for name in chain_names {
        let def = operator_defs
            .get(name)
            .ok_or_else(|| BuildError::UnknownOperatorRef(name.clone()))?;
        steps.push(build_one(name, def)?);
    }
    Ok(OperatorChain::new(steps))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn builds_and_runs_a_multi_step_chain() {
        let mut defs = HashMap::new();
        defs.insert(
            "extract".to_string(),
            json!({
                "type": "regex",
                "pattern": r"^%ASA-(?P<severity>\d)-(?P<msgid>\d+):\s*(?P<message>.*)$",
                "parse_from": "body",
                "parse_to": "attributes",
                "on_error": "pass",
            }),
        );
        defs.insert(
            "add_ds".to_string(),
            json!({"type": "add", "field": "attributes.datasource", "value": "cisco_asa"}),
        );

        let chain =
            build_chain(&defs, &["extract".to_string(), "add_ds".to_string()]).unwrap();

        let event = sg_core::Event::new(bytes::Bytes::from_static(
            b"%ASA-6-302013: Built inbound TCP connection",
        ));
        let event = chain.run(event).await.unwrap();

        assert_eq!(event.attributes.get("msgid").unwrap(), "302013");
        assert_eq!(event.attributes.get("datasource").unwrap(), "cisco_asa");
        assert!(event.meta.errors.is_empty());
    }

    #[tokio::test]
    async fn pass_on_error_keeps_event_flowing_with_error_recorded() {
        let mut defs = HashMap::new();
        defs.insert(
            "extract".to_string(),
            json!({
                "type": "regex",
                "pattern": r"^NEVER-MATCHES$",
                "parse_from": "body",
                "parse_to": "attributes",
                "on_error": "pass",
            }),
        );
        let chain = build_chain(&defs, &["extract".to_string()]).unwrap();
        let event = sg_core::Event::new(bytes::Bytes::from_static(b"anything"));
        let event = chain.run(event).await.unwrap();
        assert_eq!(event.meta.errors.len(), 1);
    }

    #[tokio::test]
    async fn drop_on_error_discards_the_event() {
        let mut defs = HashMap::new();
        defs.insert(
            "extract".to_string(),
            json!({
                "type": "regex",
                "pattern": r"^NEVER-MATCHES$",
                "parse_from": "body",
                "parse_to": "attributes",
                "on_error": "drop",
            }),
        );
        let chain = build_chain(&defs, &["extract".to_string()]).unwrap();
        let event = sg_core::Event::new(bytes::Bytes::from_static(b"anything"));
        assert!(chain.run(event).await.is_none());
    }

    #[test]
    fn unknown_operator_reference_is_a_build_error() {
        let defs = HashMap::new();
        let err = build_chain(&defs, &["missing".to_string()]).unwrap_err();
        assert!(matches!(err, BuildError::UnknownOperatorRef(_)));
    }
}
