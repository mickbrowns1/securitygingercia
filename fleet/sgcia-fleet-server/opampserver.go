package main

import (
	"context"
	"encoding/hex"
	"errors"
	"net/http"
	"strings"
	"sync"

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

// serverCapabilities is what this server advertises in every ServerToAgent
// message -- it offers remote config (Phase 2) alongside accepting status
// reports (Phase 1).
const serverCapabilities = uint64(protobufs.ServerCapabilities_ServerCapabilities_AcceptsStatus) |
	uint64(protobufs.ServerCapabilities_ServerCapabilities_OffersRemoteConfig)

// connRegistry tracks the live OpAMP Connection for each currently-connected
// agent, keyed the same way store.go keys agents (hex InstanceUid). This is
// what lets a REST handler push a RemoteConfig to a specific agent
// out-of-band, rather than only ever replying to a message that agent just
// sent. A plain mutex-guarded map is deliberately not a more elaborate
// structure -- this project's agent counts don't warrant one.
type connRegistry struct {
	mu   sync.Mutex
	byID map[string]servertypes.Connection
}

func newConnRegistry() *connRegistry {
	return &connRegistry{byID: make(map[string]servertypes.Connection)}
}

func (r *connRegistry) register(id string, conn servertypes.Connection) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.byID[id] = conn
}

// unregister removes conn from the registry regardless of which ID it was
// registered under -- OnConnectionClose gives us the Connection, not the
// instance ID, so this is a linear scan. Fine at this project's agent counts.
func (r *connRegistry) unregister(conn servertypes.Connection) {
	r.mu.Lock()
	defer r.mu.Unlock()
	for id, c := range r.byID {
		if c == conn {
			delete(r.byID, id)
			return
		}
	}
}

func (r *connRegistry) get(id string) (servertypes.Connection, bool) {
	r.mu.Lock()
	defer r.mu.Unlock()
	c, ok := r.byID[id]
	return c, ok
}

var errAgentNotConnected = errors.New("agent not connected")

// pushConfig sends configYAML to agent id as a RemoteConfig offer, if it's
// currently connected. Returns errAgentNotConnected otherwise -- callers
// (the REST handlers) are expected to still record the push as pending so
// it's visible via GET /agents/{id}, just not delivered yet. No queueing or
// auto-resend-on-reconnect exists in this phase.
func pushConfig(ctx context.Context, registry *connRegistry, id string, configYAML string, hash []byte) error {
	conn, ok := registry.get(id)
	if !ok {
		return errAgentNotConnected
	}
	msg := &protobufs.ServerToAgent{
		Capabilities: serverCapabilities,
		RemoteConfig: &protobufs.AgentRemoteConfig{
			Config: &protobufs.AgentConfigMap{
				ConfigMap: map[string]*protobufs.AgentConfigFile{
					"": {Body: []byte(configYAML), ContentType: "text/yaml"},
				},
			},
			ConfigHash: hash,
		},
	}
	return conn.Send(ctx, msg)
}

// newOpampCallbacks builds the server callbacks that turn incoming OpAMP
// AgentToServer messages into inventory-store updates. token, if non-empty,
// is checked as a shared bearer secret on every incoming connection --
// Phase 1's stand-in for the plan's eventual per-agent token model.
func newOpampCallbacks(st *store, registry *connRegistry, logger *zap.Logger, token string) servertypes.Callbacks {
	return servertypes.Callbacks{
		OnConnecting: func(req *http.Request) servertypes.ConnectionResponse {
			if token == "" {
				return servertypes.ConnectionResponse{Accept: true, ConnectionCallbacks: connectionCallbacks(st, registry, logger)}
			}
			auth := req.Header.Get("Authorization")
			if auth != "Bearer "+token {
				return servertypes.ConnectionResponse{Accept: false, HTTPStatusCode: http.StatusUnauthorized}
			}
			return servertypes.ConnectionResponse{Accept: true, ConnectionCallbacks: connectionCallbacks(st, registry, logger)}
		},
	}
}

func connectionCallbacks(st *store, registry *connRegistry, logger *zap.Logger) servertypes.ConnectionCallbacks {
	return servertypes.ConnectionCallbacks{
		OnMessage: func(ctx context.Context, conn servertypes.Connection, msg *protobufs.AgentToServer) *protobufs.ServerToAgent {
			id := hex.EncodeToString(msg.InstanceUid)
			registry.register(id, conn)
			handleAgentMessage(ctx, st, logger, id, msg)
			return &protobufs.ServerToAgent{
				InstanceUid:  msg.InstanceUid,
				Capabilities: serverCapabilities,
				CustomCapabilities: &protobufs.CustomCapabilities{
					Capabilities: []string{snapshotCapability},
				},
			}
		},
		OnConnectionClose: func(conn servertypes.Connection) {
			registry.unregister(conn)
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

	if rcs := msg.RemoteConfigStatus; rcs != nil {
		switch rcs.Status {
		case protobufs.RemoteConfigStatuses_RemoteConfigStatuses_APPLIED:
			if err := st.promoteToLastKnownGood(ctx, id); err != nil {
				logger.Error("promoting applied config to last-known-good", zap.String("agent", id), zap.Error(err))
			}
		case protobufs.RemoteConfigStatuses_RemoteConfigStatuses_FAILED:
			if err := st.recordConfigFailure(ctx, id, rcs.ErrorMessage); err != nil {
				logger.Error("recording config push failure", zap.String("agent", id), zap.Error(err))
			}
			logger.Warn("agent rejected pushed config", zap.String("agent", id), zap.String("error", rcs.ErrorMessage))
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
func startOpampServer(mux *http.ServeMux, st *store, registry *connRegistry, logger *zap.Logger, token string) (server.ConnContext, error) {
	srv := server.New(zapOpampLogger{logger: logger})
	handler, connContext, err := srv.Attach(server.Settings{
		Callbacks: newOpampCallbacks(st, registry, logger, token),
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
