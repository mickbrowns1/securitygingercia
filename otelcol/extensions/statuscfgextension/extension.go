package statuscfgextension // import "github.com/mickbrowns1/securitygingercia/otelcol/extensions/statuscfgextension"

import (
	"context"
	"encoding/json"
	"fmt"
	"math"
	"net"
	"net/http"
	"time"

	"go.opentelemetry.io/collector/component"
	"go.uber.org/zap"
)

type statusCfgExtension struct {
	cfg          *Config
	logger       *zap.Logger
	buildVersion string
	server       *http.Server
	listener     net.Listener
	startedAt    time.Time
	resolved     *resolvedConfig
	buffer       *logBuffer
	opamp        *opampReporter
}

// listenerAddr returns the actual bound address, useful in tests that ask
// for port 0 and need to know what OS-assigned port came back.
func (e *statusCfgExtension) listenerAddr() string {
	return e.listener.Addr().String()
}

func newStatusCfgExtension(cfg *Config, logger *zap.Logger, buildVersion string) *statusCfgExtension {
	return &statusCfgExtension{cfg: cfg, logger: logger, buildVersion: buildVersion, buffer: newLogBuffer()}
}

func (e *statusCfgExtension) Start(_ context.Context, _ component.Host) error {
	resolved, err := loadResolvedConfig(e.cfg.ConfigPath)
	if err != nil {
		return err
	}
	e.resolved = resolved
	e.startedAt = time.Now()

	mux := http.NewServeMux()
	mux.HandleFunc("/status", e.handleStatus)
	mux.HandleFunc("/config", e.handleConfig)
	mux.HandleFunc("/topology", e.handleTopology)
	mux.HandleFunc("/logs", e.handleLogs)
	mux.HandleFunc("/internal/logs", e.handleIngestLogs)
	mux.Handle("/", webUIHandler())
	e.server = &http.Server{Handler: mux}

	ln, err := net.Listen("tcp", e.cfg.Endpoint)
	if err != nil {
		return fmt.Errorf("statuscfg: listening on %q: %w", e.cfg.Endpoint, err)
	}
	e.listener = ln
	go func() {
		if err := e.server.Serve(ln); err != nil && err != http.ErrServerClosed {
			e.logger.Error("statuscfg server error", zap.Error(err))
		}
	}()

	opamp, err := startOpampReporter(e.cfg, e.logger, e.cfg.Endpoint, e.buildVersion, e.buildFleetReport)
	if err != nil {
		return fmt.Errorf("statuscfg: starting fleet reporter: %w", err)
	}
	e.opamp = opamp
	return nil
}

func (e *statusCfgExtension) Shutdown(ctx context.Context) error {
	e.opamp.stop()
	if e.server == nil {
		return nil
	}
	return e.server.Shutdown(ctx)
}

func (e *statusCfgExtension) handleConfig(w http.ResponseWriter, _ *http.Request) {
	w.Header().Set("Content-Type", "application/json")
	if err := json.NewEncoder(w).Encode(e.resolved.redacted); err != nil {
		e.logger.Error("statuscfg: encoding /config response", zap.Error(err))
	}
}

