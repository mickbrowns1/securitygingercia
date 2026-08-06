package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
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
// message -- it offers remote config (Phase 2) and packages (Phase 4)
// alongside accepting status reports (Phase 1) and effective-config
// reports (Phase 5, config-drift detection). AcceptsEffectiveConfig is
// purely declarative -- confirmed via the SDK source that it gates
// nothing on either side, unlike AcceptsPackages -- but it's still set
// here so agents inspecting this server's advertised capabilities see an
// accurate picture.
const serverCapabilities = uint64(protobufs.ServerCapabilities_ServerCapabilities_AcceptsStatus) |
	uint64(protobufs.ServerCapabilities_ServerCapabilities_OffersRemoteConfig) |
	uint64(protobufs.ServerCapabilities_ServerCapabilities_OffersPackages) |
	uint64(protobufs.ServerCapabilities_ServerCapabilities_AcceptsEffectiveConfig)

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

// pushPackage sends a PackagesAvailable offer to agent id, if it's
// currently connected -- the Phase 4 analogue of pushConfig. downloadURL
// points back at this same fleet server's own GET /packages/{name}/download
// endpoint; the agent fetches the actual bytes with a separate plain HTTP
// GET to that URL rather than receiving them inline over this message (per
// the OpAMP spec, DownloadableFile only ever carries a URL + hash). Only
// one package name is ever offered at a time in this project, so
// AllPackagesHash (nominally an aggregate over every package the agent
// should have) is simply set to this one package's own content hash --
// meaningful here only in that it changes whenever the offered package
// does, which is the one property this phase's manual (non-SDK-syncer)
// handling actually depends on.
//
// token, if non-empty, is carried as an Authorization header on the
// DownloadableFile -- confirmed live that this is required, not optional:
// the download endpoint is bearer-token-gated the same way OpAMP
// connections are, but the agent's own download request is a plain
// http.Get with no awareness of the OpAMP connection's own Authorization
// header, so without this the download comes back 401 and the whole push
// fails even though everything else about it was correct.
func pushPackage(ctx context.Context, registry *connRegistry, id, name, version string, hash []byte, downloadURL, token string) error {
	conn, ok := registry.get(id)
	if !ok {
		return errAgentNotConnected
	}
	file := &protobufs.DownloadableFile{
		DownloadUrl: downloadURL,
		ContentHash: hash,
	}
	if token != "" {
		file.Headers = &protobufs.Headers{
			Headers: []*protobufs.Header{{Key: "Authorization", Value: "Bearer " + token}},
		}
	}
	msg := &protobufs.ServerToAgent{
		Capabilities: serverCapabilities,
		PackagesAvailable: &protobufs.PackagesAvailable{
			Packages: map[string]*protobufs.PackageAvailable{
				name: {
					Type:    protobufs.PackageType_PackageType_TopLevel,
					Version: version,
					File:    file,
					Hash:    hash,
				},
			},
			AllPackagesHash: hash,
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

		// Restart/flap detection (Phase 5): the same snapshot blob already
		// carries started_at (statuscfgextension's MetricsSnapshot), so
		// this just needs to pull that one field out -- no new capability
		// or message type, same pattern as topology riding along in
		// Phase 3.
		var sa struct {
			StartedAt string `json:"started_at"`
		}
		if err := json.Unmarshal(cm.Data, &sa); err == nil && sa.StartedAt != "" {
			if err := st.recordStartedAt(ctx, id, sa.StartedAt); err != nil {
				logger.Error("recording restart history", zap.String("agent", id), zap.Error(err))
			}
		}
	}

	// Config-drift detection (Phase 5): msg.EffectiveConfig is whatever
	// config the agent reports actually running, independent of anything
	// this server itself pushed -- only the hash is kept (mirrors
	// LastKnownGoodConfig's "store full content privately, expose only
	// the hash" pattern, except here there's no need to keep the full
	// content at all since nothing re-sends it).
	if ec := msg.EffectiveConfig; ec != nil {
		if file := ec.GetConfigMap().GetConfigMap()[""]; file != nil {
			hash := sha256.Sum256(file.GetBody())
			if err := st.setEffectiveConfigHash(ctx, id, hex.EncodeToString(hash[:])); err != nil {
				logger.Error("recording effective config hash", zap.String("agent", id), zap.Error(err))
			}
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

	// Only one package can ever be pending per agent in this schema (mirrors
	// the config-push single-slot design), so every entry in the map is
	// treated as reporting on that one pending push -- there's no per-name
	// disambiguation needed at this project's scale (one managed package,
	// sgcia-otelcol).
	if ps := msg.PackageStatuses; ps != nil {
		for _, status := range ps.Packages {
			switch status.Status {
			case protobufs.PackageStatusEnum_PackageStatusEnum_Installed:
				if err := st.promotePackageToLastKnownGood(ctx, id); err != nil {
					logger.Error("promoting installed package to last-known-good", zap.String("agent", id), zap.Error(err))
				}
			case protobufs.PackageStatusEnum_PackageStatusEnum_InstallFailed:
				if err := st.recordPackageFailure(ctx, id, status.ErrorMessage); err != nil {
					logger.Error("recording package push failure", zap.String("agent", id), zap.Error(err))
				}
				logger.Warn("agent rejected pushed package", zap.String("agent", id), zap.String("error", status.ErrorMessage))
			}
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
