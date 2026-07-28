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
