//! Generic Splunk-HEC-compatible exporter: standard sibling-key envelope
//! (event/source/sourcetype/index/host/time), batching/retry inherited
//! from `sg-exporter-core`.

mod config;
mod envelope;

pub use config::SplunkHecConfig;
pub use envelope::SplunkHecEnvelopeBuilder;
use sg_core::ExporterMetrics;
use sg_exporter_core::HttpHecExporter;
use std::sync::Arc;

pub fn build(
    name: &str,
    value: &serde_json::Value,
    metrics: Arc<ExporterMetrics>,
) -> Result<HttpHecExporter<SplunkHecEnvelopeBuilder>, String> {
    let cfg = SplunkHecConfig::from_value(name, value)?;
    let builder = SplunkHecEnvelopeBuilder {
        source: cfg.source,
        sourcetype_field: cfg.sourcetype_field,
        default_sourcetype: cfg.default_sourcetype,
        index: cfg.index,
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
