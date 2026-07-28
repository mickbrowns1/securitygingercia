use serde::Deserialize;
use serde_json::Value;
use sg_config::parse_duration;
use sg_exporter_core::{BatchConfig, RetryPolicy};
use std::collections::HashMap;

fn default_max_events() -> usize {
    100
}
fn default_max_bytes() -> usize {
    1_000_000
}
fn default_flush_interval() -> String {
    "2s".to_string()
}
fn default_max_attempts() -> u32 {
    5
}
fn default_initial_backoff() -> String {
    "500ms".to_string()
}
fn default_max_backoff() -> String {
    "30s".to_string()
}

#[derive(Debug, Deserialize)]
struct BatchDef {
    #[serde(default = "default_max_events")]
    max_events: usize,
    #[serde(default = "default_max_bytes")]
    max_bytes: usize,
    #[serde(default = "default_flush_interval")]
    flush_interval: String,
}

impl Default for BatchDef {
    fn default() -> Self {
        Self {
            max_events: default_max_events(),
            max_bytes: default_max_bytes(),
            flush_interval: default_flush_interval(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct RetryDef {
    #[serde(default = "default_max_attempts")]
    max_attempts: u32,
    #[serde(default = "default_initial_backoff")]
    initial_backoff: String,
    #[serde(default = "default_max_backoff")]
    max_backoff: String,
}

impl Default for RetryDef {
    fn default() -> Self {
        Self {
            max_attempts: default_max_attempts(),
            initial_backoff: default_initial_backoff(),
            max_backoff: default_max_backoff(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct S1HecConfigDef {
    endpoint: String,
    token: String,
    sourcetype: String,
    #[serde(default)]
    datasource: Option<String>,
    #[serde(default)]
    msgid_field: Option<String>,
    #[serde(default)]
    static_fields: HashMap<String, Value>,
    #[serde(default)]
    batch: BatchDef,
    #[serde(default)]
    retry: RetryDef,
}

pub struct S1HecConfig {
    pub endpoint: reqwest::Url,
    pub token: String,
    pub sourcetype: String,
    pub datasource: Option<String>,
    pub msgid_field: Option<String>,
    pub static_fields: HashMap<String, Value>,
    pub batch: BatchConfig,
    pub retry: RetryPolicy,
}

impl S1HecConfig {
    pub fn from_value(id: &str, value: &Value) -> Result<Self, String> {
        let def: S1HecConfigDef =
            serde_json::from_value(value.clone()).map_err(|e| format!("{id}: {e}"))?;
        let endpoint = def
            .endpoint
            .parse()
            .map_err(|e| format!("{id}: invalid endpoint '{}': {e}", def.endpoint))?;
        let batch = BatchConfig {
            max_events: def.batch.max_events,
            max_bytes: def.batch.max_bytes,
            flush_interval: parse_duration(&def.batch.flush_interval)
                .map_err(|e| format!("{id}: invalid batch.flush_interval: {e}"))?,
        };
        let retry = RetryPolicy {
            max_attempts: def.retry.max_attempts,
            initial_backoff: parse_duration(&def.retry.initial_backoff)
                .map_err(|e| format!("{id}: invalid retry.initial_backoff: {e}"))?,
            max_backoff: parse_duration(&def.retry.max_backoff)
                .map_err(|e| format!("{id}: invalid retry.max_backoff: {e}"))?,
        };
        Ok(Self {
            endpoint,
            token: def.token,
            sourcetype: def.sourcetype,
            datasource: def.datasource,
            msgid_field: def.msgid_field,
            static_fields: def.static_fields,
            batch,
            retry,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn applies_defaults() {
        let cfg = S1HecConfig::from_value(
            "sentinelone_hec",
            &json!({
                "endpoint": "https://xdr.us1.sentinelone.net/services/collector/event",
                "token": "secret",
                "sourcetype": "cisco_asa_ts_parser",
            }),
        )
        .unwrap();
        assert_eq!(cfg.batch.max_events, 100);
        assert_eq!(cfg.retry.max_attempts, 5);
    }
}
