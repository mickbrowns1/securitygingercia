package statuscfgextension // import "github.com/mickbrowns1/securitygingercia/otelcol/extensions/statuscfgextension"

import (
	"context"
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"time"

	"go.opentelemetry.io/collector/component"
	"go.uber.org/zap"
)

type statusCfgExtension struct {
	cfg       *Config
	logger    *zap.Logger
	server    *http.Server
	listener  net.Listener
	startedAt time.Time
	resolved  *resolvedConfig
	buffer    *logBuffer
}

// listenerAddr returns the actual bound address, useful in tests that ask
// for port 0 and need to know what OS-assigned port came back.
func (e *statusCfgExtension) listenerAddr() string {
	return e.listener.Addr().String()
}

func newStatusCfgExtension(cfg *Config, logger *zap.Logger) *statusCfgExtension {
	return &statusCfgExtension{cfg: cfg, logger: logger, buffer: newLogBuffer()}
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
	return nil
}

func (e *statusCfgExtension) Shutdown(ctx context.Context) error {
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

	pipelines := make(map[string]PipelineSnapshot, len(e.resolved.pipelines))
	for name, topo := range e.resolved.pipelines {
		var in, out, dropped uint64
		for _, r := range topo.Receivers {
			in += acceptedLog[r]
		}
		for _, x := range topo.Exporters {
			out += sentLog[x]
			dropped += failedLog[x]
		}
		pipelines[name] = PipelineSnapshot{EventsIn: in, EventsOut: out, EventsDropped: dropped}
	}

	return MetricsSnapshot{
		StartedAt:     e.startedAt,
		UptimeSeconds: int64(time.Since(e.startedAt).Seconds()),
		Receivers:     receivers,
		Pipelines:     pipelines,
		Exporters:     exporters,
	}, nil
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
