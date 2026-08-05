package main

import (
	"context"
	"encoding/hex"
	"net/http"
	"strings"

	"github.com/open-telemetry/opamp-go/protobufs"
	"github.com/open-telemetry/opamp-go/server"
	servertypes "github.com/open-telemetry/opamp-go/server/types"
	"go.uber.org/zap"
)

// snapshotCapability is the reverse-FQDN custom capability sgcia agents use
// to carry their existing /status MetricsSnapshot JSON over OpAMP, rather
// than trying to fit that data into OpAMP's generic ComponentHealth fields.
const (
	snapshotCapability  = "io.sgcia.snapshot"
	snapshotMessageType = "metrics_snapshot"
)

// newOpampCallbacks builds the server callbacks that turn incoming OpAMP
// AgentToServer messages into inventory-store updates. token, if non-empty,
// is checked as a shared bearer secret on every incoming connection --
// Phase 1's stand-in for the plan's eventual per-agent token model.
func newOpampCallbacks(st *store, logger *zap.Logger, token string) servertypes.Callbacks {
	return servertypes.Callbacks{
		OnConnecting: func(req *http.Request) servertypes.ConnectionResponse {
			if token == "" {
				return servertypes.ConnectionResponse{Accept: true, ConnectionCallbacks: connectionCallbacks(st, logger)}
			}
			auth := req.Header.Get("Authorization")
			if auth != "Bearer "+token {
				return servertypes.ConnectionResponse{Accept: false, HTTPStatusCode: http.StatusUnauthorized}
			}
			return servertypes.ConnectionResponse{Accept: true, ConnectionCallbacks: connectionCallbacks(st, logger)}
		},
	}
}

func connectionCallbacks(st *store, logger *zap.Logger) servertypes.ConnectionCallbacks {
	return servertypes.ConnectionCallbacks{
		OnMessage: func(ctx context.Context, conn servertypes.Connection, msg *protobufs.AgentToServer) *protobufs.ServerToAgent {
			id := hex.EncodeToString(msg.InstanceUid)
			handleAgentMessage(ctx, st, logger, id, msg)
			return &protobufs.ServerToAgent{
				InstanceUid:  msg.InstanceUid,
				Capabilities: uint64(protobufs.ServerCapabilities_ServerCapabilities_AcceptsStatus),
				CustomCapabilities: &protobufs.CustomCapabilities{
					Capabilities: []string{snapshotCapability},
				},
			}
		},
	}
}

func handleAgentMessage(ctx context.Context, st *store, logger *zap.Logger, id string, msg *protobufs.AgentToServer) {
	if desc := msg.AgentDescription; desc != nil {
		hostname := attrValue(desc.IdentifyingAttributes, "host.name")
		version := attrValue(desc.IdentifyingAttributes, "service.version")
		localUIAddr := attrValue(desc.NonIdentifyingAttributes, "sgcia.local_ui_addr")
		if err := st.upsertAgentDescription(ctx, id, hostname, version, localUIAddr); err != nil {
			logger.Error("recording agent description", zap.String("agent", id), zap.Error(err))
		}
	} else {
		if err := st.touchLastSeen(ctx, id); err != nil {
			logger.Error("touching last_seen", zap.String("agent", id), zap.Error(err))
		}
	}

	if h := msg.Health; h != nil {
		if err := st.setHealth(ctx, id, h.Healthy, h.LastError); err != nil {
			logger.Error("recording health", zap.String("agent", id), zap.Error(err))
		}
	}

	if cm := msg.CustomMessage; cm != nil && cm.Capability == snapshotCapability && cm.Type == snapshotMessageType {
		if err := st.setSnapshot(ctx, id, string(cm.Data)); err != nil {
			logger.Error("recording snapshot", zap.String("agent", id), zap.Error(err))
		}
	}
}

func attrValue(attrs []*protobufs.KeyValue, key string) string {
	for _, kv := range attrs {
		if kv.Key != key {
			continue
		}
		if s := kv.Value.GetStringValue(); s != "" {
			return s
		}
	}
	return ""
}

// startOpampServer attaches the OpAMP protocol handler at /v1/opamp on the
// given mux -- the caller owns the actual http.Server / listener, since the
// same server also serves the REST API and the embedded Fleet UI. The
// returned ConnContext must be set on that http.Server (it's only
// exercised for plain-HTTP OpAMP transport, but Attach's contract expects
// it to be wired through regardless of which transport agents use).
func startOpampServer(mux *http.ServeMux, st *store, logger *zap.Logger, token string) (server.ConnContext, error) {
	srv := server.New(zapOpampLogger{logger: logger})
	handler, connContext, err := srv.Attach(server.Settings{
		Callbacks: newOpampCallbacks(st, logger, token),
	})
	if err != nil {
		return nil, err
	}
	mux.HandleFunc("/v1/opamp", handler)
	return connContext, nil
}

func normalizeToken(raw string) string {
	return strings.TrimSpace(raw)
}
