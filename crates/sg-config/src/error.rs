#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file {0}: {1}")]
    Io(String, String),

    #[error("failed to parse config: {0}")]
    Parse(String),

    #[error("undefined environment variable ${{{0}}} referenced in config")]
    UndefinedEnvVar(String),

    #[error("pipeline '{pipeline}' references unknown {kind} '{name}'")]
    UnknownComponent {
        pipeline: String,
        kind: &'static str,
        name: String,
    },

    #[error("pipeline '{0}' has no receivers")]
    EmptyPipelineReceivers(String),

    #[error("pipeline '{0}' has no exporters")]
    EmptyPipelineExporters(String),
}