func (e *statusCfgExtension) handleStatus(w http.ResponseWriter, _ *http.Request) {
	snapshot, err := e.buildSnapshot()
	if err != nil {
		http.Error(w, err.Error(), http.StatusServiceUnavailable)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	if err := json.NewEncoder(w).Encode(snapshot); err != nil {
		e.logger.Error("statuscfg: encoding /status response", zap.Error(err))
	}
}

func (e *statusCfgExtension) buildSnapshot() (MetricsSnapshot, error) {
	resp, err := http.Get(e.cfg.MetricsURL)
	if err != nil {
		return MetricsSnapshot{}, fmt.Errorf("scraping %q: %w", e.cfg.MetricsURL, err)
	}
	defer resp.Body.Close()
	families := parsePrometheusText(resp.Body)

	acceptedLog := sumByLabel(families, "otelcol_receiver_accepted_log_records", "receiver")
	sentLog := sumByLabel(families, "otelcol_exporter_sent_log_records", "exporter")
	failedLog := sumByLabel(families, "otelcol_exporter_send_failed_log_records", "exporter")

	receivers := make(map[string]ReceiverSnapshot, len(e.resolved.receiverIDs))
	for _, id := range e.resolved.receiverIDs {
		receivers[id] = ReceiverSnapshot{EventsIn: acceptedLog[id]}
	}

	// batches_sent/batches_failed, retries, and last_error have no
	// dedicated Prometheus equivalent at the default telemetry level --
	// approximated here from the record-count counters that do exist.
	// This is a best-effort mapping, not an exact replay of what the
	// retired Rust exporters tracked directly.
	exporters := make(map[string]ExporterSnapshot, len(e.resolved.exporterIDs))
	for _, id := range e.resolved.exporterIDs {
		exporters[id] = ExporterSnapshot{
			EventsIn:      sentLog[id] + failedLog[id],
			BatchesSent:   sentLog[id],
			BatchesFailed: failedLog[id],
		}
	}

	pipelines := computePipelineSnapshots(e.resolved.pipelines, acceptedLog, sentLog, failedLog)

	process := ProcessSnapshot{
		CPUSeconds:     firstValue(families, "otelcol_process_cpu_seconds"),
		MemoryRSSBytes: uint64(firstValue(families, "otelcol_process_memory_rss")),
		HeapAllocBytes: uint64(firstValue(families, "otelcol_process_runtime_heap_alloc_bytes")),
	}

	return MetricsSnapshot{
		StartedAt:     e.startedAt,
		UptimeSeconds: int64(time.Since(e.startedAt).Seconds()),
		Process:       process,
		Receivers:     receivers,
		Pipelines:     pipelines,
		Exporters:     exporters,
	}, nil
}

// buildFleetReport is what actually goes out over OpAMP -- the same
// snapshot buildSnapshot already computes for the local /status endpoint,
// plus the structural topology graph the local /topology endpoint already
// serves (buildTopology, topology.go). Combining them here rather than
// sending two separate OpAMP messages keeps the fleet server's per-agent
// state a single self-contained blob (store.go's snapshot_json column
// already stores it as an opaque blob, so this needed no schema change).
func (e *statusCfgExtension) buildFleetReport() (fleetReport, error) {
	snapshot, err := e.buildSnapshot()
	if err != nil {
		return fleetReport{}, err
	}
	return fleetReport{MetricsSnapshot: snapshot, Topology: e.resolved.buildTopology()}, nil
}

// computePipelineSnapshots derives each pipeline's events_out/events_dropped
// from per-exporter Prometheus counters that are only ever labeled by
// exporter, never by (pipeline, exporter) -- OTel's default telemetry has
// no such breakdown. That's exact for an exporter used by exactly one
// pipeline, but naively summing sentLog/failedLog per exporter for every
// pipeline that references it (the original approach) massively
// overcounts once an exporter is shared: a pipeline with zero real
// traffic would still report the exporter's *entire* global total just
// because it's wired to the same shared exporter as a busy pipeline.
//
// Instead, each exporter's global total is split across the pipelines
// that use it, weighted by each pipeline's own events_in share of the
// combined events_in of every pipeline sharing that exporter (falling
// back to an even split if none of them have any events in yet). Still
// an approximation -- there's no way to get the true per-pipeline number
// from this telemetry -- but it no longer redundantly attributes a
// shared exporter's full volume to pipelines that didn't produce it.
func computePipelineSnapshots(pipelines map[string]pipelineTopology, acceptedLog, sentLog, failedLog map[string]uint64) map[string]PipelineSnapshot {
	pipelineIn := make(map[string]uint64, len(pipelines))
	for name, topo := range pipelines {
		var in uint64
		for _, r := range topo.Receivers {
			in += acceptedLog[r]
		}
		pipelineIn[name] = in
	}

	pipelineOut := make(map[string]float64, len(pipelines))
	pipelineDropped := make(map[string]float64, len(pipelines))
	exporterUsers := make(map[string][]string)
	for name, topo := range pipelines {
		for _, x := range topo.Exporters {
			exporterUsers[x] = append(exporterUsers[x], name)
		}
	}
	for exporterID, users := range exporterUsers {
		var totalIn uint64
		for _, name := range users {
			totalIn += pipelineIn[name]
		}
		for _, name := range users {
			share := 1.0 / float64(len(users))
			if totalIn > 0 {
				share = float64(pipelineIn[name]) / float64(totalIn)
			}
			pipelineOut[name] += share * float64(sentLog[exporterID])
			pipelineDropped[name] += share * float64(failedLog[exporterID])
		}
	}

	out := make(map[string]PipelineSnapshot, len(pipelines))
	for name := range pipelines {
		out[name] = PipelineSnapshot{
			EventsIn:      pipelineIn[name],
			EventsOut:     uint64(math.Round(pipelineOut[name])),
			EventsDropped: uint64(math.Round(pipelineDropped[name])),
		}
	}
	return out
}

func (e *statusCfgExtension) handleTopology(w http.ResponseWriter, _ *http.Request) {
	w.Header().Set("Content-Type", "application/json")
	if err := json.NewEncoder(w).Encode(e.resolved.buildTopology()); err != nil {
		e.logger.Error("statuscfg: encoding /topology response", zap.Error(err))
	}
}

// handleLogs dispatches GET (read the buffer) and DELETE (clear it --
// the web UI's "Clear buffer" action) on the same /logs route.
func (e *statusCfgExtension) handleLogs(w http.ResponseWriter, r *http.Request) {
	switch r.Method {
	case http.MethodGet:
		e.handleGetLogs(w, r)
	case http.MethodDelete:
		e.buffer.Clear()
		w.WriteHeader(http.StatusNoContent)
	default:
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
	}
}

// handleGetLogs serves the current contents of the in-memory log
// buffer (empty if no pipeline includes a logbuffer exporter), optionally
// filtered by ?q=<substring>, ?severity=<exact match>, and/or
// ?attr_key=<key>&attr_value=<value> (exact match against either the
// attributes or resource map -- powers the web UI's click-to-correlate).
func (e *statusCfgExtension) handleGetLogs(w http.ResponseWriter, r *http.Request) {
	q := r.URL.Query()
	entries := e.buffer.Snapshot(q.Get("q"), q.Get("severity"), q.Get("attr_key"), q.Get("attr_value"))
	w.Header().Set("Content-Type", "application/json")
	if err := json.NewEncoder(w).Encode(entries); err != nil {
		e.logger.Error("statuscfg: encoding /logs response", zap.Error(err))
	}
}

// handleIngestLogs is the receiving end of the logbuffer exporter's own
// POST -- loopback-only, like every other route here, so this is only
// reachable by something already running on the same machine.
func (e *statusCfgExtension) handleIngestLogs(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	var entries []LogEntry
	if err := json.NewDecoder(r.Body).Decode(&entries); err != nil {
		http.Error(w, fmt.Sprintf("decoding body: %v", err), http.StatusBadRequest)
		return
	}
	e.buffer.Push(entries)
	w.WriteHeader(http.StatusOK)
}
