use crate::error::SgError;
use crate::event::Event;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[async_trait]
pub trait Exporter: Send + Sync {
    fn name(&self) -> &str;

    async fn run(
        self: Box<Self>,
        rx: mpsc::Receiver<Arc<Event>>,
        shutdown: CancellationToken,
    ) -> Result<(), SgError>;
}
