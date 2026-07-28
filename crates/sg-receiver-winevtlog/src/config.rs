use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StartAt {
    Beginning,
    #[default]
    End,
}

fn default_query() -> String {
    "*".to_string()
}

#[derive(Debug, Deserialize)]
struct WinEventLogConfigDef {
    channel: String,
    #[serde(default = "default_query")]
    query: String,
    #[serde(default)]
    start_at: StartAt,
    bookmark_file: PathBuf,
}

#[derive(Debug, Clone)]
pub struct WinEventLogConfig {
    pub channel: String,
    pub query: String,
    pub start_at: StartAt,
    pub bookmark_file: PathBuf,
}

impl WinEventLogConfig {
    pub fn from_value(id: &str, value: &serde_json::Value) -> Result<Self, String> {
        let def: WinEventLogConfigDef =
            serde_json::from_value(value.clone()).map_err(|e| format!("{id}: {e}"))?;
        Ok(Self {
            channel: def.channel,
            query: def.query,
            start_at: def.start_at,
            bookmark_file: def.bookmark_file,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn applies_defaults() {
        let cfg = WinEventLogConfig::from_value(
            "windows_eventlog/security",
            &json!({"channel": "Security", "bookmark_file": "C:\\bookmark.xml"}),
        )
        .unwrap();
        assert_eq!(cfg.query, "*");
        assert_eq!(cfg.start_at, StartAt::End);
    }
}
