//! SentinelOne DataPipeline HEC exporter: envelope shape validated on the
//! wire by the sibling DPM-Syslog-NG project (top-level `time`/`host`/
//! `event`/`fields` only), batching/retry inherited from `sg-exporter-core`.

mod config;
mod envelope;

pub use config::S1HecConfig;
pub use envelope::S1HecEnvelopeBuilder;
use sg_core::ExporterMetrics;
use sg_exporter_core::HttpHecExporter;
use std::sync::Arc;

pub fn build(
    name: &str,
    value: &serde_json::Value,
    metrics: Arc<ExporterMetrics>,
) -> Result<HttpHecExporter<S1HecEnvelopeBuilder>, String> {
    let cfg = S1HecConfig::from_value(name, value)?;
    let builder = S1HecEnvelopeBuilder {
        sourcetype: cfg.sourcetype,
        datasource: cfg.datasource,
        msgid_field: cfg.msgid_field,
        static_fields: cfg.static_fields,
    };
    Ok(HttpHecExporter::new(
        name,
        cfg.endpoint,
        cfg.token,
        builder,
        cfg.batch,
        cfg.retry,
        metrics,
    ))
}
