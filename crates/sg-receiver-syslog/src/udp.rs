use crate::config::SyslogConfig;
use crate::parser::parse_into_event;
use sg_core::{Event, SgError};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub async fn run(
    name: &str,
    config: &SyslogConfig,
    tx: mpsc::Sender<Event>,
    shutdown: CancellationToken,
) -> Result<(), SgError> {
    let socket = UdpSocket::bind(config.listen_address).await?;
    tracing::info!(receiver = %name, addr = %config.listen_address, "syslog UDP listening");

    let mut buf = vec![0u8; config.max_message_size];
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            res = socket.recv_from(&mut buf) => {
                let (n, _peer) = match res {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(receiver = %name, error = %e, "udp recv error");
                        continue;
                    }
                };
                let raw = bytes::Bytes::copy_from_slice(&buf[..n]);
                let event = parse_into_event(raw, config.rfc, name);
                // A full channel here means the OS socket buffer keeps
                // filling and the kernel drops datagrams -- UDP can't be
                // backpressured any other way. Acceptable per design; not
                // separately counted in this build phase.
                if tx.send(event).await.is_err() {
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}
