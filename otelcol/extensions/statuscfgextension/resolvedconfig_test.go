package statuscfgextension

import (
	"os"
	"path/filepath"
	"testing"
)

const sampleYAML = `
receivers:
  syslog/udp:
    protocol: rfc3164
exporters:
  splunk_hec/sentinelone:
    endpoint: "https://example.invalid/services/collector/event"
    token: "super-secret-token"
service:
  pipelines:
    logs/syslog:
      receivers: [syslog/udp]
      exporters: [splunk_hec/sentinelone]
`

func writeTempConfig(t *testing.T, contents string) string {
	t.Helper()
	dir := t.TempDir()
	path := filepath.Join(dir, "config.yaml")
	if err := os.WriteFile(path, []byte(contents), 0o600); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestLoadResolvedConfig_RedactsSensitiveKeysAtAnyDepth(t *testing.T) {
	path := writeTempConfig(t, sampleYAML)
	rc, err := loadResolvedConfig(path)
	if err != nil {
		t.Fatal(err)
	}

	exporters := asMap(rc.redacted["exporters"])
	s1 := asMap(exporters["splunk_hec/sentinelone"])
	if s1["token"] != "***redacted***" {
		t.Errorf("token = %v, want ***redacted***", s1["token"])
	}
	if s1["endpoint"] != "https://example.invalid/services/collector/event" {
		t.Errorf("endpoint was unexpectedly modified: %v", s1["endpoint"])
	}
}

func TestLoadResolvedConfig_ExtractsPipelineTopology(t *testing.T) {
	path := writeTempConfig(t, sampleYAML)
	rc, err := loadResolvedConfig(path)
	if err != nil {
		t.Fatal(err)
	}

	topo, ok := rc.pipelines["logs/syslog"]
	if !ok {
		t.Fatalf("pipeline logs/syslog not found in %v", rc.pipelines)
	}
	if len(topo.Receivers) != 1 || topo.Receivers[0] != "syslog/udp" {
		t.Errorf("receivers = %v, want [syslog/udp]", topo.Receivers)
	}
	if len(topo.Exporters) != 1 || topo.Exporters[0] != "splunk_hec/sentinelone" {
		t.Errorf("exporters = %v, want [splunk_hec/sentinelone]", topo.Exporters)
	}
}

func TestLoadResolvedConfig_CollectsComponentIDs(t *testing.T) {
	path := writeTempConfig(t, sampleYAML)
	rc, err := loadResolvedConfig(path)
	if err != nil {
		t.Fatal(err)
	}
	if len(rc.receiverIDs) != 1 || rc.receiverIDs[0] != "syslog/udp" {
		t.Errorf("receiverIDs = %v, want [syslog/udp]", rc.receiverIDs)
	}
	if len(rc.exporterIDs) != 1 || rc.exporterIDs[0] != "splunk_hec/sentinelone" {
		t.Errorf("exporterIDs = %v, want [splunk_hec/sentinelone]", rc.exporterIDs)
	}
}

func TestRedactInPlace_MatchesKeyCaseInsensitivelyAtAnyDepth(t *testing.T) {
	v := map[string]any{
		"outer": map[string]any{
			"TOKEN": "secret-value",
			"nested": []any{
				map[string]any{"password": "hunter2"},
				map[string]any{"keep": "me"},
			},
		},
	}
	redactInPlace(v)

	outer := asMap(v["outer"])
	if outer["TOKEN"] != "***redacted***" {
		t.Errorf("TOKEN = %v, want ***redacted***", outer["TOKEN"])
	}
	nested := outer["nested"].([]any)
	if asMap(nested[0])["password"] != "***redacted***" {
		t.Errorf("password = %v, want ***redacted***", asMap(nested[0])["password"])
	}
	if asMap(nested[1])["keep"] != "me" {
		t.Errorf("keep = %v, want me (untouched)", asMap(nested[1])["keep"])
	}
}
