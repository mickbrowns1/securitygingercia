use crate::editor::schema_registry::ComponentCategory;
use serde_json::{Map, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

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
/// feature) rather than any strongly-typed config struct, so that:
/// - unknown/not-yet-registered fields survive a load -> edit -> save
///   round trip untouched (only fields the user actually edits change),
/// - `${VAR}` tokens inside string values are preserved byte-for-byte,
///   since they're never parsed as anything but opaque strings here.
///
/// This mirrors real OTel Collector config shape: `receivers`/
/// `exporters`/`extensions` are id-keyed maps; each receiver owns its own
/// inline `operators:` list rather than referencing a shared top-level
/// section. `service.extensions` is derived automatically (every defined
/// extension is active -- there's no supported "defined but unused"
/// case), but `service.pipelines`' receiver/exporter lists are real,
/// user-managed per-pipeline choices.
#[derive(Debug, Clone, Default)]
pub struct EditorDoc {
    pub receivers: Map<String, Value>,
    pub exporters: Map<String, Value>,
    pub extensions: Map<String, Value>,
    /// Each value is an object `{receivers: [...], exporters: [...]}`.
    pub pipelines: Map<String, Value>,
}

impl EditorDoc {
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads a config file, skipping env expansion on purpose so `${VAR}`
    /// tokens stay literal in memory. A missing file loads as an empty
    /// (fresh) document rather than an error, so `sgcia edit
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
            exporters: get_map("exporters"),
            extensions: get_map("extensions"),
            pipelines,
        })
    }

    pub fn to_value(&self) -> Value {
        let mut extension_ids: Vec<&String> = self.extensions.keys().collect();
        extension_ids.sort();
        serde_json::json!({
            "receivers": self.receivers,
            "exporters": self.exporters,
            "extensions": self.extensions,
            "service": {
                "extensions": extension_ids,
                "pipelines": self.pipelines,
            },
        })
    }

    /// Runs the real `sgcia-otelcol validate` binary against this
    /// document -- there is no Rust-side validator anymore now that the
    /// collector engine itself is the OCB-built Go binary, so that binary
    /// is the only source of truth for validity. `${VAR}`-looking strings
    /// are replaced by a harmless placeholder first, so validation
    /// doesn't require the real secret to be present on the editing
    /// machine (the placeholder is never written to the real config
    /// file -- only used in this temp copy).
    fn validate_with_binary(&self, bin: &Path) -> Result<(), EditorError> {
        let value = placeholder_for_validation(&self.to_value());
        let yaml_text = serde_yaml_ng::to_string(&value)
            .map_err(|e| EditorError::Serialize(e.to_string()))?;

        let mut tmp = tempfile::Builder::new()
            .suffix(".yaml")
            .tempfile()
            .map_err(|e| EditorError::Io("temp file".to_string(), e.to_string()))?;
        tmp.write_all(yaml_text.as_bytes())
            .map_err(|e| EditorError::Io(tmp.path().display().to_string(), e.to_string()))?;
        tmp.flush()
            .map_err(|e| EditorError::Io(tmp.path().display().to_string(), e.to_string()))?;

        let output = std::process::Command::new(bin)
            .arg("validate")
            .arg("--config")
            .arg(format!("file:{}", tmp.path().display()))
            .output()
            .map_err(|e| {
                EditorError::Invalid(format!(
                    "couldn't run '{} validate' (set SGCIA_OTELCOL_BIN if it's not on PATH): {e}",
                    bin.display()
                ))
            })?;

        if !output.status.success() {
            let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(EditorError::Invalid(if message.is_empty() {
                format!("{} validate exited with {}", bin.display(), output.status)
            } else {
                message
            }));
        }
        Ok(())
    }

    /// Validates, then atomically writes (temp file in the same
    /// directory + `fsync` + `rename`) -- never overwrites the good file
    /// on disk with an invalid one.
    pub fn save(&self, path: &Path) -> Result<(), EditorError> {
        self.save_with_binary(path, &otelcol_binary_path())
    }

    fn save_with_binary(&self, path: &Path, bin: &Path) -> Result<(), EditorError> {
        self.validate_with_binary(bin)?;
        let yaml_text = serde_yaml_ng::to_string(&self.to_value())
            .map_err(|e| EditorError::Serialize(e.to_string()))?;
        atomic_write(path, &yaml_text)
            .map_err(|e| EditorError::Io(path.display().to_string(), e.to_string()))?;
        Ok(())
    }

    /// Names of pipelines that still reference `id` in the given
    /// category's list. Extensions are never listed per-pipeline (every
    /// defined extension is active collector-wide -- see `to_value`), so
    /// this always returns empty for `ComponentCategory::Extension`; a
    /// receiver's `storage:` reference to an extension id is checked by
    /// the real validator at save time instead.
    pub fn pipelines_referencing(&self, category: ComponentCategory, id: &str) -> Vec<String> {
        let Some(key) = category_key(category) else {
            return Vec::new();
        };
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
        let Some(key) = category_key(category) else {
            return;
        };
        for def in self.pipelines.values_mut() {
            if let Some(arr) = def.get_mut(key).and_then(|v| v.as_array_mut()) {
                arr.retain(|v| v.as_str() != Some(id));
            }
        }
    }
}

