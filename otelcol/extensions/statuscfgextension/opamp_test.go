package statuscfgextension // import "github.com/mickbrowns1/securitygingercia/otelcol/extensions/statuscfgextension"

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/open-telemetry/opamp-go/client/types"
	"github.com/open-telemetry/opamp-go/protobufs"
	"go.uber.org/zap"
)

// Compile-time check that simplePackagesStateProvider actually satisfies
// the interface the OpAMP SDK requires for AcceptsPackages/
// ReportsPackageStatuses to work at all -- see startOpampReporter's
// SetCapabilities comment for why this is required, not optional.
var _ types.PackagesStateProvider = (*simplePackagesStateProvider)(nil)

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

// newTestPackagesStateProvider builds a simplePackagesStateProvider
// pointed at a fake "self" binary path under t.TempDir(), bypassing
// os.Executable() -- the constructor newSimplePackagesStateProvider
// always resolves the real running test binary, which isn't what these
// tests want to exercise being overwritten.
func newTestPackagesStateProvider(t *testing.T, selfContent string) *simplePackagesStateProvider {
	t.Helper()
	selfPath := filepath.Join(t.TempDir(), "self-binary")
	if err := os.WriteFile(selfPath, []byte(selfContent), 0o755); err != nil {
		t.Fatal(err)
	}
	return &simplePackagesStateProvider{
		packages:        make(map[string]types.PackageState),
		fileContentHash: make(map[string][]byte),
		selfPath:        selfPath,
		logger:          zap.NewNop(),
	}
}

func TestUpdateContent_SuccessAppliesBinaryAtomically(t *testing.T) {
	provider := newTestPackagesStateProvider(t, "old binary content")

	newContent := []byte("#!/bin/sh\nexit 0\n")
	hash := sha256.Sum256(newContent)

	if err := provider.UpdateContent(context.Background(), "sgcia-otelcol", bytes.NewReader(newContent), hash[:], nil); err != nil {
		t.Fatalf("expected success, got: %v", err)
	}

	got, err := os.ReadFile(provider.selfPath)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != string(newContent) {
		t.Fatalf("expected the live binary to be replaced, got %q", got)
	}
	if _, err := os.Stat(provider.selfPath + ".tmp"); !os.IsNotExist(err) {
		t.Fatal("expected the temp file to be gone after a successful rename")
	}
	if gotHash, _ := provider.FileContentHash("sgcia-otelcol"); !bytes.Equal(gotHash, hash[:]) {
		t.Fatalf("expected FileContentHash to be recorded after a successful update, got %x", gotHash)
	}
}

func TestUpdateContent_HashMismatchNeverTouchesLiveBinary(t *testing.T) {
	provider := newTestPackagesStateProvider(t, "old binary content")

	newContent := []byte("#!/bin/sh\nexit 0\n")
	wrongHash := sha256.Sum256([]byte("something else entirely"))

	err := provider.UpdateContent(context.Background(), "sgcia-otelcol", bytes.NewReader(newContent), wrongHash[:], nil)
	if err == nil {
		t.Fatal("expected a hash mismatch error")
	}
	if !strings.Contains(err.Error(), "does not match") {
		t.Fatalf("expected a hash-mismatch message, got: %v", err)
	}

	got, readErr := os.ReadFile(provider.selfPath)
	if readErr != nil {
		t.Fatal(readErr)
	}
	if string(got) != "old binary content" {
		t.Fatalf("live binary must be untouched on hash mismatch, got %q", got)
	}
	if _, statErr := os.Stat(provider.selfPath + ".tmp"); !os.IsNotExist(statErr) {
		t.Fatal("expected the rejected candidate's temp file to be cleaned up")
	}
}

func TestUpdateContent_VersionCheckFailureNeverTouchesLiveBinary(t *testing.T) {
	provider := newTestPackagesStateProvider(t, "old binary content")

	newContent := []byte("#!/bin/sh\necho 'boom: this is not a real collector binary' >&2\nexit 1\n")
	hash := sha256.Sum256(newContent)

	err := provider.UpdateContent(context.Background(), "sgcia-otelcol", bytes.NewReader(newContent), hash[:], nil)
	if err == nil {
		t.Fatal("expected the --version check to fail")
	}
	if !strings.Contains(err.Error(), "boom") {
		t.Fatalf("expected the fake binary's stderr in the error, got: %v", err)
	}

	got, readErr := os.ReadFile(provider.selfPath)
	if readErr != nil {
		t.Fatal(readErr)
	}
	if string(got) != "old binary content" {
		t.Fatalf("live binary must be untouched when the candidate fails to run, got %q", got)
	}
	if _, statErr := os.Stat(provider.selfPath + ".tmp"); !os.IsNotExist(statErr) {
		t.Fatal("expected the rejected candidate's temp file to be cleaned up")
	}
}

// fakeOpAMPClient is a minimal test double for client.OpAMPClient -- only
// UpdateEffectiveConfig has real behavior (recording calls / returning a
// configurable error); every other method is a no-op stub, since
// checkEffectiveConfigDrift only ever calls that one.
type fakeOpAMPClient struct {
	updateEffectiveConfigCalls int
	updateEffectiveConfigErr   error
}

