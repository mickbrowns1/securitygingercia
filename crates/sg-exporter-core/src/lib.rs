//! Shared exporter infrastructure.
//!
//! `http` provides the batching/retry/newline-framing core that both HEC
//! exporters (S1 DataPipeline + generic Splunk) build on -- only the
//! envelope shape differs between them. `stdout` is a debug-only exporter
//! used to prove out receivers/operators without real HEC credentials.

mod http;
mod stdout;

pub use http::{BatchConfig, EnvelopeBuilder, EnvelopeError, HttpHecExporter, RetryPolicy};
pub use stdout::StdoutExporter;
