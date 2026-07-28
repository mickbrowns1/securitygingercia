use crate::error::SgError;
use crate::event::Event;
use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Common integration surface for syslog/file/Windows-Event-Log receivers.
/// A receiver owns its own long-lived task; it constructs `Event`s and
/// pushes them onto the pipeline's shared channel until `shutdown` fires
/// or the channel closes.
#[async_trait]
pub trait Receiver: Send + Sync {
    fn name(&self) -> &str;

    async fn run(
        self: Box<Self>,
        tx: mpsc::Sender<Event>,
        shutdown: CancellationToken,
    ) -> Result<(), SgError>;
}
