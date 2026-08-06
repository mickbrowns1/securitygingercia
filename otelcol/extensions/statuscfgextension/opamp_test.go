package statuscfgextension // import "github.com/mickbrowns1/securitygingercia/otelcol/extensions/statuscfgextension"

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"go.uber.org/zap"
)

// TestFleetReport_MarshalsSnapshotAndTopologyAtTheSameLevel confirms
// embedding MetricsSnapshot in fleetReport actually flattens its fields
// to the top level alongside "topology", rather than nesting it under a
// "MetricsSnapshot" key -- the fleet webui's Sankey view expects to feed
// this same object in as both computeSankeyLayout(graph, status)
// arguments (topology as the graph, everything else as the status), so
// the shape matters, not just that the data is present somewhere.
func TestFleetReport_MarshalsSnapshotAndTopologyAtTheSameLevel(t *testing.T) {
	report := fleetReport{
		MetricsSnapshot: MetricsSnapshot{
			Receivers: map[string]ReceiverSnapshot{"syslog/udp": {EventsIn: 5}},
		},
		Topology: topologyGraph{
			Nodes: []topologyNode{{ID: "syslog/udp", Type: "receiver"}},
			Edges: []topologyEdge{{From: "syslog/udp", To: "logs/syslog"}},
		},
	}

	data, err := json.Marshal(report)
	if err != nil {
		t.Fatal(err)
	}

	var decoded map[string]any
	if err := json.Unmarshal(data, &decoded); err != nil {
		t.Fatal(err)
	}

	if _, ok := decoded["receivers"]; !ok {
		t.Fatalf("expected \"receivers\" at the top level (MetricsSnapshot should be flattened), got keys: %v", mapKeys(decoded))
	}
	if _, ok := decoded["MetricsSnapshot"]; ok {
		t.Fatal("MetricsSnapshot should be flattened, not nested under its own key")
	}
	topo, ok := decoded["topology"].(map[string]any)
	if !ok {
		t.Fatalf("expected a \"topology\" object, got keys: %v", mapKeys(decoded))
	}
	if _, ok := topo["nodes"]; !ok {
		t.Fatal("expected topology.nodes")
	}
}

// writeFakeValidateBinary writes a tiny shell script standing in for
// `sgcia-otelcol validate` -- exits 0 (silently) if shouldFail is false,
// otherwise exits 1 with a distinctive stderr message, so validateAndApply
// can be tested without a real OCB-built binary.
func writeFakeValidateBinary(t *testing.T, shouldFail bool) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), "fake-sgcia-otelcol")
	script := "#!/bin/sh\nexit 0\n"
	if shouldFail {
		script = "#!/bin/sh\necho 'boom: unknown receiver type \"nonsense\"' >&2\nexit 1\n"
	}
	if err := os.WriteFile(path, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestValidateAndApply_SuccessAppliesConfigAtomically(t *testing.T) {
	configPath := filepath.Join(t.TempDir(), "config.yaml")
	if err := os.WriteFile(configPath, []byte("old content"), 0o640); err != nil {
		t.Fatal(err)
	}
	self := writeFakeValidateBinary(t, false)

	if err := validateAndApply(configPath, self, []byte("new content")); err != nil {
		t.Fatalf("expected success, got: %v", err)
	}

	got, err := os.ReadFile(configPath)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != "new content" {
		t.Fatalf("expected the live config to be replaced, got %q", got)
	}
	if _, err := os.Stat(configPath + ".tmp"); !os.IsNotExist(err) {
		t.Fatal("expected the temp file to be gone after a successful rename")
	}
}

func TestValidateAndApply_RejectionNeverTouchesLiveFile(t *testing.T) {
	configPath := filepath.Join(t.TempDir(), "config.yaml")
	if err := os.WriteFile(configPath, []byte("old content"), 0o640); err != nil {
		t.Fatal(err)
	}
	self := writeFakeValidateBinary(t, true)

	err := validateAndApply(configPath, self, []byte("new content"))
	if err == nil {
		t.Fatal("expected validation to fail")
	}
	if !strings.Contains(err.Error(), "boom") {
		t.Fatalf("expected the fake validator's stderr in the error, got: %v", err)
	}

	got, readErr := os.ReadFile(configPath)
	if readErr != nil {
		t.Fatal(readErr)
	}
	if string(got) != "old content" {
		t.Fatalf("live config must be untouched on rejection, got %q", got)
	}
	if _, statErr := os.Stat(configPath + ".tmp"); !os.IsNotExist(statErr) {
		t.Fatal("expected the rejected candidate's temp file to be cleaned up")
	}
}

func TestValidateErrorMessage_FallbackChain(t *testing.T) {
	cases := []struct {
		name           string
		stdout, stderr string
		want           string
	}{
		{"prefers stderr", "some stdout", "the real error", "the real error"},
		{"falls back to stdout when stderr is empty", "stdout-only error", "", "stdout-only error"},
		{"falls back to a generic message when both are empty", "", "", "self validate exited with exit status 1"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := validateErrorMessage("self", tc.stdout, tc.stderr, errors.New("exit status 1"))
			if got != tc.want {
				t.Fatalf("got %q, want %q", got, tc.want)
			}
		})
	}
}

func TestLoadOrCreateInstanceUID_PersistsAcrossCalls(t *testing.T) {
	path := filepath.Join(t.TempDir(), "nested", "instance-uid")
	logger := zap.NewNop()

	first := loadOrCreateInstanceUID(path, logger)
	second := loadOrCreateInstanceUID(path, logger)

	if first != second {
		t.Fatalf("expected the same instance UID across calls (simulating a restart), got %x then %x", first, second)
	}
}

func TestLoadOrCreateInstanceUID_EmptyPathIsNotPersistent(t *testing.T) {
	logger := zap.NewNop()

	first := loadOrCreateInstanceUID("", logger)
	second := loadOrCreateInstanceUID("", logger)

	if first == second {
		t.Fatalf("expected different instance UIDs when persistence is disabled (empty path), got the same value twice: %x", first)
	}
}

func TestLoadOrCreateInstanceUID_CorruptFileRegeneratesRatherThanFails(t *testing.T) {
	path := filepath.Join(t.TempDir(), "instance-uid")
	if err := os.WriteFile(path, []byte("not-valid-hex"), 0o600); err != nil {
		t.Fatal(err)
	}

	got := loadOrCreateInstanceUID(path, zap.NewNop())
	var zero [16]byte
	if got == zero {
		t.Fatal("expected a real generated UID, not the zero value")
	}
}

func TestDecodeInstanceUID_RejectsWrongLength(t *testing.T) {
	if _, err := decodeInstanceUID("abcd"); err == nil {
		t.Fatal("expected an error for a too-short hex string")
	}
	if _, err := decodeInstanceUID("not hex at all!!"); err == nil {
		t.Fatal("expected an error for non-hex content")
	}
}
