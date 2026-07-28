use crate::editor::schema_registry::ComponentCategory;
use serde_json::{Map, Value};
use sg_config::RawConfig;
use std::io::Write;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum EditorError {
    #[error("failed to read {0}: {1}")]
    Io(String, String),
    #[error("failed to parse YAML: {0}")]
    Parse(String),
    #[error("failed to serialize YAML: {0}")]
    Serialize(String),
    #[error("invalid config: {0}")]
    Invalid(String),
}

/// In-memory config document, kept as plain `serde_json::Map`s (already
/// order-preserving under the workspace's `serde_json` `preserve_order`
/// feature) rather than `sg_config::RawConfig` directly, so that:
/// - unknown/not-yet-registered fields survive a load -> edit -> save
///   round trip untouched (only fields the user actually edits change),
/// - `${VAR}` tokens inside string values are preserved byte-for-byte,
///   since they're never parsed as anything but opaque strings here.
#[derive(Debug, Clone, Default)]
pub struct EditorDoc {
    pub receivers: Map<String, Value>,
    pub operators: Map<String, Value>,
    pub exporters: Map<String, Value>,
    /// Each value is an object `{receivers: [...], operators: [...],
    /// exporters: [...]}`, matching `sg_config::PipelineConfig`'s shape.
    pub pipelines: Map<String, Value>,
}

impl EditorDoc {
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads a config file, skipping `sg_config::expand_env` on purpose
    /// so `${VAR}` tokens stay literal in memory. A missing file loads as
    /// an empty (fresh) document rather than an error, so `sgcia edit
    /// --config new.yaml` can build one from scratch.
    pub fn load(path: &Path) -> Result<Self, EditorError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::new()),
            Err(e) => Err(EditorError::Io(path.display().to_string(), e.to_string())),
        }
    }

    pub fn parse(text: &str) -> Result<Self, EditorError> {
        let doc: Value =
            serde_yaml_ng::from_str(text).map_err(|e| EditorError::Parse(e.to_string()))?;
        let obj = doc.as_object().cloned().unwrap_or_default();
        let get_map = |key: &str| {
            obj.get(key)
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default()
        };
        let pipelines = obj
            .get("service")
            .and_then(|s| s.get("pipelines"))
            .and_then(|p| p.as_object())
            .cloned()
            .unwrap_or_default();
        Ok(Self {
            receivers: get_map("receivers"),
            operators: get_map("operators"),
            exporters: get_map("exporters"),
            pipelines,
        })
    }

    pub fn to_value(&self) -> Value {
        serde_json::json!({
            "receivers": self.receivers,
            "operators": self.operators,
            "exporters": self.exporters,
            "service": { "pipelines": self.pipelines },
        })
    }

    /// Structural validation (`sg_config::RawConfig::validate` -- unknown
    /// pipeline references, empty receiver/exporter lists) plus a deeper,
    /// best-effort pass running each component's *real* `from_value`/
    /// operator builder against a clone with any `${...}`-looking string
    /// replaced by a harmless placeholder first, so validation doesn't
    /// require the real secret to be present on the editing machine (the
    /// placeholder is never written to disk -- only used in-memory here).
    pub fn validate(&self) -> Result<(), EditorError> {
        let raw: RawConfig = serde_json::from_value(self.to_value())
            .map_err(|e| EditorError::Invalid(format!("structurally invalid: {e}")))?;
        raw.validate()
            .map_err(|e| EditorError::Invalid(e.to_string()))?;

        for (id, def) in &self.receivers {
            validate_receiver(id, &placeholder_for_validation(def))?;
        }
        for (id, def) in &self.exporters {
            validate_exporter(id, &placeholder_for_validation(def))?;
        }
        for (id, def) in &self.operators {
            sg_operators::build_one(id, &placeholder_for_validation(def))
                .map_err(|e| EditorError::Invalid(e.to_string()))?;
        }
        Ok(())
    }

    /// Validates, then atomically writes (temp file in the same
    /// directory + `fsync` + `rename`) -- never overwrites the good file
    /// on disk with an invalid one.
    pub fn save(&self, path: &Path) -> Result<(), EditorError> {
        self.validate()?;
        let yaml_text = serde_yaml_ng::to_string(&self.to_value())
            .map_err(|e| EditorError::Serialize(e.to_string()))?;
        atomic_write(path, &yaml_text)
            .map_err(|e| EditorError::Io(path.display().to_string(), e.to_string()))?;
        Ok(())
    }

    /// Names of pipelines that still reference `id` in the given
    /// category's list.
    pub fn pipelines_referencing(&self, category: ComponentCategory, id: &str) -> Vec<String> {
        let key = category_key(category);
        let mut names: Vec<String> = self
            .pipelines
            .iter()
            .filter(|(_, def)| {
                def.get(key)
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().any(|v| v.as_str() == Some(id)))
                    .unwrap_or(false)
            })
            .map(|(name, _)| name.clone())
            .collect();
        names.sort();
        names
    }

    /// Removes `id` from every pipeline's list for the given category --
    /// call before actually deleting the component so no dangling
    /// reference survives to fail validation on next save.
    pub fn strip_from_pipelines(&mut self, category: ComponentCategory, id: &str) {
        let key = category_key(category);
        for def in self.pipelines.values_mut() {
            if let Some(arr) = def.get_mut(key).and_then(|v| v.as_array_mut()) {
                arr.retain(|v| v.as_str() != Some(id));
            }
        }
    }
}

