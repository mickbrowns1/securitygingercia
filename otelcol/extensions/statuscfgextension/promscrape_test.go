package statuscfgextension

import (
	"strings"
	"testing"
)

const samplePromText = `# HELP otelcol_receiver_accepted_log_records Number of log records successfully pushed into the pipeline.
# TYPE otelcol_receiver_accepted_log_records counter
otelcol_receiver_accepted_log_records{receiver="syslog/tcp",transport=""} 42
otelcol_receiver_accepted_log_records{receiver="filelog/app",transport=""} 7
# HELP otelcol_exporter_sent_log_records Number of log record successfully sent to destination.
# TYPE otelcol_exporter_sent_log_records counter
otelcol_exporter_sent_log_records{exporter="splunk_hec/sentinelone"} 40
# HELP otelcol_exporter_send_failed_log_records Number of log records in failed attempts to send to destination.
# TYPE otelcol_exporter_send_failed_log_records counter
otelcol_exporter_send_failed_log_records{exporter="splunk_hec/sentinelone"} 2
# HELP otelcol_process_cpu_seconds Total CPU user and system time in seconds.
# TYPE otelcol_process_cpu_seconds counter
otelcol_process_cpu_seconds 2.75
# HELP otelcol_process_memory_rss Total physical memory (resident set size).
# TYPE otelcol_process_memory_rss gauge
otelcol_process_memory_rss 2.922496e+07
# HELP otelcol_process_runtime_heap_alloc_bytes Bytes of allocated heap objects (see 'go doc runtime.MemStats.HeapAlloc').
# TYPE otelcol_process_runtime_heap_alloc_bytes gauge
otelcol_process_runtime_heap_alloc_bytes 1.1116088e+07
`

func TestParsePrometheusText_ExtractsLabeledCounters(t *testing.T) {
	families := parsePrometheusText(strings.NewReader(samplePromText))

	accepted := sumByLabel(families, "otelcol_receiver_accepted_log_records", "receiver")
	if accepted["syslog/tcp"] != 42 {
		t.Errorf("syslog/tcp accepted = %d, want 42", accepted["syslog/tcp"])
	}
	if accepted["filelog/app"] != 7 {
		t.Errorf("filelog/app accepted = %d, want 7", accepted["filelog/app"])
	}

	sent := sumByLabel(families, "otelcol_exporter_sent_log_records", "exporter")
	if sent["splunk_hec/sentinelone"] != 40 {
		t.Errorf("sent = %d, want 40", sent["splunk_hec/sentinelone"])
	}

	failed := sumByLabel(families, "otelcol_exporter_send_failed_log_records", "exporter")
	if failed["splunk_hec/sentinelone"] != 2 {
		t.Errorf("failed = %d, want 2", failed["splunk_hec/sentinelone"])
	}
}

func TestParsePrometheusText_IgnoresCommentsAndBlankLines(t *testing.T) {
	families := parsePrometheusText(strings.NewReader("# just a comment\n\n"))
	if len(families) != 0 {
		t.Errorf("families = %v, want empty", families)
	}
}

func TestSumByLabel_MissingMetricReturnsEmptyMap(t *testing.T) {
	families := parsePrometheusText(strings.NewReader(samplePromText))
	result := sumByLabel(families, "does_not_exist", "receiver")
	if len(result) != 0 {
		t.Errorf("result = %v, want empty", result)
	}
}

func TestFirstValue_ExtractsUnlabeledGauge(t *testing.T) {
	families := parsePrometheusText(strings.NewReader(samplePromText))
	if got := firstValue(families, "otelcol_process_cpu_seconds"); got != 2.75 {
		t.Errorf("otelcol_process_cpu_seconds = %v, want 2.75", got)
	}
	if got := firstValue(families, "otelcol_process_memory_rss"); got != 2.922496e+07 {
		t.Errorf("otelcol_process_memory_rss = %v, want 2.922496e+07", got)
	}
}

func TestFirstValue_MissingMetricReturnsZero(t *testing.T) {
	families := parsePrometheusText(strings.NewReader(samplePromText))
	if got := firstValue(families, "does_not_exist"); got != 0 {
		t.Errorf("firstValue for missing metric = %v, want 0", got)
	}
}
