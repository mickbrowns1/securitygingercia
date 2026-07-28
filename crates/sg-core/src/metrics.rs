//! Shared metrics model for the status API + dashboard. Plain atomic
//! counters built once (one entry per receiver/pipeline/exporter name,
//! known from config before any task spawns) and handed out as `Arc`
//! clones -- no concurrent map needed, only the per-key atomics are ever
//! touched after construction.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
pub struct Counter(AtomicU64);

impl Counter {
    pub fn inc(&self) {
        self.add(1);
    }

    pub fn add(&self, n: u64) {
        self.0.fetch_add(n, Ordering::Relaxed);
    }

    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastError {
    pub message: String,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Default)]
pub struct ReceiverMetrics {
    pub events_in: Counter,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ReceiverSnapshot {
    pub events_in: u64,
}

impl ReceiverMetrics {
    pub fn snapshot(&self) -> ReceiverSnapshot {
        ReceiverSnapshot {
            events_in: self.events_in.get(),
        }
    }
}

#[derive(Debug, Default)]
pub struct PipelineMetrics {
    pub events_in: Counter,
    pub events_out: Counter,
    pub events_dropped: Counter,
    pub events_dead_lettered: Counter,
    pub parse_errors: Counter,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PipelineSnapshot {
    pub events_in: u64,
    pub events_out: u64,
    pub events_dropped: u64,
    pub events_dead_lettered: u64,
    pub parse_errors: u64,
}

impl PipelineMetrics {
    pub fn snapshot(&self) -> PipelineSnapshot {
        PipelineSnapshot {
            events_in: self.events_in.get(),
            events_out: self.events_out.get(),
            events_dropped: self.events_dropped.get(),
            events_dead_lettered: self.events_dead_lettered.get(),
            parse_errors: self.parse_errors.get(),
        }
    }
}

#[derive(Debug, Default)]
pub struct ExporterMetrics {
    pub events_in: Counter,
    pub batches_sent: Counter,
    pub batches_failed: Counter,
    pub retries: Counter,
    pub last_error: Mutex<Option<LastError>>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ExporterSnapshot {
    pub events_in: u64,
    pub batches_sent: u64,
    pub batches_failed: u64,
    pub retries: u64,
    pub last_error: Option<LastError>,
}

impl ExporterMetrics {
    pub fn record_retry(&self) {
        self.retries.inc();
    }

    pub fn record_success(&self, batch_len: u64) {
        self.events_in.add(batch_len);
        self.batches_sent.inc();
    }

    pub fn record_failure(&self, batch_len: u64, message: impl Into<String>) {
        self.events_in.add(batch_len);
        self.batches_failed.inc();
        *self.last_error.lock().unwrap() = Some(LastError {
            message: message.into(),
            at: Utc::now(),
        });
    }

    pub fn snapshot(&self) -> ExporterSnapshot {
        ExporterSnapshot {
            events_in: self.events_in.get(),
            batches_sent: self.batches_sent.get(),
            batches_failed: self.batches_failed.get(),
            retries: self.retries.get(),
            last_error: self.last_error.lock().unwrap().clone(),
        }
    }
}

/// Root handle: one entry per receiver/pipeline/exporter name, built once
/// from the resolved config before any task spawns, then `Arc`-cloned
/// into every task and into the status API handler.
pub struct Metrics {
    pub receivers: HashMap<String, Arc<ReceiverMetrics>>,
    pub pipelines: HashMap<String, Arc<PipelineMetrics>>,
    pub exporters: HashMap<String, Arc<ExporterMetrics>>,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MetricsSnapshot {
    pub started_at: DateTime<Utc>,
    pub uptime_seconds: i64,
    pub receivers: HashMap<String, ReceiverSnapshot>,
    pub pipelines: HashMap<String, PipelineSnapshot>,
    pub exporters: HashMap<String, ExporterSnapshot>,
}

impl Metrics {
    pub fn new(
        receiver_names: impl IntoIterator<Item = String>,
        pipeline_names: impl IntoIterator<Item = String>,
        exporter_names: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            receivers: receiver_names
                .into_iter()
                .map(|n| (n, Arc::new(ReceiverMetrics::default())))
                .collect(),
            pipelines: pipeline_names
                .into_iter()
                .map(|n| (n, Arc::new(PipelineMetrics::default())))
                .collect(),
            exporters: exporter_names
                .into_iter()
                .map(|n| (n, Arc::new(ExporterMetrics::default())))
                .collect(),
            started_at: Utc::now(),
        }
    }

    /// Falls back to a fresh, never-registered handle rather than
    /// panicking if asked for a name that wasn't in the config at
    /// construction time -- defensive, should not normally happen.
    pub fn receiver(&self, name: &str) -> Arc<ReceiverMetrics> {
        self.receivers.get(name).cloned().unwrap_or_default()
    }

    pub fn pipeline(&self, name: &str) -> Arc<PipelineMetrics> {
        self.pipelines.get(name).cloned().unwrap_or_default()
    }

    pub fn exporter(&self, name: &str) -> Arc<ExporterMetrics> {
        self.exporters.get(name).cloned().unwrap_or_default()
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        let now = Utc::now();
        MetricsSnapshot {
            started_at: self.started_at,
            uptime_seconds: (now - self.started_at).num_seconds(),
            receivers: self
                .receivers
                .iter()
                .map(|(k, v)| (k.clone(), v.snapshot()))
                .collect(),
            pipelines: self
                .pipelines
                .iter()
                .map(|(k, v)| (k.clone(), v.snapshot()))
                .collect(),
            exporters: self
                .exporters
                .iter()
                .map(|(k, v)| (k.clone(), v.snapshot()))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_add_and_get() {
        let c = Counter::default();
        c.inc();
        c.add(4);
        assert_eq!(c.get(), 5);
    }

    #[test]
    fn exporter_metrics_records_success_and_failure() {
        let m = ExporterMetrics::default();
        m.record_success(10);
        m.record_retry();
        m.record_failure(3, "boom");

        let snap = m.snapshot();
        assert_eq!(snap.events_in, 13);
        assert_eq!(snap.batches_sent, 1);
        assert_eq!(snap.batches_failed, 1);
        assert_eq!(snap.retries, 1);
        assert_eq!(snap.last_error.unwrap().message, "boom");
    }

    #[test]
    fn metrics_registry_builds_from_names_and_snapshots() {
        let metrics = Metrics::new(
            vec!["syslog/udp".to_string()],
            vec!["logs/syslog".to_string()],
            vec!["sentinelone_hec".to_string()],
        );

        metrics.receiver("syslog/udp").events_in.inc();
        metrics.pipeline("logs/syslog").events_out.add(2);
        metrics.exporter("sentinelone_hec").record_success(2);

        let snap = metrics.snapshot();
        assert_eq!(snap.receivers["syslog/udp"].events_in, 1);
        assert_eq!(snap.pipelines["logs/syslog"].events_out, 2);
        assert_eq!(snap.exporters["sentinelone_hec"].batches_sent, 1);
        assert!(snap.uptime_seconds >= 0);
    }

    #[test]
    fn unregistered_name_falls_back_to_fresh_handle_instead_of_panicking() {
        let metrics = Metrics::new(vec![], vec![], vec![]);
        let handle = metrics.receiver("not-registered");
        handle.events_in.inc();
        assert_eq!(handle.events_in.get(), 1);
    }
}
