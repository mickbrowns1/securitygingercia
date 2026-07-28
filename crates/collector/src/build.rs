use sg_config::component_type;
use sg_core::{Exporter, ExporterMetrics, Receiver};
use std::sync::Arc;

pub fn build_receiver(id: &str, value: &serde_json::Value) -> anyhow::Result<Box<dyn Receiver>> {
    match component_type(id) {
        "syslog" => {
            let cfg = sg_receiver_syslog::SyslogConfig::from_value(id, value)
                .map_err(anyhow::Error::msg)?;
            Ok(Box::new(sg_receiver_syslog::SyslogReceiver::new(id, cfg)))
        }
        "filelog" => {
            let cfg =
                sg_receiver_file::FileLogConfig::from_value(id, value).map_err(anyhow::Error::msg)?;
            Ok(Box::new(sg_receiver_file::FileLogReceiver::new(id, cfg)))
        }
        "windows_eventlog" => build_windows_eventlog_receiver(id, value),
        other => anyhow::bail!("receiver '{id}': unknown receiver type '{other}'"),
    }
}

// UNVERIFIED: sg-receiver-winevtlog's EvtSubscribe wiring has only been
// type-checked (`cargo check --target x86_64-pc-windows-msvc`), never run
// against a real Windows Event Log service. Confirm on a real Windows host
// or CI before relying on it.
#[cfg(windows)]
fn build_windows_eventlog_receiver(
    id: &str,
    value: &serde_json::Value,
) -> anyhow::Result<Box<dyn Receiver>> {
    let cfg = sg_receiver_winevtlog::WinEventLogConfig::from_value(id, value)
        .map_err(anyhow::Error::msg)?;
    Ok(Box::new(sg_receiver_winevtlog::WinEventLogReceiver::new(
        id, cfg,
    )))
}

#[cfg(not(windows))]
fn build_windows_eventlog_receiver(
    id: &str,
    _value: &serde_json::Value,
) -> anyhow::Result<Box<dyn Receiver>> {
    anyhow::bail!(
        "receiver '{id}': windows_eventlog is only available on Windows builds (this binary was \
         built for '{}'); it cannot be built or tested on this platform",
        std::env::consts::OS
    )
}

pub fn build_exporter(
    id: &str,
    value: &serde_json::Value,
    metrics: Arc<ExporterMetrics>,
) -> anyhow::Result<Box<dyn Exporter>> {
    let type_str = value
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| component_type(id));
    match type_str {
        "s1hec" => Ok(Box::new(
            sg_exporter_s1_hec::build(id, value, metrics).map_err(anyhow::Error::msg)?,
        )),
        "splunkhec" => Ok(Box::new(
            sg_exporter_splunk_hec::build(id, value, metrics).map_err(anyhow::Error::msg)?,
        )),
        "stdout" => Ok(Box::new(sg_exporter_core::StdoutExporter::new(id, metrics))),
        other => anyhow::bail!("exporter '{id}': unknown exporter type '{other}'"),
    }
}
