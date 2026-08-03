package statuscfgextension // import "github.com/mickbrowns1/securitygingercia/otelcol/extensions/statuscfgextension"

import (
	"fmt"
	"os"
	"strings"

	"gopkg.in/yaml.v3"
)

// pipelineTopology is which receiver/exporter component IDs feed a given
// service::pipelines entry. There is no Prometheus metric labeled by
// pipeline name, so /status derives per-pipeline totals by summing the
// receiver/exporter counters for the components each pipeline actually
// lists -- see handleStatus.
type pipelineTopology struct {
	Receivers []string
	Exporters []string
}

// resolvedConfig is this extension's own read of the same YAML file the
// collector itself was started with (there is no API to read back the
// collector's already-parsed config). Loaded once in Start(); a config
// edit made without restarting the collector process is, correctly, not
// reflected here either -- OTel Collector itself does not hot-reload.
type resolvedConfig struct {
	// redacted is the full config tree with sensitive values replaced,
	// ready to serve as-is on every /config request.
	redacted map[string]any

	receiverIDs []string
	exporterIDs []string
	pipelines   map[string]pipelineTopology
}

var sensitiveKeys = map[string]bool{
	"token":    true,
	"password": true,
	"secret":   true,
	"api_key":  true,
	"apikey":   true,
}

func loadResolvedConfig(path string) (*resolvedConfig, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("reading config_path %q: %w", path, err)
	}

	var raw map[string]any
	if err := yaml.Unmarshal(data, &raw); err != nil {
		return nil, fmt.Errorf("parsing config_path %q: %w", path, err)
	}

	rc := &resolvedConfig{
		receiverIDs: mapKeys(asMap(raw["receivers"])),
		exporterIDs: mapKeys(asMap(raw["exporters"])),
		pipelines:   extractPipelines(raw["service"]),
	}

	redactInPlace(raw)
	rc.redacted = raw
	return rc, nil
}

func extractPipelines(service any) map[string]pipelineTopology {
	out := make(map[string]pipelineTopology)
	svc := asMap(service)
	pipelines := asMap(svc["pipelines"])
	for name, def := range pipelines {
		d := asMap(def)
		out[name] = pipelineTopology{
			Receivers: asStringList(d["receivers"]),
			Exporters: asStringList(d["exporters"]),
		}
	}
	return out
}

func asMap(v any) map[string]any {
	m, _ := v.(map[string]any)
	return m
}

func asStringList(v any) []string {
	list, ok := v.([]any)
	if !ok {
		return nil
	}
	out := make([]string, 0, len(list))
	for _, item := range list {
		if s, ok := item.(string); ok {
			out = append(out, s)
		}
	}
	return out
}

func mapKeys(m map[string]any) []string {
	out := make([]string, 0, len(m))
	for k := range m {
		out = append(out, k)
	}
	return out
}

// redactInPlace mirrors the Rust status_api.rs redact() function: replace
// values by key name at any depth, regardless of whether the original YAML
// held a literal secret or an already-expanded ${VAR} reference (env
// expansion happens before this extension ever sees the file's *runtime*
// values -- but since this extension parses the file directly rather than
// asking the collector for its resolved config, ${VAR} references are
// still present here, so they're redacted as literal strings too).
func redactInPlace(v any) {
	switch val := v.(type) {
	case map[string]any:
		for k, child := range val {
			if sensitiveKeys[strings.ToLower(k)] {
				val[k] = "***redacted***"
			} else {
				redactInPlace(child)
			}
		}
	case []any:
		for _, item := range val {
			redactInPlace(item)
		}
	}
}