func (f *fakeOpAMPClient) Start(context.Context, types.StartSettings) error          { return nil }
func (f *fakeOpAMPClient) Stop(context.Context) error                                { return nil }
func (f *fakeOpAMPClient) SetAgentDescription(*protobufs.AgentDescription) error     { return nil }
func (f *fakeOpAMPClient) AgentDescription() *protobufs.AgentDescription             { return nil }
func (f *fakeOpAMPClient) SetHealth(*protobufs.ComponentHealth) error                { return nil }
func (f *fakeOpAMPClient) SetRemoteConfigStatus(*protobufs.RemoteConfigStatus) error { return nil }
func (f *fakeOpAMPClient) SetPackageStatuses(*protobufs.PackageStatuses) error       { return nil }
func (f *fakeOpAMPClient) SetCustomCapabilities(*protobufs.CustomCapabilities) error { return nil }
func (f *fakeOpAMPClient) SetFlags(protobufs.AgentToServerFlags)                     {}
func (f *fakeOpAMPClient) SetAvailableComponents(*protobufs.AvailableComponents) error {
	return nil
}
func (f *fakeOpAMPClient) SetCapabilities(*protobufs.AgentCapabilities) error { return nil }
func (f *fakeOpAMPClient) RequestConnectionSettings(*protobufs.ConnectionSettingsRequest) error {
	return nil
}
func (f *fakeOpAMPClient) SendCustomMessage(*protobufs.CustomMessage) (chan struct{}, error) {
	return nil, nil
}
func (f *fakeOpAMPClient) UpdateEffectiveConfig(context.Context) error {
	f.updateEffectiveConfigCalls++
	return f.updateEffectiveConfigErr
}

func TestBuildEffectiveConfig_WrapsFileContent(t *testing.T) {
	configPath := filepath.Join(t.TempDir(), "config.yaml")
	if err := os.WriteFile(configPath, []byte("receivers: {}"), 0o640); err != nil {
		t.Fatal(err)
	}

	ec, err := buildEffectiveConfig(configPath)
	if err != nil {
		t.Fatal(err)
	}
	file := ec.GetConfigMap().GetConfigMap()[""]
	if file == nil {
		t.Fatal("expected a config file entry keyed by an empty string")
	}
	if string(file.GetBody()) != "receivers: {}" {
		t.Fatalf("body = %q, want the file's exact content", file.GetBody())
	}
}

func TestConfigFileHash_ChangesWithContentAndIsDeterministic(t *testing.T) {
	configPath := filepath.Join(t.TempDir(), "config.yaml")
	if err := os.WriteFile(configPath, []byte("a: 1"), 0o640); err != nil {
		t.Fatal(err)
	}

	h1, err := configFileHash(configPath)
	if err != nil {
		t.Fatal(err)
	}
	h2, err := configFileHash(configPath)
	if err != nil {
		t.Fatal(err)
	}
	if h1 != h2 {
		t.Fatalf("expected the same hash for unchanged content, got %q then %q", h1, h2)
	}

	if err := os.WriteFile(configPath, []byte("a: 2"), 0o640); err != nil {
		t.Fatal(err)
	}
	h3, err := configFileHash(configPath)
	if err != nil {
		t.Fatal(err)
	}
	if h3 == h1 {
		t.Fatal("expected a different hash after the content changed")
	}
}

func TestConfigFileHash_MissingFileReturnsError(t *testing.T) {
	if _, err := configFileHash(filepath.Join(t.TempDir(), "does-not-exist.yaml")); err == nil {
		t.Fatal("expected an error for a missing config file")
	}
}

func TestCheckEffectiveConfigDrift_SendsOnFirstCheckAndOnChangeOnly(t *testing.T) {
	configPath := filepath.Join(t.TempDir(), "config.yaml")
	if err := os.WriteFile(configPath, []byte("receivers: {}"), 0o640); err != nil {
		t.Fatal(err)
	}
	fake := &fakeOpAMPClient{}
	r := &opampReporter{client: fake, configPath: configPath}

	r.checkEffectiveConfigDrift(zap.NewNop())
	if fake.updateEffectiveConfigCalls != 1 {
		t.Fatalf("expected 1 call after the first check, got %d", fake.updateEffectiveConfigCalls)
	}

	r.checkEffectiveConfigDrift(zap.NewNop())
	if fake.updateEffectiveConfigCalls != 1 {
		t.Fatalf("expected no additional call when the file is unchanged, got %d total", fake.updateEffectiveConfigCalls)
	}

	if err := os.WriteFile(configPath, []byte("receivers: {}\n# edited by hand"), 0o640); err != nil {
		t.Fatal(err)
	}
	r.checkEffectiveConfigDrift(zap.NewNop())
	if fake.updateEffectiveConfigCalls != 2 {
		t.Fatalf("expected a second call after the file changed, got %d total", fake.updateEffectiveConfigCalls)
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
