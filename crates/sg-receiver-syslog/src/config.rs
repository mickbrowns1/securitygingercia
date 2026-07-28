use serde::Deserialize;
use serde_json::Value;
use std::net::SocketAddr;
use syslog_loose::Variant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    Udp,
    Tcp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RfcMode {
    #[default]
    Auto,
    Rfc3164,
    Rfc5424,
}

impl RfcMode {
    pub fn to_variant(self) -> Variant {
        match self {
            RfcMode::Auto => Variant::Either,
            RfcMode::Rfc3164 => Variant::RFC3164,
            RfcMode::Rfc5424 => Variant::RFC5424,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FramingMode {
    #[default]
    Auto,
    OctetCounting,
    NonTransparent,
}

fn default_max_message_size() -> usize {
    65536
}

#[derive(Debug, Deserialize)]
struct SyslogConfigDef {
    protocol: Protocol,
    listen_address: String,
    #[serde(default)]
    rfc: RfcMode,
    #[serde(default)]
    framing: FramingMode,
    #[serde(default = "default_max_message_size")]
    max_message_size: usize,
}

#[derive(Debug, Clone)]
pub struct SyslogConfig {
    pub protocol: Protocol,
    pub listen_address: SocketAddr,
    pub rfc: RfcMode,
    pub framing: FramingMode,
    pub max_message_size: usize,
}

impl SyslogConfig {
    pub fn from_value(id: &str, value: &Value) -> Result<Self, String> {
        let def: SyslogConfigDef =
            serde_json::from_value(value.clone()).map_err(|e| format!("{id}: {e}"))?;
        let listen_address = def
            .listen_address
            .parse()
            .map_err(|e| format!("{id}: invalid listen_address '{}': {e}", def.listen_address))?;
        Ok(Self {
            protocol: def.protocol,
            listen_address,
            rfc: def.rfc,
            framing: def.framing,
            max_message_size: def.max_message_size,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_udp_config_with_defaults() {
        let cfg = SyslogConfig::from_value(
            "syslog/udp",
            &json!({"protocol": "udp", "listen_address": "0.0.0.0:514"}),
        )
        .unwrap();
        assert_eq!(cfg.protocol, Protocol::Udp);
        assert_eq!(cfg.rfc, RfcMode::Auto);
        assert_eq!(cfg.max_message_size, 65536);
    }

    #[test]
    fn rejects_invalid_listen_address() {
        assert!(SyslogConfig::from_value(
            "syslog/tcp",
            &json!({"protocol": "tcp", "listen_address": "not-an-address"})
        )
        .is_err());
    }
}
