use sg_core::MetricsSnapshot;
use std::net::SocketAddr;

pub async fn fetch_status(
    client: &reqwest::Client,
    addr: SocketAddr,
) -> Result<MetricsSnapshot, String> {
    let url = format!("http://{addr}/status");
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("connection failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("status API returned {}", resp.status()));
    }
    resp.json::<MetricsSnapshot>()
        .await
        .map_err(|e| format!("invalid response: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use axum::Router;

    // Captured verbatim from a real run of otelcol's statuscfgextension
    // (otelcol/extensions/statuscfgextension) -- this is the pivot's load-
    // bearing contract: the Go extension's /status output must deserialize
    // into the exact same sg_core::MetricsSnapshot the old Rust status API
    // produced, with no changes needed here.
    const GO_EXTENSION_STATUS_JSON: &str = r#"{"started_at":"2026-08-03T12:51:24.878639-04:00","uptime_seconds":19,"receivers":{"file_log/app":{"events_in":0},"syslog/tcp":{"events_in":1},"syslog/udp":{"events_in":0},"windows_event_log/security":{"events_in":0}},"pipelines":{"logs/files":{"events_in":0,"events_out":1,"events_dropped":0,"events_dead_lettered":0,"parse_errors":0},"logs/syslog":{"events_in":1,"events_out":1,"events_dropped":0,"events_dead_lettered":0,"parse_errors":0},"logs/windows":{"events_in":0,"events_out":1,"events_dropped":0,"events_dead_lettered":0,"parse_errors":0}},"exporters":{"splunk_hec/sentinelone":{"events_in":1,"batches_sent":1,"batches_failed":0,"retries":0,"last_error":null},"splunk_hec/splunk":{"events_in":0,"batches_sent":0,"batches_failed":0,"retries":0,"last_error":null}}}"#;

    #[tokio::test]
    async fn fetch_status_deserializes_go_extension_response() {
        let app = Router::new().route(
            "/status",
            get(|| async {
                (
                    [("content-type", "application/json")],
                    GO_EXTENSION_STATUS_JSON,
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let snapshot = fetch_status(&client, addr).await.unwrap();

        assert_eq!(snapshot.uptime_seconds, 19);
        assert_eq!(snapshot.receivers["syslog/tcp"].events_in, 1);
        assert_eq!(snapshot.pipelines["logs/syslog"].events_out, 1);
        assert_eq!(
            snapshot.exporters["splunk_hec/sentinelone"].batches_sent,
            1
        );
        assert!(snapshot.exporters["splunk_hec/splunk"].last_error.is_none());
    }
}
