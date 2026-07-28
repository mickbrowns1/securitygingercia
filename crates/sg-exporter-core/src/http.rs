use async_trait::async_trait;
use serde_json::Value;
use sg_core::{Event, Exporter, ExporterMetrics, SgError};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, thiserror::Error)]
pub enum EnvelopeError {
    #[error("{0}")]
    Message(String),
}

/// Everything that differs between HEC-flavored exporters (SentinelOne
/// DataPipeline vs generic Splunk HEC): how one `Event` becomes one wire
/// envelope. Batching, retry/backoff, and the newline-delimited framing
/// that DataPipeline requires all live in `HttpHecExporter` and are
/// shared by every implementation.
pub trait EnvelopeBuilder: Send + Sync {
    fn build(&self, event: &Event) -> Result<Value, EnvelopeError>;
}

#[derive(Debug, Clone)]
pub struct BatchConfig {
    pub max_events: usize,
    pub max_bytes: usize,
    pub flush_interval: Duration,
}

impl Default for BatchConfig {
    fn default() -> Self {
        Self {
            max_events: 100,
            max_bytes: 1_000_000,
            flush_interval: Duration::from_secs(2),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            initial_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(30),
        }
    }
}

pub struct HttpHecExporter<B> {
    name: String,
    client: reqwest::Client,
    endpoint: reqwest::Url,
    auth_header: String,
    builder: B,
    batch: BatchConfig,
    retry: RetryPolicy,
    metrics: Arc<ExporterMetrics>,
}

impl<B: EnvelopeBuilder> HttpHecExporter<B> {
    pub fn new(
        name: impl Into<String>,
        endpoint: reqwest::Url,
        token: impl Into<String>,
        builder: B,
        batch: BatchConfig,
        retry: RetryPolicy,
        metrics: Arc<ExporterMetrics>,
    ) -> Self {
        Self {
            name: name.into(),
            client: reqwest::Client::new(),
            endpoint,
            auth_header: format!("Splunk {}", token.into()),
            builder,
            batch,
            retry,
            metrics,
        }
    }

    fn encode_batch(&self, batch: &[Arc<Event>]) -> Result<Vec<u8>, EnvelopeError> {
        let mut body = Vec::new();
        for event in batch {
            let value = self.builder.build(event)?;
            serde_json::to_writer(&mut body, &value)
                .map_err(|e| EnvelopeError::Message(e.to_string()))?;
            // REQUIRED: DataPipeline concatenates/merges adjacent JSON
            // objects in one body without this delimiter.
            body.push(b'\n');
        }
        Ok(body)
    }

    async fn send_with_retry(&self, body: Vec<u8>, batch_len: u64) {
        let mut attempt = 0u32;
        let mut backoff = self.retry.initial_backoff;
        loop {
            attempt += 1;
            let result = self
                .client
                .post(self.endpoint.clone())
                .header("Authorization", &self.auth_header)
                .header("Content-Type", "application/json")
                .body(body.clone())
                .send()
                .await;

            let failure_message = match result {
                Ok(resp) if resp.status().is_success() => {
                    self.metrics.record_success(batch_len);
                    return;
                }
                Ok(resp) => {
                    let status = resp.status();
                    tracing::warn!(exporter = %self.name, %status, attempt, "HEC export rejected");
                    format!("HEC export rejected: {status}")
                }
                Err(e) => {
                    tracing::warn!(exporter = %self.name, error = %e, attempt, "HEC export request error");
                    format!("HEC export request error: {e}")
                }
            };

            if attempt >= self.retry.max_attempts {
                tracing::error!(exporter = %self.name, attempts = attempt, "giving up on batch after max retry attempts");
                self.metrics.record_failure(batch_len, failure_message);
                return;
            }
            self.metrics.record_retry();
            tokio::time::sleep(backoff).await;
            backoff = std::cmp::min(backoff * 2, self.retry.max_backoff);
        }
    }

    async fn flush(&self, batch: Vec<Arc<Event>>) {
        if batch.is_empty() {
            return;
        }
        let batch_len = batch.len() as u64;
        match self.encode_batch(&batch) {
            Ok(body) => self.send_with_retry(body, batch_len).await,
            Err(e) => tracing::error!(exporter = %self.name, error = %e, "failed to encode batch"),
        }
    }
}

fn approx_size(event: &Event) -> usize {
    event.render_body().len() + 64
}

#[async_trait]
impl<B: EnvelopeBuilder + 'static> Exporter for HttpHecExporter<B> {
    fn name(&self) -> &str {
        &self.name
    }

    async fn run(
        self: Box<Self>,
        mut rx: mpsc::Receiver<Arc<Event>>,
        shutdown: CancellationToken,
    ) -> Result<(), SgError> {
        let mut batch: Vec<Arc<Event>> = Vec::new();
        let mut batch_bytes = 0usize;
        let mut ticker = tokio::time::interval(self.batch.flush_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            // Biased: prefer draining an already-buffered event over
            // noticing shutdown, so an event queued right as shutdown
            // fires still gets picked up and flushed rather than silently
            // dropped by the race between the two futures.
            tokio::select! {
                biased;

                event = rx.recv() => {
                    match event {
                        Some(event) => {
                            batch_bytes += approx_size(&event);
                            batch.push(event);
                            if batch.len() >= self.batch.max_events || batch_bytes >= self.batch.max_bytes {
                                self.flush(std::mem::take(&mut batch)).await;
                                batch_bytes = 0;
                            }
                        }
                        None => {
                            self.flush(std::mem::take(&mut batch)).await;
                            break;
                        }
                    }
                }
                _ = ticker.tick() => {
                    if !batch.is_empty() {
                        self.flush(std::mem::take(&mut batch)).await;
                        batch_bytes = 0;
                    }
                }
                _ = shutdown.cancelled() => {
                    self.flush(std::mem::take(&mut batch)).await;
                    break;
                }
            }
        }
        Ok(())
    }
}