/// `None` for `Extension`: extensions are never listed inside a
/// pipeline's own `receivers`/`exporters` arrays.
fn category_key(category: ComponentCategory) -> Option<&'static str> {
    match category {
        ComponentCategory::Receiver => Some("receivers"),
        ComponentCategory::Exporter => Some("exporters"),
        ComponentCategory::Extension => None,
    }
}

fn otelcol_binary_path() -> PathBuf {
    if let Ok(p) = std::env::var("SGCIA_OTELCOL_BIN") {
        return PathBuf::from(p);
    }
    // Dev convenience: repo-relative path when `sgcia edit` is run from
    // the workspace root without the collector installed alongside it.
    let dev_path = PathBuf::from("otelcol/dist/sgcia-otelcol");
    if dev_path.exists() {
        return dev_path;
    }
    PathBuf::from("sgcia-otelcol")
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
  file_log/app:
    include: ["/var/log/app/*.log"]
exporters:
  splunk_hec/sentinelone:
    endpoint: "https://xdr.us1.sentinelone.net/services/collector/event"
    token: ${S1_HEC_TOKEN}
extensions:
  file_storage:
    directory: /var/lib/sgcia/otelcol-storage
    create_directory: true
service:
  extensions: [file_storage]
  pipelines:
    logs/app:
      receivers: [file_log/app]
      exporters: [splunk_hec/sentinelone]
"#;

    /// A stub `sgcia-otelcol` standing in for the real Go binary in tests
    /// that don't need to exercise it for real (only that `EditorDoc`
    /// invokes *some* binary correctly and handles its exit status).
    /// Exercising the real binary is covered by the integration test
    /// below, gated on the binary actually being built.
    #[cfg(unix)]
    fn stub_binary(exit_success: bool, stderr: &str) -> tempfile::TempPath {
        use std::os::unix::fs::PermissionsExt;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let exit_code = if exit_success { 0 } else { 1 };
        writeln!(file, "#!/bin/sh\n>&2 echo '{stderr}'\nexit {exit_code}").unwrap();
        file.flush().unwrap();
        let path = file.into_temp_path();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

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
            doc.exporters["splunk_hec/sentinelone"]["token"],
            "${S1_HEC_TOKEN}"
        );
    }

    #[test]
    fn to_value_derives_service_extensions_from_defined_extensions() {
        let doc = EditorDoc::parse(SAMPLE).unwrap();
        let value = doc.to_value();
        assert_eq!(value["service"]["extensions"], json!(["file_storage"]));
    }

    #[cfg(unix)]
    #[test]
    fn validate_succeeds_when_binary_exits_zero() {
        let doc = EditorDoc::parse(SAMPLE).unwrap();
        let bin = stub_binary(true, "");
        doc.validate_with_binary(&bin).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn validate_surfaces_binary_stderr_on_failure() {
        let doc = EditorDoc::parse(SAMPLE).unwrap();
        let bin = stub_binary(false, "boom: bad config");
        let err = doc.validate_with_binary(&bin).unwrap_err();
        assert!(matches!(err, EditorError::Invalid(msg) if msg.contains("boom: bad config")));
    }

    #[cfg(unix)]
    #[test]
    fn save_rejects_when_binary_reports_failure_and_does_not_touch_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "original content").unwrap();

        let doc = EditorDoc::parse(SAMPLE).unwrap();
        let bin = stub_binary(false, "nope");

        let err = doc.save_with_binary(&path, &bin).unwrap_err();
        assert!(matches!(err, EditorError::Invalid(_)));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original content");
    }

    #[cfg(unix)]
    #[test]
    fn save_with_binary_writes_file_when_binary_accepts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/config.yaml");
        let doc = EditorDoc::parse(SAMPLE).unwrap();
        let bin = stub_binary(true, "");

        doc.save_with_binary(&path, &bin).unwrap();
        assert!(!path.with_extension("yaml.tmp").exists());
        let reloaded = EditorDoc::load(&path).unwrap();
        assert!(reloaded.receivers.contains_key("file_log/app"));
    }

    #[test]
    fn save_and_reload_round_trips_including_var_reference() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/config.yaml");
        let mut doc = EditorDoc::parse(SAMPLE).unwrap();
        doc.exporters
            .get_mut("splunk_hec/sentinelone")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("index".to_string(), json!("main"));

        // Write the file directly via the same atomic_write path save()
        // uses, bypassing validate() so this test doesn't depend on the
        // real otelcol binary being built.
        let yaml_text = serde_yaml_ng::to_string(&doc.to_value()).unwrap();
        atomic_write(&path, &yaml_text).unwrap();
        assert!(!path.with_extension("yaml.tmp").exists());

        let reloaded = EditorDoc::load(&path).unwrap();
        assert_eq!(
            reloaded.exporters["splunk_hec/sentinelone"]["token"],
            "${S1_HEC_TOKEN}",
            "the ${{VAR}} reference must survive the round trip literally"
        );
        assert_eq!(
            reloaded.exporters["splunk_hec/sentinelone"]["index"],
            "main"
        );
    }

    #[test]
    fn pipelines_referencing_finds_and_strip_removes() {
        let mut doc = EditorDoc::parse(SAMPLE).unwrap();
        let refs = doc.pipelines_referencing(ComponentCategory::Receiver, "file_log/app");
        assert_eq!(refs, vec!["logs/app".to_string()]);

        doc.strip_from_pipelines(ComponentCategory::Receiver, "file_log/app");
        let refs_after = doc.pipelines_referencing(ComponentCategory::Receiver, "file_log/app");
        assert!(refs_after.is_empty());
    }

    #[test]
    fn pipelines_referencing_is_always_empty_for_extensions() {
        let doc = EditorDoc::parse(SAMPLE).unwrap();
        assert!(doc
            .pipelines_referencing(ComponentCategory::Extension, "file_storage")
            .is_empty());
    }

    /// `cargo test` runs test binaries with cwd set to this crate's own
    /// manifest directory (crates/collector), not the workspace root, so
    /// production `otelcol_binary_path()`'s plain relative
    /// "otelcol/dist/..." check (correct for *real* runtime use, where
    /// baking in a build-machine absolute path would be wrong) never
    /// resolves under `cargo test` -- anchor to CARGO_MANIFEST_DIR
    /// (valid here since tests only ever run from the same checkout that
    /// built them) instead, just for this test.
    fn test_otelcol_binary_path() -> PathBuf {
        if let Ok(p) = std::env::var("SGCIA_OTELCOL_BIN") {
            return PathBuf::from(p);
        }
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../otelcol/dist/sgcia-otelcol")
    }

    /// Real integration coverage for `validate`/`save`, gated on the OCB-
    /// built binary actually existing (it's a separate build step, not
    /// part of `cargo test`) so this degrades to a skip rather than a
    /// failure on a workspace that hasn't run `ocb --config
    /// builder-config.yaml` yet.
    #[test]
    fn validate_accepts_a_well_formed_document_against_the_real_binary() {
        let bin = test_otelcol_binary_path();
        if !bin.exists() {
            eprintln!("skipping: {} not built (see otelcol/README)", bin.display());
            return;
        }
        let doc = EditorDoc::parse(SAMPLE).unwrap();
        doc.validate_with_binary(&bin).unwrap();
    }
}
