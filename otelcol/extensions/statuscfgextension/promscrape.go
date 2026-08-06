package statuscfgextension // import "github.com/mickbrowns1/securitygingercia/otelcol/extensions/statuscfgextension"

import (
	"bufio"
	"io"
	"regexp"
	"strconv"
	"strings"
)

// promSample is one label-set + value pair for a Prometheus metric family,
// e.g. otelcol_receiver_accepted_log_records{receiver="syslog/tcp"} 42.
type promSample struct {
	Labels map[string]string
	Value  float64
}

var (
	lineRE  = regexp.MustCompile(`^([a-zA-Z_:][a-zA-Z0-9_:]*)(\{[^}]*\})?\s+(\S+)$`)
	labelRE = regexp.MustCompile(`([a-zA-Z_][a-zA-Z0-9_]*)="((?:[^"\\]|\\.)*)"`)
)

// parsePrometheusText is a minimal reader for the text exposition format
// otelcol's own promhttp handler emits -- just enough to pull out the
// specific counters this extension cares about (receiver/exporter names +
// values), not a general-purpose Prometheus client.
func parsePrometheusText(r io.Reader) map[string][]promSample {
	families := make(map[string][]promSample)
	scanner := bufio.NewScanner(r)
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		m := lineRE.FindStringSubmatch(line)
		if m == nil {
			continue
		}
		name, labelsRaw, valueStr := m[1], m[2], m[3]
		value, err := strconv.ParseFloat(valueStr, 64)
		if err != nil {
			continue
		}
		labels := make(map[string]string)
		for _, lm := range labelRE.FindAllStringSubmatch(labelsRaw, -1) {
			labels[lm[1]] = lm[2]
		}
		families[name] = append(families[name], promSample{Labels: labels, Value: value})
	}
	return families
}

// sumByLabel adds up every sample's value for the given metric family,
// keyed by one label (typically "receiver" or "exporter"). Samples with an
// extra dimension (e.g. receiver metrics also carry "transport") collapse
// into the same key here, which is what we want -- one counter per
// component ID, matching sg_core's keying.
func sumByLabel(families map[string][]promSample, metric, label string) map[string]uint64 {
	out := make(map[string]uint64)
	for _, s := range families[metric] {
		key := s.Labels[label]
		if key == "" {
			continue
		}
		out[key] += uint64(s.Value)
	}
	return out
}

// firstValue returns a metric family's single value -- for families with
// no meaningful label dimension (e.g. otelcol_process_* gauges/counters,
// which report exactly one process-wide value per scrape), unlike
// sumByLabel above which is for per-component counters keyed by label. 0
// if the family wasn't present in this scrape.
func firstValue(families map[string][]promSample, metric string) float64 {
	if samples := families[metric]; len(samples) > 0 {
		return samples[0].Value
	}
	return 0
}
