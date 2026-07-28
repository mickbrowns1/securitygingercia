use sg_core::{Event, ReceiverMetrics};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Forwards every event from a receiver's own inner channel into the
/// pipeline's shared channel, incrementing `events_in` per item. Kept as
/// a thin orchestration-layer wrapper rather than a change to the
/// `Receiver` trait itself -- receivers stay opaque `Box<dyn Receiver>`
/// values with no metrics awareness.
pub fn spawn_counting_relay(
    mut inner_rx: mpsc::Receiver<Event>,
    out_tx: mpsc::Sender<Event>,
    metrics: Arc<ReceiverMetrics>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(event) = inner_rx.recv().await {
            metrics.events_in.inc();
            if out_tx.send(event).await.is_err() {
                break;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn forwards_events_and_counts_them() {
        let (inner_tx, inner_rx) = mpsc::channel(4);
        let (out_tx, mut out_rx) = mpsc::channel(4);
        let metrics = Arc::new(ReceiverMetrics::default());

        let handle = spawn_counting_relay(inner_rx, out_tx, metrics.clone());

        inner_tx
            .send(Event::new(bytes::Bytes::from_static(b"one")))
            .await
            .unwrap();
        inner_tx
            .send(Event::new(bytes::Bytes::from_static(b"two")))
            .await
            .unwrap();
        drop(inner_tx);

        let first = out_rx.recv().await.unwrap();
        let second = out_rx.recv().await.unwrap();
        assert_eq!(first.render_body(), "one");
        assert_eq!(second.render_body(), "two");
        assert!(out_rx.recv().await.is_none());

        handle.await.unwrap();
        assert_eq!(metrics.events_in.get(), 2);
    }

    #[tokio::test]
    async fn stops_when_downstream_channel_closes() {
        let (inner_tx, inner_rx) = mpsc::channel(4);
        let (out_tx, out_rx) = mpsc::channel(4);
        let metrics = Arc::new(ReceiverMetrics::default());
        drop(out_rx); // downstream gone before anything is sent

        let handle = spawn_counting_relay(inner_rx, out_tx, metrics.clone());
        inner_tx
            .send(Event::new(bytes::Bytes::from_static(b"x")))
            .await
            .unwrap();

        handle.await.unwrap();
        assert_eq!(metrics.events_in.get(), 1);
    }
}
