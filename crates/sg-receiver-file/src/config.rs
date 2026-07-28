use serde::Deserialize;
use sg_config::parse_duration;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StartAt {
    Beginning,
    #[default]
    End,
}

fn default_poll_interval() -> String {
    "500ms".to_string()
}

#[derive(Debug, Deserialize)]
struct FileLogConfigDef {
    include: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(default)]
    start_at: StartAt,
    #[serde(default = "default_poll_interval")]
    poll_interval: String,
    checkpoint_file: PathBuf,
}

#[derive(Debug, Clone)]
pub struct FileLogConfig {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub start_at: StartAt,
    pub poll_interval: Duration,
    pub checkpoint_file: PathBuf,
}

impl FileLogConfig {
    pub fn from_value(id: &str, value: &serde_json::Value) -> Result<Self, String> {
        let def: FileLogConfigDef =
            serde_json::from_value(value.clone()).map_err(|e| format!("{id}: {e}"))?;
        let poll_interval = parse_duration(&def.poll_interval)
            .map_err(|e| format!("{id}: invalid poll_interval: {e}"))?;
        Ok(Self {
            include: def.include,
            exclude: def.exclude,
            start_at: def.start_at,
            poll_interval,
            checkpoint_file: def.checkpoint_file,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn applies_defaults_and_parses_poll_interval() {
        let cfg = FileLogConfig::from_value(
            "filelog/app",
            &json!({
                "include": ["/var/log/app/*.log"],
                "checkpoint_file": "/tmp/app.checkpoint.json",
            }),
        )
        .unwrap();
        assert_eq!(cfg.start_at, StartAt::End);
        assert_eq!(cfg.poll_interval, Duration::from_millis(500));
    }

    #[test]
    fn rejects_invalid_poll_interval() {
        assert!(FileLogConfig::from_value(
            "filelog/app",
            &json!({
                "include": ["/var/log/app/*.log"],
                "checkpoint_file": "/tmp/app.checkpoint.json",
                "poll_interval": "5x",
            })
        )
        .is_err());
    }
}
