//! Local status/monitoring API for a running `sgcia run` process. Bound
//! to loopback by default (the caller chooses the address); no auth --
//! the loopback binding is the security boundary. `GET /status` serves a
//! live metrics snapshot, `GET /config` serves the resolved config with
//! secrets redacted.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::Value;
use sg_config::RawConfig;
use sg_core::{Metrics, MetricsSnapshot};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct AppState {
    metrics: Arc<Metrics>,
    config: Arc<RawConfig>,
}

pub fn router(metrics: Arc<Metrics>, config: Arc<RawConfig>) -> Router {
    Router::new()
        .route("/status", get(get_status))
        .route("/config", get(get_config))
        .with_state(AppState { metrics, config })
}

pub async fn serve(
    addr: SocketAddr,
    metrics: Arc<Metrics>,
    config: Arc<RawConfig>,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router(metrics, config))
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await?;
    Ok(())
}

async fn get_status(State(state): State<AppState>) -> Json<MetricsSnapshot> {
    Json(state.metrics.snapshot())
}

async fn get_config(State(state): State<AppState>) -> Json<Value> {
    let mut value = serde_json::to_value(&*state.config).unwrap_or(Value::Null);
    redact(&mut value);
    Json(value)
}

/// Redact by key name rather than by "was this value originally a
/// `${VAR}` reference" -- by the time a resolved config reaches this
/// handler, env expansion has already happened and the original `${VAR}`
/// literal is gone. Redacting by key name is simpler and strictly safer:
/// it also catches a token written literally in YAML, which
/// provenance-tracking would miss.
const SENSITIVE_KEYS: &[&str] = &["token", "password", "secret", "api_key", "apikey"];

fn redact(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                if SENSITIVE_KEYS.contains(&k.to_lowercase().as_str()) {
                    *v = Value::String("***redacted***".to_string());
                } else {
                    redact(v);
                }
            }
        }
        Value::Array(arr) => arr.iter_mut().for_each(redact),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use serde_json::json;
    use tower::ServiceExt;

    fn sample_config() -> RawConfig {
        let yaml = r#"
receivers:
  syslog/udp:
    protocol: udp
    listen_address: "0.0.0.0:514"
exporters:
  sentinelone_hec:
    type: s1hec
    endpoint: "https://example.invalid/services/collector/event"
    token: "super-secret-token"
service:
  pipelines:
    logs/syslog:
      receivers: [syslog/udp]
      exporters: [sentinelone_hec]
"#;
        sg_config::load_str(yaml).unwrap()
    }

    #[test]
    fn redact_hides_sensitive_keys_at_any_depth() {
        let mut value = json!({
            "outer": {
                "token": "secret-value",
                "nested": [{"password": "hunter2"}, {"keep": "me"}]
            }
        });
        redact(&mut value);
        assert_eq!(value["outer"]["token"], "***redacted***");
        assert_eq!(value["outer"]["nested"][0]["password"], "***redacted***");
        assert_eq!(value["outer"]["nested"][1]["keep"], "me");
    }

    #[tokio::test]
    async fn status_endpoint_returns_metrics_snapshot() {
        let metrics = Arc::new(Metrics::new(
            vec!["syslog/udp".to_string()],
            vec!["logs/syslog".to_string()],
            vec!["sentinelone_hec".to_string()],
        ));
        metrics.receiver("syslog/udp").events_in.inc();

        let app = router(metrics, Arc::new(sample_config()));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["receivers"]["syslog/udp"]["events_in"], 1);
    }

    #[tokio::test]
    async fn config_endpoint_redacts_token() {
        let metrics = Arc::new(Metrics::new(vec![], vec![], vec![]));
        let app = router(metrics, Arc::new(sample_config()));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            json["exporters"]["sentinelone_hec"]["token"],
            "***redacted***"
        );
        assert_eq!(
            json["exporters"]["sentinelone_hec"]["endpoint"],
            "https://example.invalid/services/collector/event"
        );
    }
}
