use sg_core::{Event, Exporter, ExporterMetrics};
use sg_exporter_core::StdoutExporter;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn drains_events_until_channel_closes_and_records_metrics() {
    let (tx, rx) = mpsc::channel(4);
    let shutdown = CancellationToken::new();
    let metrics = Arc::new(ExporterMetrics::default());
    let exporter = Box::new(StdoutExporter::new("debug", metrics.clone()));

    let handle = tokio::spawn(exporter.run(rx, shutdown));

    tx.send(Arc::new(Event::new(bytes::Bytes::from_static(b"hello"))))
        .await
        .unwrap();
    drop(tx);

    handle.await.unwrap().unwrap();

    assert_eq!(metrics.snapshot().batches_sent, 1);
}
