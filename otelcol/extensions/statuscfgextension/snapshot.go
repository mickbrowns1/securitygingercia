package statuscfgextension // import "github.com/mickbrowns1/securitygingercia/otelcol/extensions/statuscfgextension"

import "time"

// The types below mirror sg_core::metrics::MetricsSnapshot field-for-field
// (crates/sg-core/src/metrics.rs) so the Rust dashboard's existing
// serde-based deserialization keeps working unmodified against this Go
// extension's /status response.

type ReceiverSnapshot struct {
	EventsIn uint64 `json:"events_in"`
}

type PipelineSnapshot struct {
	EventsIn           uint64 `json:"events_in"`
	EventsOut          uint64 `json:"events_out"`
	EventsDropped      uint64 `json:"events_dropped"`
	EventsDeadLettered uint64 `json:"events_dead_lettered"`
	ParseErrors        uint64 `json:"parse_errors"`
}

type LastError struct {
	Message string    `json:"message"`
	At      time.Time `json:"at"`
}

type ExporterSnapshot struct {
	EventsIn      uint64     `json:"events_in"`
	BatchesSent   uint64     `json:"batches_sent"`
	BatchesFailed uint64     `json:"batches_failed"`
	Retries       uint64     `json:"retries"`
	LastError     *LastError `json:"last_error"`
}

// ProcessSnapshot is this agent's own resource usage, straight from the
// otelcol_process_* family the collector's own default self-telemetry
// already exposes at MetricsURL (:8888/metrics) -- confirmed live via
// otelcol_process_cpu_seconds/_memory_rss/_runtime_heap_alloc_bytes, none
// of which carry a receiver/exporter label (one process-wide value each,
// unlike the labeled counters above).
type ProcessSnapshot struct {
	CPUSeconds     float64 `json:"cpu_seconds"`
	MemoryRSSBytes uint64  `json:"memory_rss_bytes"`
	HeapAllocBytes uint64  `json:"heap_alloc_bytes"`
}

type MetricsSnapshot struct {
	StartedAt     time.Time                   `json:"started_at"`
	UptimeSeconds int64                       `json:"uptime_seconds"`
	Process       ProcessSnapshot             `json:"process"`
	Receivers     map[string]ReceiverSnapshot `json:"receivers"`
	Pipelines     map[string]PipelineSnapshot `json:"pipelines"`
	Exporters     map[string]ExporterSnapshot `json:"exporters"`
}
