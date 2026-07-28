use crate::error::ConfigError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level config shape, deliberately shallow: each component's own
/// concrete schema (syslog receiver options, S1 HEC exporter options, ...)
/// is resolved later by the crate that owns that component, keeping this
/// crate free of dependencies on any concrete receiver/operator/exporter
/// implementation.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RawConfig {
    #[serde(default)]
    pub receivers: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub operators: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub exporters: HashMap<String, serde_json::Value>,
    pub service: ServiceConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServiceConfig {
    pub pipelines: HashMap<String, PipelineConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PipelineConfig {
    #[serde(default)]
    pub receivers: Vec<String>,
    #[serde(default)]
    pub operators: Vec<String>,
    #[serde(default)]
    pub exporters: Vec<String>,
}

/// Component IDs follow the otel-contrib convention `type[/name]`
/// (e.g. `syslog/udp`, `filelog/app`) — the part before `/` selects
/// which concrete config schema to deserialize the value into.
pub fn component_type(id: &str) -> &str {
    id.split('/').next().unwrap_or(id)
}

impl RawConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        for (pipeline_name, pipeline) in &self.service.pipelines {
            if pipeline.receivers.is_empty() {
                return Err(ConfigError::EmptyPipelineReceivers(pipeline_name.clone()));
            }
            if pipeline.exporters.is_empty() {
                return Err(ConfigError::EmptyPipelineExporters(pipeline_name.clone()));
            }
            for name in &pipeline.receivers {
                if !self.receivers.contains_key(name) {
                    return Err(ConfigError::UnknownComponent {
                        pipeline: pipeline_name.clone(),
                        kind: "receiver",
                        name: name.clone(),
                    });
                }
            }
            for name in &pipeline.operators {
                if !self.operators.contains_key(name) {
                    return Err(ConfigError::UnknownComponent {
                        pipeline: pipeline_name.clone(),
                        kind: "operator",
                        name: name.clone(),
                    });
                }
            }
            for name in &pipeline.exporters {
                if !self.exporters.contains_key(name) {
                    return Err(ConfigError::UnknownComponent {
                        pipeline: pipeline_name.clone(),
                        kind: "exporter",
                        name: name.clone(),
                    });
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"
receivers:
  syslog/udp:
    protocol: udp
    listen_address: "0.0.0.0:514"
exporters:
  sentinelone_hec:
    type: s1hec
    endpoint: "https://example.invalid/services/collector/event"
    token: "test-token"
service:
  pipelines:
    logs/syslog:
      receivers: [syslog/udp]
      exporters: [sentinelone_hec]
"#;

    #[test]
    fn parses_and_validates_minimal_config() {
        let cfg: RawConfig = serde_yaml_ng::from_str(EXAMPLE).unwrap();
        cfg.validate().unwrap();
        assert_eq!(component_type("syslog/udp"), "syslog");
        assert_eq!(component_type("sentinelone_hec"), "sentinelone_hec");
    }

    #[test]
    fn rejects_unknown_receiver_reference() {
        let bad_cfg = r#"
receivers:
  syslog/udp:
    protocol: udp
exporters:
  sentinelone_hec:
    type: s1hec
service:
  pipelines:
    logs/syslog:
      receivers: [syslog/does_not_exist]
      exporters: [sentinelone_hec]
"#;
        let cfg: RawConfig = serde_yaml_ng::from_str(bad_cfg).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(err, ConfigError::UnknownComponent { .. }));
    }
}
