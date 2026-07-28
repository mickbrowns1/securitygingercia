use crate::config::SyslogConfig;
use crate::framing::FrameReader;
use crate::parser::parse_into_event;
use sg_core::{Event, SgError};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub async fn run(
    name: &str,
    config: &SyslogConfig,
    tx: mpsc::Sender<Event>,
    shutdown: CancellationToken,
) -> Result<(), SgError> {
    let listener = TcpListener::bind(config.listen_address).await?;
    tracing::info!(receiver = %name, addr = %config.listen_address, "syslog TCP listening");

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            res = listener.accept() => {
                let (stream, peer) = match res {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(receiver = %name, error = %e, "tcp accept error");
                        continue;
                    }
                };
                tracing::debug!(receiver = %name, %peer, "accepted syslog TCP connection");
                let conn_tx = tx.clone();
                let conn_shutdown = shutdown.clone();
                let conn_name = name.to_string();
                let framing = config.framing;
                let rfc = config.rfc;
                let max_message_size = config.max_message_size;
                tokio::spawn(async move {
                    handle_connection(conn_name, stream, framing, rfc, max_message_size, conn_tx, conn_shutdown).await;
                });
            }
        }
    }
    Ok(())
}

async fn handle_connection(
    name: String,
    stream: tokio::net::TcpStream,
    framing: crate::config::FramingMode,
    rfc: crate::config::RfcMode,
    max_message_size: usize,
    tx: mpsc::Sender<Event>,
    shutdown: CancellationToken,
) {
    let mut reader = FrameReader::new(stream, framing, max_message_size);
    loop {
        let frame = tokio::select! {
            _ = shutdown.cancelled() => break,
            res = reader.next_frame() => res,
        };
        match frame {
            Ok(Some(bytes)) => {
                let event = parse_into_event(bytes, rfc, &name);
                if tx.send(event).await.is_err() {
                    break;
                }
            }
            Ok(None) => break, // connection closed cleanly
            Err(e) => {
                tracing::warn!(receiver = %name, error = %e, "syslog TCP framing error, closing connection");
                break;
            }
        }
    }
}

