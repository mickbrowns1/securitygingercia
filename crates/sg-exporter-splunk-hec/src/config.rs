use serde::Deserialize;
use serde_json::Value;
use sg_config::parse_duration;
use sg_exporter_core::{BatchConfig, RetryPolicy};

fn default_max_events() -> usize {
    200
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
struct SplunkHecConfigDef {
    endpoint: String,
    token: String,
    #[serde(default = "default_source")]
    source: String,
    #[serde(default)]
    sourcetype_field: Option<String>,
    #[serde(default)]
    sourcetype: Option<String>,
    #[serde(default)]
    index: Option<String>,
    #[serde(default)]
    batch: BatchDef,
    #[serde(default)]
    retry: RetryDef,
}

fn default_source() -> String {
    "sgcia".to_string()
}

pub struct SplunkHecConfig {
    pub endpoint: reqwest::Url,
    pub token: String,
    pub source: String,
    pub sourcetype_field: Option<String>,
    pub default_sourcetype: Option<String>,
    pub index: Option<String>,
    pub batch: BatchConfig,
    pub retry: RetryPolicy,
}

impl SplunkHecConfig {
    pub fn from_value(id: &str, value: &Value) -> Result<Self, String> {
        let def: SplunkHecConfigDef =
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
            source: def.source,
            sourcetype_field: def.sourcetype_field,
            default_sourcetype: def.sourcetype,
            index: def.index,
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
        let cfg = SplunkHecConfig::from_value(
            "splunk_hec",
            &json!({
                "endpoint": "https://splunk.example.com:8088/services/collector/event",
                "token": "secret",
            }),
        )
        .unwrap();
        assert_eq!(cfg.source, "sgcia");
        assert_eq!(cfg.batch.max_events, 200);
    }
}
