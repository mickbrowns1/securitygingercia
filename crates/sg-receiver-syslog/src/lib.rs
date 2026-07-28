//! Syslog receiver: UDP + TCP, RFC 3164 / RFC 5424 (via `syslog_loose`),
//! RFC 6587 octet-counting and non-transparent TCP framing.

mod config;
mod framing;
mod parser;
mod receiver;
mod tcp;
mod udp;

pub use config::{FramingMode, Protocol, RfcMode, SyslogConfig};
pub use receiver::SyslogReceiver;
