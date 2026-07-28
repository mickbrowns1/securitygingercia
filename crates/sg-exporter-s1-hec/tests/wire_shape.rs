use serde_json::{json, Value};
use sg_core::{Event, Exporter, ExporterMetrics};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{header, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// End-to-end: config -> `sg_exporter_s1_hec::build` -> real batching
/// HTTP exporter -> a mocked HEC endpoint. Confirms the exact envelope
/// invariant that matters most: only `time`/`host`/`event`/`fields` at
/// the top level, or DataPipeline prefixes stray keys with `splunk_`.
#[tokio::test]
async fn built_exporter_posts_envelopes_with_exact_top_level_keys() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("Authorization", "Splunk from-config-token"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let cfg = json!({
        "endpoint": server.uri(),
        "token": "from-config-token",
        "sourcetype": "cisco_asa_ts_parser",
        "datasource": "cisco_asa",
        "batch": {"max_events": 1, "max_bytes": 1000000, "flush_interval": "60s"},
        "retry": {"max_attempts": 1, "initial_backoff": "10ms", "max_backoff": "10ms"},
    });
    let metrics = Arc::new(ExporterMetrics::default());
    let exporter = Box::new(sg_exporter_s1_hec::build("sentinelone_hec", &cfg, metrics).unwrap());

    let (tx, rx) = mpsc::channel(4);
    let shutdown = CancellationToken::new();
    let handle = tokio::spawn(exporter.run(rx, shutdown.clone()));

    tx.send(Arc::new(Event::new(bytes::Bytes::from_static(
        b"%ASA-6-302013: Built inbound TCP connection",
    ))))
    .await
    .unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let body = String::from_utf8(requests[0].body.clone()).unwrap();
    let line = body.lines().next().unwrap();
    let envelope: Value = serde_json::from_str(line).unwrap();

    let keys: HashSet<&str> = envelope.as_object().unwrap().keys().map(|s| s.as_str()).collect();
    assert_eq!(keys, HashSet::from(["time", "host", "event", "fields"]));
    assert_eq!(envelope["fields"]["sourcetype"], "cisco_asa_ts_parser");
    assert_eq!(envelope["fields"]["datasource"], "cisco_asa");

    shutdown.cancel();
    handle.await.unwrap().unwrap();
}
