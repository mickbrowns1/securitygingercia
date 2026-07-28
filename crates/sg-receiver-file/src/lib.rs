//! Flat-file tailing receiver: glob-based discovery, rotation-aware tail
//! (identity-keyed, not path-keyed), checkpointed offsets via
//! `sg-checkpoint`.

mod config;
mod discovery;
mod receiver;

pub use config::{FileLogConfig, StartAt};
pub use receiver::FileLogReceiver;
