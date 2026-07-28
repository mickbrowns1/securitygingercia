use crate::config::{Protocol, SyslogConfig};
use crate::{tcp, udp};
use async_trait::async_trait;
use sg_core::{Event, Receiver, SgError};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub struct SyslogReceiver {
    name: String,
    config: SyslogConfig,
}

impl SyslogReceiver {
    pub fn new(name: impl Into<String>, config: SyslogConfig) -> Self {
        Self {
            name: name.into(),
            config,
        }
    }
}

#[async_trait]
impl Receiver for SyslogReceiver {
    fn name(&self) -> &str {
        &self.name
    }

    async fn run(
        self: Box<Self>,
        tx: mpsc::Sender<Event>,
        shutdown: CancellationToken,
    ) -> Result<(), SgError> {
        match self.config.protocol {
            Protocol::Udp => udp::run(&self.name, &self.config, tx, shutdown).await,
            Protocol::Tcp => tcp::run(&self.name, &self.config, tx, shutdown).await,
        }
    }
}
