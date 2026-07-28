//! Configurable parsing/transform operators (regex, json, kv, severity,
//! timestamp, field-ops) chained into an `sg_core::OperatorChain`.

mod build;
mod field_op;
mod json_op;
mod kv_op;
mod regex_op;
mod severity_op;
mod timestamp_op;

pub use build::{build_chain, build_one, BuildError};
