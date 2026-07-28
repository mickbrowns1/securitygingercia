use async_trait::async_trait;
use sg_core::{Event, Exporter, ExporterMetrics, SgError};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Debug exporter: prints each event's rendered body plus attributes as a
/// single JSON line to stdout. No batching/retry -- used to prove out
/// receivers and operator chains before the real HEC exporters exist.
pub struct StdoutExporter {
    name: String,
    metrics: Arc<ExporterMetrics>,
}

impl StdoutExporter {
    pub fn new(name: impl Into<String>, metrics: Arc<ExporterMetrics>) -> Self {
        Self {
            name: name.into(),
            metrics,
        }
    }
}

#[async_trait]
impl Exporter for StdoutExporter {
    fn name(&self) -> &str {
        &self.name
    }

    async fn run(
        self: Box<Self>,
        mut rx: mpsc::Receiver<Arc<Event>>,
        shutdown: CancellationToken,
    ) -> Result<(), SgError> {
        loop {
            // Biased: drain anything already buffered before honoring
            // shutdown (see the matching note in http.rs).
            tokio::select! {
                biased;

                event = rx.recv() => {
                    match event {
                        Some(event) => {
                            let line = serde_json::json!({
                                "time": event.timestamp.timestamp(),
                                "body": event.render_body(),
                                "attributes": event.attributes,
                                "severity": event.severity.as_ref().map(|s| &s.text),
                            });
                            println!("{line}");
                            self.metrics.record_success(1);
                        }
                        None => break,
                    }
                }
                _ = shutdown.cancelled() => break,
            }
        }
        Ok(())
    }
}
