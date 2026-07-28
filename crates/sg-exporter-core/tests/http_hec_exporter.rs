use serde_json::{json, Value};
use sg_core::{Event, Exporter, ExporterMetrics};
use sg_exporter_core::{BatchConfig, EnvelopeBuilder, EnvelopeError, HttpHecExporter, RetryPolicy};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct DummyBuilder;
impl EnvelopeBuilder for DummyBuilder {
    fn build(&self, event: &Event) -> Result<Value, EnvelopeError> {
        Ok(json!({"event": event.render_body()}))
    }
}

fn small_batch() -> BatchConfig {
    BatchConfig {
        max_events: 2,
        max_bytes: 1_000_000,
        flush_interval: Duration::from_secs(60),
    }
}

fn fast_retry() -> RetryPolicy {
    RetryPolicy {
        max_attempts: 4,
        initial_backoff: Duration::from_millis(10),
        max_backoff: Duration::from_millis(50),
    }
}

#[tokio::test]
async fn batches_are_newline_delimited_json_with_auth_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/services/collector/event"))
        .and(header("Authorization", "Splunk test-token"))
        .and(header("Content-Type", "application/json"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let endpoint = format!("{}/services/collector/event", server.uri())
        .parse()
        .unwrap();
    let metrics = Arc::new(ExporterMetrics::default());
    let exporter = Box::new(HttpHecExporter::new(
        "test",
        endpoint,
        "test-token",
        DummyBuilder,
        small_batch(),
        fast_retry(),
        metrics.clone(),
    ));

    let (tx, rx) = mpsc::channel(8);
    let shutdown = CancellationToken::new();
    let handle = tokio::spawn(exporter.run(rx, shutdown.clone()));

    tx.send(Arc::new(Event::new(bytes::Bytes::from_static(b"one"))))
        .await
        .unwrap();
    tx.send(Arc::new(Event::new(bytes::Bytes::from_static(b"two"))))
        .await
        .unwrap();

    // max_events=2 triggers an immediate flush -- give it a moment.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let body = String::from_utf8(requests[0].body.clone()).unwrap();
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(lines.len(), 2, "expected exactly two newline-delimited JSON objects");
    assert_eq!(
        serde_json::from_str::<Value>(lines[0]).unwrap(),
        json!({"event": "one"})
    );
    assert_eq!(
        serde_json::from_str::<Value>(lines[1]).unwrap(),
        json!({"event": "two"})
    );

    let snap = metrics.snapshot();
    assert_eq!(snap.batches_sent, 1);
    assert_eq!(snap.events_in, 2);
    assert_eq!(snap.batches_failed, 0);

    shutdown.cancel();
    handle.await.unwrap().unwrap();
}

#[tokio::test]
async fn retries_on_server_error_then_succeeds() {
    let server = MockServer::start().await;

    // First two attempts fail with 500, third succeeds.
    let attempts = Arc::new(AtomicUsize::new(0));
    Mock::given(method("POST"))
        .and(path("/collector"))
        .respond_with(move |_req: &wiremock::Request| {
            let n = attempts.fetch_add(1, Ordering::SeqCst);
            if n < 2 {
                ResponseTemplate::new(500)
            } else {
                ResponseTemplate::new(200)
            }
        })
        .expect(3)
        .mount(&server)
        .await;

    let endpoint = format!("{}/collector", server.uri()).parse().unwrap();
    let metrics = Arc::new(ExporterMetrics::default());
    let exporter = Box::new(HttpHecExporter::new(
        "test",
        endpoint,
        "tok",
        DummyBuilder,
        BatchConfig {
            max_events: 1,
            max_bytes: 1_000_000,
            flush_interval: Duration::from_secs(60),
        },
        fast_retry(),
        metrics.clone(),
    ));

    let (tx, rx) = mpsc::channel(8);
    let shutdown = CancellationToken::new();
    let handle = tokio::spawn(exporter.run(rx, shutdown.clone()));

    tx.send(Arc::new(Event::new(bytes::Bytes::from_static(b"only"))))
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(500)).await;

    let snap = metrics.snapshot();
    assert_eq!(snap.batches_sent, 1);
    assert_eq!(snap.retries, 2);
    assert_eq!(snap.batches_failed, 0);

    shutdown.cancel();
    handle.await.unwrap().unwrap();
    // wiremock's `.expect(3)` assertion (verified on drop) confirms all
    // three attempts happened.
}

#[tokio::test]
async fn gives_up_after_max_attempts_and_records_failure_with_last_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .expect(4) // fast_retry().max_attempts
        .mount(&server)
        .await;

    let endpoint = server.uri().parse().unwrap();
    let metrics = Arc::new(ExporterMetrics::default());
    let exporter = Box::new(HttpHecExporter::new(
        "test",
        endpoint,
        "tok",
        DummyBuilder,
        BatchConfig {
            max_events: 1,
            max_bytes: 1_000_000,
            flush_interval: Duration::from_secs(60),
        },
        fast_retry(),
        metrics.clone(),
    ));

    let (tx, rx) = mpsc::channel(8);
    let shutdown = CancellationToken::new();
    let handle = tokio::spawn(exporter.run(rx, shutdown.clone()));

    tx.send(Arc::new(Event::new(bytes::Bytes::from_static(b"never"))))
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(500)).await;

    let snap = metrics.snapshot();
    assert_eq!(snap.batches_sent, 0);
    assert_eq!(snap.batches_failed, 1);
    assert_eq!(snap.retries, 3); // 3 retries before giving up on the 4th attempt
    assert!(snap.last_error.unwrap().message.contains("503"));

    shutdown.cancel();
    handle.await.unwrap().unwrap();
}

#[tokio::test]
async fn shutdown_flushes_remaining_partial_batch() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let endpoint = server.uri().parse().unwrap();
    let metrics = Arc::new(ExporterMetrics::default());
    let exporter = Box::new(HttpHecExporter::new(
        "test",
        endpoint,
        "tok",
        DummyBuilder,
        BatchConfig {
            max_events: 100, // never reached -- only shutdown flush should fire
            max_bytes: 1_000_000,
            flush_interval: Duration::from_secs(60),
        },
        fast_retry(),
        metrics,
    ));

    let (tx, rx) = mpsc::channel(8);
    let shutdown = CancellationToken::new();
    let handle = tokio::spawn(exporter.run(rx, shutdown.clone()));

    tx.send(Arc::new(Event::new(bytes::Bytes::from_static(b"partial"))))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    shutdown.cancel();
    handle.await.unwrap().unwrap();
}
