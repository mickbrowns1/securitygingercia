mod duration;
mod env;
mod error;
mod schema;

pub use duration::parse_duration;
pub use env::expand_env;
pub use error::ConfigError;
pub use schema::{component_type, PipelineConfig, RawConfig, ServiceConfig};

/// Load and validate a YAML config from a string: expands `${VAR}`
/// environment references, parses into the top-level shape, and checks
/// every name referenced by a pipeline actually exists.
pub fn load_str(yaml: &str) -> Result<RawConfig, ConfigError> {
    let expanded = expand_env(yaml)?;
    let raw: RawConfig = serde_yaml_ng::from_str(&expanded)
        .map_err(|e| ConfigError::Parse(e.to_string()))?;
    raw.validate()?;
    Ok(raw)
}

pub fn load_file(path: &std::path::Path) -> Result<RawConfig, ConfigError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ConfigError::Io(path.display().to_string(), e.to_string()))?;
    load_str(&text)
}
