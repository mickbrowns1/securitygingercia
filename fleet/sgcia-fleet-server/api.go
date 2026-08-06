package main

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"net/http"
	"strings"

	"go.uber.org/zap"
	"gopkg.in/yaml.v3"
)

// newAPIHandlers registers the fleet server's REST endpoints: Phase 1's
// read-only GET /agents (list) and GET /agents/{id} (detail), plus Phase
// 2's write path -- POST /agents/{id}/config to push a new config, and
// POST /agents/{id}/rollback to re-push the last-known-good one.
func newAPIHandlers(mux *http.ServeMux, st *store, registry *connRegistry, logger *zap.Logger) {
	mux.HandleFunc("/agents", func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet {
			http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
			return
		}
		agents, err := st.listAgents(r.Context())
		if err != nil {
			logger.Error("listing agents", zap.Error(err))
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}
		if agents == nil {
			agents = []Agent{}
		}
		writeJSON(w, logger, agents)
	})

	mux.HandleFunc("/agents/", func(w http.ResponseWriter, r *http.Request) {
		id, action, ok := splitAgentPath(r.URL.Path)
		if !ok {
			http.NotFound(w, r)
			return
		}
		switch {
		case action == "" && r.Method == http.MethodGet:
			handleGetAgent(w, r, st, logger, id)
		case action == "config" && r.Method == http.MethodPost:
			handlePushConfig(w, r, st, registry, logger, id)
		case action == "rollback" && r.Method == http.MethodPost:
			handleRollback(w, r, st, registry, logger, id)
		default:
			http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		}
	})
}

// splitAgentPath parses "/agents/{id}" or "/agents/{id}/{action}" out of a
// request path. Returns ok=false for anything else (e.g. a bare "/agents/").
func splitAgentPath(path string) (id, action string, ok bool) {
	rest := strings.TrimPrefix(path, "/agents/")
	if rest == "" {
		return "", "", false
	}
	parts := strings.SplitN(rest, "/", 2)
	if parts[0] == "" {
		return "", "", false
	}
	if len(parts) == 1 {
		return parts[0], "", true
	}
	return parts[0], parts[1], true
}

func handleGetAgent(w http.ResponseWriter, r *http.Request, st *store, logger *zap.Logger, id string) {
	agent, err := st.getAgent(r.Context(), id)
	if err != nil {
		logger.Error("getting agent", zap.String("agent", id), zap.Error(err))
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	if agent == nil {
		http.NotFound(w, r)
		return
	}
	writeJSON(w, logger, agent)
}

type pushConfigRequest struct {
	ConfigYAML string `json:"config_yaml"`
}

// handlePushConfig validates the request is well-formed YAML (cheap,
// server-side check that saves a round trip to a clearly-broken push --
// the agent's own `sgcia-otelcol validate` remains the real safety gate),
// records it as pending, and delivers it immediately if the agent is
// currently connected. No queueing: an offline agent gets its push
// recorded but not resent on reconnect in this phase.
func handlePushConfig(w http.ResponseWriter, r *http.Request, st *store, registry *connRegistry, logger *zap.Logger, id string) {
	var req pushConfigRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, "decoding request body: "+err.Error(), http.StatusBadRequest)
		return
	}
	if strings.TrimSpace(req.ConfigYAML) == "" {
		http.Error(w, "config_yaml must not be empty", http.StatusBadRequest)
		return
	}
	var probe any
	if err := yaml.Unmarshal([]byte(req.ConfigYAML), &probe); err != nil {
		http.Error(w, "config_yaml is not valid YAML: "+err.Error(), http.StatusBadRequest)
		return
	}

	hash := sha256.Sum256([]byte(req.ConfigYAML))
	hashHex := hex.EncodeToString(hash[:])

	if err := st.setPendingConfig(r.Context(), id, req.ConfigYAML, hashHex); err != nil {
		logger.Error("recording pending config", zap.String("agent", id), zap.Error(err))
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}

	if err := pushConfig(r.Context(), registry, id, req.ConfigYAML, hash[:]); err != nil {
		if errors.Is(err, errAgentNotConnected) {
			w.WriteHeader(http.StatusConflict)
			writeJSON(w, logger, map[string]string{"error": "agent not connected", "status": "pending"})
			return
		}
		logger.Error("pushing config", zap.String("agent", id), zap.Error(err))
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}

	w.WriteHeader(http.StatusAccepted)
	writeJSON(w, logger, map[string]string{"status": "sent", "config_hash": hashHex})
}

// handleRollback re-sends an agent's last-known-good config the same way
// handlePushConfig sends a new one.
func handleRollback(w http.ResponseWriter, r *http.Request, st *store, registry *connRegistry, logger *zap.Logger, id string) {
	agent, err := st.getAgent(r.Context(), id)
	if err != nil {
		logger.Error("getting agent for rollback", zap.String("agent", id), zap.Error(err))
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	if agent == nil || agent.LastKnownGoodConfig == "" {
		http.Error(w, "no last-known-good config recorded for this agent", http.StatusNotFound)
		return
	}

	hash := sha256.Sum256([]byte(agent.LastKnownGoodConfig))
	hashHex := hex.EncodeToString(hash[:])

	if err := st.setPendingConfig(r.Context(), id, agent.LastKnownGoodConfig, hashHex); err != nil {
		logger.Error("recording pending rollback", zap.String("agent", id), zap.Error(err))
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}

	if err := pushConfig(r.Context(), registry, id, agent.LastKnownGoodConfig, hash[:]); err != nil {
		if errors.Is(err, errAgentNotConnected) {
			w.WriteHeader(http.StatusConflict)
			writeJSON(w, logger, map[string]string{"error": "agent not connected", "status": "pending"})
			return
		}
		logger.Error("pushing rollback", zap.String("agent", id), zap.Error(err))
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}

	w.WriteHeader(http.StatusAccepted)
	writeJSON(w, logger, map[string]string{"status": "sent", "config_hash": hashHex})
}

func writeJSON(w http.ResponseWriter, logger *zap.Logger, v any) {
	w.Header().Set("Content-Type", "application/json")
	if err := json.NewEncoder(w).Encode(v); err != nil {
		logger.Error("encoding JSON response", zap.Error(err))
	}
}
