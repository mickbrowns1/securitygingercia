package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"strings"

	"go.uber.org/zap"
	"gopkg.in/yaml.v3"
)

// newAPIHandlers registers the fleet server's REST endpoints: Phase 1's
// read-only GET /agents (list, optionally filtered by ?tag=) and GET
// /agents/{id} (detail); Phase 2's POST /agents/{id}/config (push a new
// config) and POST /agents/{id}/rollback (re-push the last-known-good
// one); and Phase 3's PUT /agents/{id}/tags and POST
// /agents/bulk/config?tag=... (push to every agent currently carrying a
// tag).
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
		if tag := r.URL.Query().Get("tag"); tag != "" {
			agents = filterByTag(agents, tag)
		}
		if agents == nil {
			agents = []Agent{}
		}
		writeJSON(w, logger, agents)
	})

	// Registered before "/agents/" so this exact path is matched first --
	// otherwise splitAgentPath below would parse "bulk" as an agent id.
	mux.HandleFunc("/agents/bulk/config", func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPost {
			http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
			return
		}
		handleBulkPushConfig(w, r, st, registry, logger)
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
		case action == "tags" && r.Method == http.MethodPut:
			handleSetTags(w, r, st, logger, id)
		default:
			http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		}
	})
}

// filterByTag returns only the agents whose Tags include tag, exactly --
// computed in Go over the already-fetched list rather than in SQL, matching
// this codebase's existing style (e.g. the webui's own healthy/unhealthy
// counts) and in keeping with this project's small-fleet scale.
func filterByTag(agents []Agent, tag string) []Agent {
	out := make([]Agent, 0, len(agents))
	for _, a := range agents {
		for _, t := range a.Tags {
			if t == tag {
				out = append(out, a)
				break
			}
		}
	}
	return out
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

// pushConfigToAgent is the shared sequence behind every config push in
// this server, whatever triggers it (a single-agent push, a rollback, or
// one iteration of a bulk-by-tag push): record it as pending, then deliver
// it immediately if the agent is connected. Returns errAgentNotConnected
// (unwrapped, check with errors.Is) if it isn't -- the push still stays
// recorded as pending; there's no queueing/resend-on-reconnect in this
// phase.
func pushConfigToAgent(ctx context.Context, st *store, registry *connRegistry, id, configYAML, hashHex string, hash []byte) error {
	if err := st.setPendingConfig(ctx, id, configYAML, hashHex); err != nil {
		return fmt.Errorf("recording pending config: %w", err)
	}
	return pushConfig(ctx, registry, id, configYAML, hash)
}

// handlePushConfig validates the request is well-formed YAML (cheap,
// server-side check that saves a round trip to a clearly-broken push --
// the agent's own `sgcia-otelcol validate` remains the real safety gate),
// then delivers it via pushConfigToAgent.
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

	if err := pushConfigToAgent(r.Context(), st, registry, id, req.ConfigYAML, hashHex, hash[:]); err != nil {
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

	if err := pushConfigToAgent(r.Context(), st, registry, id, agent.LastKnownGoodConfig, hashHex, hash[:]); err != nil {
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

type bulkPushResult struct {
	ID       string `json:"id"`
	Hostname string `json:"hostname"`
	Status   string `json:"status"`
	Error    string `json:"error,omitempty"`
}

// handleBulkPushConfig pushes one config to every agent currently carrying
// ?tag=..., reusing pushConfigToAgent per agent rather than a parallel
// implementation. Unlike the single-agent endpoint, one call can produce a
// mix of outcomes (some agents connected, some not), so the response
// always reports per-agent results instead of a single status code
// standing in for all of them.
func handleBulkPushConfig(w http.ResponseWriter, r *http.Request, st *store, registry *connRegistry, logger *zap.Logger) {
	tag := r.URL.Query().Get("tag")
	if tag == "" {
		http.Error(w, "?tag= is required", http.StatusBadRequest)
		return
	}

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

	all, err := st.listAgents(r.Context())
	if err != nil {
		logger.Error("listing agents for bulk push", zap.Error(err))
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	matched := filterByTag(all, tag)
	if len(matched) == 0 {
		http.Error(w, "no agents carry tag "+tag, http.StatusNotFound)
		return
	}

	hash := sha256.Sum256([]byte(req.ConfigYAML))
	hashHex := hex.EncodeToString(hash[:])

	results := make([]bulkPushResult, 0, len(matched))
	for _, a := range matched {
		res := bulkPushResult{ID: a.ID, Hostname: a.Hostname, Status: "sent"}
		if err := pushConfigToAgent(r.Context(), st, registry, a.ID, req.ConfigYAML, hashHex, hash[:]); err != nil {
			if errors.Is(err, errAgentNotConnected) {
				res.Status = "pending"
				res.Error = "agent not connected"
			} else {
				logger.Error("bulk-pushing config", zap.String("agent", a.ID), zap.Error(err))
				res.Status = "error"
				res.Error = err.Error()
			}
		}
		results = append(results, res)
	}

	writeJSON(w, logger, map[string]any{"matched": len(matched), "results": results})
}

type setTagsRequest struct {
	Tags []string `json:"tags"`
}

// handleSetTags full-replaces an agent's tag set. Tags are normalized
// (lowercased, trimmed, deduped) here so filterByTag's exact-match
// comparison stays simple; a tag containing a comma is rejected outright
// since that would be ambiguous with the storage delimiter.
func handleSetTags(w http.ResponseWriter, r *http.Request, st *store, logger *zap.Logger, id string) {
	var req setTagsRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, "decoding request body: "+err.Error(), http.StatusBadRequest)
		return
	}

	seen := make(map[string]bool, len(req.Tags))
	normalized := make([]string, 0, len(req.Tags))
	for _, t := range req.Tags {
		t = strings.ToLower(strings.TrimSpace(t))
		if t == "" {
			continue
		}
		if strings.Contains(t, ",") {
			http.Error(w, "tags must not contain a comma: "+t, http.StatusBadRequest)
			return
		}
		if !seen[t] {
			seen[t] = true
			normalized = append(normalized, t)
		}
	}

	if err := st.setTags(r.Context(), id, normalized); err != nil {
		logger.Error("setting tags", zap.String("agent", id), zap.Error(err))
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}

	writeJSON(w, logger, map[string]any{"id": id, "tags": normalized})
}

func writeJSON(w http.ResponseWriter, logger *zap.Logger, v any) {
	w.Header().Set("Content-Type", "application/json")
	if err := json.NewEncoder(w).Encode(v); err != nil {
		logger.Error("encoding JSON response", zap.Error(err))
	}
}
