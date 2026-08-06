package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"path/filepath"
	"testing"

	"github.com/open-telemetry/opamp-go/protobufs"
	"go.uber.org/zap"
)

// TestHandleAgentMessage_SnapshotWithStartedAtRecordsRestartHistory
// confirms the Phase 5 restart-detection hook actually fires from a real
// AgentToServer message: the metrics_snapshot CustomMessage already
// carries started_at (statuscfgextension's MetricsSnapshot, flattened by
// fleetReport's embedding), so a changed value across two messages should
// show up as a recorded restart.
func TestHandleAgentMessage_SnapshotWithStartedAtRecordsRestartHistory(t *testing.T) {
	st, err := openStore(filepath.Join(t.TempDir(), "opamp.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer st.close()
	ctx := context.Background()
	logger := zap.NewNop()

	first := &protobufs.AgentToServer{
		CustomMessage: &protobufs.CustomMessage{
			Capability: snapshotCapability,
			Type:       snapshotMessageType,
			Data:       []byte(`{"started_at":"2026-01-01T00:00:00Z","receivers":{}}`),
		},
	}
	handleAgentMessage(ctx, st, logger, "a1", first)

	agent, err := st.getAgent(ctx, "a1")
	if err != nil {
		t.Fatal(err)
	}
	if agent.LastStartedAt != "2026-01-01T00:00:00Z" {
		t.Fatalf("expected the baseline started_at to be recorded, got %q", agent.LastStartedAt)
	}
	if len(agent.RestartHistory) != 0 {
		t.Fatalf("expected no restart from the first-ever message, got %v", agent.RestartHistory)
	}

	second := &protobufs.AgentToServer{
		CustomMessage: &protobufs.CustomMessage{
			Capability: snapshotCapability,
			Type:       snapshotMessageType,
			Data:       []byte(`{"started_at":"2026-01-01T00:05:00Z","receivers":{}}`),
		},
	}
	handleAgentMessage(ctx, st, logger, "a1", second)

	agent, err = st.getAgent(ctx, "a1")
	if err != nil {
		t.Fatal(err)
	}
	if len(agent.RestartHistory) != 1 || agent.RestartHistory[0] != "2026-01-01T00:05:00Z" {
		t.Fatalf("expected the second message's started_at change to be recorded as a restart, got %v", agent.RestartHistory)
	}
}

// TestHandleAgentMessage_EffectiveConfigRecordsHash confirms the Phase 5
// drift-detection hook hashes msg.EffectiveConfig's body and stores it,
// via a real AgentToServer message shaped exactly like what the agent's
// GetEffectiveConfig callback produces.
func TestHandleAgentMessage_EffectiveConfigRecordsHash(t *testing.T) {
	st, err := openStore(filepath.Join(t.TempDir(), "opamp.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer st.close()
	ctx := context.Background()
	logger := zap.NewNop()

	body := []byte("receivers:\n  syslog/udp: {}\n")
	msg := &protobufs.AgentToServer{
		EffectiveConfig: &protobufs.EffectiveConfig{
			ConfigMap: &protobufs.AgentConfigMap{
				ConfigMap: map[string]*protobufs.AgentConfigFile{
					"": {Body: body, ContentType: "text/yaml"},
				},
			},
		},
	}
	handleAgentMessage(ctx, st, logger, "a1", msg)

	agent, err := st.getAgent(ctx, "a1")
	if err != nil {
		t.Fatal(err)
	}
	want := sha256.Sum256(body)
	if agent.EffectiveConfigHash != hex.EncodeToString(want[:]) {
		t.Fatalf("effective_config_hash = %q, want %x", agent.EffectiveConfigHash, want)
	}
}
