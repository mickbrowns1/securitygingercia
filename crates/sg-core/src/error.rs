#[derive(Debug, thiserror::Error)]
pub enum SgError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("config error: {0}")]
    Config(String),

    #[error("channel closed")]
    ChannelClosed,

    #[error("{0}")]
    Other(String),
}