fn category_key(category: ComponentCategory) -> &'static str {
    match category {
        ComponentCategory::Receiver => "receivers",
        ComponentCategory::Operator => "operators",
        ComponentCategory::Exporter => "exporters",
    }
}

fn placeholder_for_validation(value: &Value) -> Value {
    match value {
        Value::String(s) if s.contains("${") => Value::String("placeholder-value".to_string()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), placeholder_for_validation(v)))
                .collect(),
        ),
        Value::Array(arr) => Value::Array(arr.iter().map(placeholder_for_validation).collect()),
        other => other.clone(),
    }
}

fn validate_receiver(id: &str, value: &Value) -> Result<(), EditorError> {
    match sg_config::component_type(id) {
        "syslog" => sg_receiver_syslog::SyslogConfig::from_value(id, value).map(|_| ()),
        "filelog" => sg_receiver_file::FileLogConfig::from_value(id, value).map(|_| ()),
        "windows_eventlog" => {
            sg_receiver_winevtlog::WinEventLogConfig::from_value(id, value).map(|_| ())
        }
        other => Err(format!("unknown receiver type '{other}'")),
    }
    .map_err(EditorError::Invalid)
}

fn validate_exporter(id: &str, value: &Value) -> Result<(), EditorError> {
    let type_str = value
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| sg_config::component_type(id));
    match type_str {
        "s1hec" => sg_exporter_s1_hec::S1HecConfig::from_value(id, value).map(|_| ()),
        "splunkhec" => sg_exporter_splunk_hec::SplunkHecConfig::from_value(id, value).map(|_| ()),
        "stdout" => Ok(()),
        other => Err(format!("unknown exporter type '{other}'")),
    }
    .map_err(EditorError::Invalid)
}

fn atomic_write(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let tmp_path = path.with_extension("yaml.tmp");
    {
        let mut file = std::fs::File::create(&tmp_path)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SAMPLE: &str = r#"
receivers:
  filelog/app:
    include: ["/var/log/app/*.log"]
    checkpoint_file: "/var/lib/sgcia/app.checkpoint.json"
exporters:
  sentinelone_hec:
    type: s1hec
    endpoint: "https://xdr.us1.sentinelone.net/services/collector/event"
    token: ${S1_HEC_TOKEN}
    sourcetype: "app_ts_parser"
service:
  pipelines:
    logs/app:
      receivers: [filelog/app]
      exporters: [sentinelone_hec]
"#;

    #[test]
    fn missing_file_loads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let doc = EditorDoc::load(&dir.path().join("does-not-exist.yaml")).unwrap();
        assert!(doc.receivers.is_empty());
        assert!(doc.pipelines.is_empty());
    }

    #[test]
    fn load_preserves_var_reference_literally() {
        let doc = EditorDoc::parse(SAMPLE).unwrap();
        assert_eq!(
            doc.exporters["sentinelone_hec"]["token"],
            "${S1_HEC_TOKEN}"
        );
    }

    #[test]
    fn validate_passes_on_a_well_formed_document() {
        let doc = EditorDoc::parse(SAMPLE).unwrap();
        doc.validate().unwrap();
    }

    #[test]
    fn save_and_reload_round_trips_including_var_reference() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/config.yaml");
        let mut doc = EditorDoc::parse(SAMPLE).unwrap();

        // Mutate one field in memory, then save.
        doc.exporters
            .get_mut("sentinelone_hec")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("datasource".to_string(), json!("cisco_asa"));
        doc.save(&path).unwrap();
        assert!(!path.with_extension("yaml.tmp").exists());

        let reloaded = EditorDoc::load(&path).unwrap();
        assert_eq!(
            reloaded.exporters["sentinelone_hec"]["token"],
            "${S1_HEC_TOKEN}",
            "the ${{VAR}} reference must survive the round trip literally"
        );
        assert_eq!(
            reloaded.exporters["sentinelone_hec"]["datasource"],
            "cisco_asa"
        );
    }

    #[test]
    fn save_rejects_dangling_pipeline_reference_and_does_not_touch_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "original content").unwrap();

        let mut doc = EditorDoc::parse(SAMPLE).unwrap();
        doc.pipelines
            .get_mut("logs/app")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .get_mut("receivers")
            .unwrap()
            .as_array_mut()
            .unwrap()
            .push(json!("filelog/does-not-exist"));

        let err = doc.save(&path).unwrap_err();
        assert!(matches!(err, EditorError::Invalid(_)));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original content");
    }

    #[test]
    fn pipelines_referencing_finds_and_strip_removes() {
        let mut doc = EditorDoc::parse(SAMPLE).unwrap();
        let refs = doc.pipelines_referencing(ComponentCategory::Receiver, "filelog/app");
        assert_eq!(refs, vec!["logs/app".to_string()]);

        doc.strip_from_pipelines(ComponentCategory::Receiver, "filelog/app");
        let refs_after = doc.pipelines_referencing(ComponentCategory::Receiver, "filelog/app");
        assert!(refs_after.is_empty());
    }
}
