package main

import (
	"encoding/json"
	"net/http"
	"strings"

	"go.uber.org/zap"
)

// newAPIHandlers registers the read-only inventory REST endpoints Phase 1
// needs: GET /agents (list) and GET /agents/{id} (detail). No write paths
// exist yet -- remote config push is a later phase.
func newAPIHandlers(mux *http.ServeMux, st *store, logger *zap.Logger) {
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
		if r.Method != http.MethodGet {
			http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
			return
		}
		id := strings.TrimPrefix(r.URL.Path, "/agents/")
		if id == "" {
			http.NotFound(w, r)
			return
		}
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
	})
}

func writeJSON(w http.ResponseWriter, logger *zap.Logger, v any) {
	w.Header().Set("Content-Type", "application/json")
	if err := json.NewEncoder(w).Encode(v); err != nil {
		logger.Error("encoding JSON response", zap.Error(err))
	}
}
