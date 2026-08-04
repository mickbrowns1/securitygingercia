package logbufferexporter // import "github.com/mickbrowns1/securitygingercia/otelcol/exporters/logbufferexporter"

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
	"time"

	"go.opentelemetry.io/collector/exporter"
	"go.opentelemetry.io/collector/pdata/pcommon"
	"go.opentelemetry.io/collector/pdata/plog"
	"go.uber.org/zap"
)

// logEntry mirrors statuscfgextension's own decoding shape for POST
// /internal/logs -- a plain wire contract, not a shared Go type, so this
// exporter has no local cross-module dependency on the extension.
type logEntry struct {
	Timestamp  time.Time         `json:"timestamp"`
	Severity   string            `json:"severity"`
	Body       string            `json:"body"`
	Attributes map[string]string `json:"attributes,omitempty"`
	Resource   map[string]string `json:"resource,omitempty"`
}

type logBufferExporter struct {
	endpoint string
	client   *http.Client
	logger   *zap.Logger
}

func newExporter(cfg *Config, set exporter.Settings) *logBufferExporter {
	return &logBufferExporter{
		endpoint: strings.TrimSuffix(cfg.Endpoint, "/") + "/internal/logs",
		client:   &http.Client{Timeout: 5 * time.Second},
		logger:   set.TelemetrySettings.Logger,
	}
}

func (e *logBufferExporter) consumeLogs(ctx context.Context, ld plog.Logs) error {
	entries := make([]logEntry, 0, ld.LogRecordCount())
	for i := 0; i < ld.ResourceLogs().Len(); i++ {
		rl := ld.ResourceLogs().At(i)
		resourceAttrs := attrsToMap(rl.Resource().Attributes())
		for j := 0; j < rl.ScopeLogs().Len(); j++ {
			sl := rl.ScopeLogs().At(j)
			for k := 0; k < sl.LogRecords().Len(); k++ {
				lr := sl.LogRecords().At(k)
				entries = append(entries, logEntry{
					Timestamp:  lr.Timestamp().AsTime(),
					Severity:   severityLabel(lr),
					Body:       lr.Body().AsString(),
					Attributes: attrsToMap(lr.Attributes()),
					Resource:   resourceAttrs,
				})
			}
		}
	}
	if len(entries) == 0 {
		return nil
	}

	body, err := json.Marshal(entries)
	if err != nil {
		return fmt.Errorf("logbuffer: marshaling entries: %w", err)
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, e.endpoint, bytes.NewReader(body))
	if err != nil {
		return fmt.Errorf("logbuffer: building request: %w", err)
	}
	req.Header.Set("Content-Type", "application/json")
	resp, err := e.client.Do(req)
	if err != nil {
		return fmt.Errorf("logbuffer: posting to %s: %w", e.endpoint, err)
	}
	defer resp.Body.Close()
	if resp.StatusCode >= 300 {
		return fmt.Errorf("logbuffer: %s returned %s", e.endpoint, resp.Status)
	}
	return nil
}

func severityLabel(lr plog.LogRecord) string {
	if text := lr.SeverityText(); text != "" {
		return text
	}
	return lr.SeverityNumber().String()
}

func attrsToMap(m pcommon.Map) map[string]string {
	if m.Len() == 0 {
		return nil
	}
	out := make(map[string]string, m.Len())
	m.Range(func(k string, v pcommon.Value) bool {
		out[k] = v.AsString()
		return true
	})
	return out
}
