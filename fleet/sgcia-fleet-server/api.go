package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strings"

	"go.uber.org/zap"
	"gopkg.in/yaml.v3"
)

// newAPIHandlers registers the fleet server's REST endpoints: Phase 1's
// read-only GET /agents (list, optionally filtered by ?tag=) and GET
// /agents/{id} (detail); Phase 2's POST /agents/{id}/config (push a new
// config) and POST /agents/{id}/rollback (re-push the last-known-good
// one); Phase 3's PUT /agents/{id}/tags and POST
// /agents/bulk/config?tag=... (push to every agent currently carrying a
// tag); DELETE /agents/{id} to clear a stale/duplicate inventory row; and
// Phase 4's GET /packages (list uploaded versions), POST
// /packages/{name}?version=... (upload a new one), GET
// /packages/{name}/download?version=... (fetched by agents, not browsers
// -- token-gated the same way OpAMP connections are), POST
// /agents/{id}/package + /agents/bulk/package?tag=... (push), and POST
// /agents/{id}/package/rollback.
func newAPIHandlers(mux *http.ServeMux, st *store, registry *connRegistry, logger *zap.Logger, packagesDir, token string) {
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

	mux.HandleFunc("/agents/bulk/package", func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPost {
			http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
			return
		}
		handleBulkPushPackage(w, r, st, registry, logger, token)
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
		case action == "" && r.Method == http.MethodDelete:
			handleDeleteAgent(w, r, st, logger, id)
		case action == "package" && r.Method == http.MethodPost:
			handlePushPackage(w, r, st, registry, logger, id, token)
		case action == "package/rollback" && r.Method == http.MethodPost:
			handleRollbackPackage(w, r, st, registry, logger, id, token)
		default:
			http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		}
	})

	mux.HandleFunc("/packages", func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet {
			http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
			return
		}
		handleListPackages(w, r, st, logger)
	})

	mux.HandleFunc("/packages/", func(w http.ResponseWriter, r *http.Request) {
		name, action, ok := splitPackagePath(r.URL.Path)
		if !ok {
			http.NotFound(w, r)
			return
		}
		switch {
		case action == "" && r.Method == http.MethodPost:
			handleUploadPackage(w, r, st, logger, packagesDir, name)
		case action == "download" && r.Method == http.MethodGet:
			handleDownloadPackage(w, r, st, logger, packagesDir, token, name)
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

// splitPackagePath parses "/packages/{name}" or "/packages/{name}/download"
// out of a request path -- the packages-tree analogue of splitAgentPath
// above (same shape, different prefix).
func splitPackagePath(path string) (name, action string, ok bool) {
	rest := strings.TrimPrefix(path, "/packages/")
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

// isSafePackagePathComponent rejects anything that isn't safe to use
// directly as a single path segment on disk (packagesDir/name/version) --
// package names and versions come from an authenticated operator via the
// REST API, not from agents or the public, but this is still cheap
// insurance against a stray "/" or ".." reaching the filesystem.
func isSafePackagePathComponent(s string) bool {
	return s != "" && !strings.Contains(s, "/") && !strings.Contains(s, "\\") && s != "." && s != ".."
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

// handleDeleteAgent removes an agent's inventory row -- for clearing
// stale/duplicate entries. If the agent is actually still running and
// connected, it simply reappears on its next OpAMP message; this only
// ever touches the inventory row, never the live connection.
func handleDeleteAgent(w http.ResponseWriter, r *http.Request, st *store, logger *zap.Logger, id string) {
	deleted, err := st.deleteAgent(r.Context(), id)
	if err != nil {
		logger.Error("deleting agent", zap.String("agent", id), zap.Error(err))
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	if !deleted {
		http.NotFound(w, r)
		return
	}
	w.WriteHeader(http.StatusNoContent)
}

// handleUploadPackage stores a new named+versioned binary (raw request
// body) under packagesDir/name/version, writing to a temp file in the same
// directory and renaming into place only once the full body has been
// received -- the same atomic-write pattern used everywhere else in this
// project a file must never be left half-written. The uploaded content's
// sha256 is computed while streaming to disk (never buffered fully in
// memory -- these binaries can be tens of megabytes) and recorded via
// st.recordPackage, becoming the ContentHash a later push offers to
// agents.
func handleUploadPackage(w http.ResponseWriter, r *http.Request, st *store, logger *zap.Logger, packagesDir, name string) {
	version := strings.TrimSpace(r.URL.Query().Get("version"))
	if version == "" {
		http.Error(w, "?version= is required", http.StatusBadRequest)
		return
	}
	if !isSafePackagePathComponent(name) || !isSafePackagePathComponent(version) {
		http.Error(w, "name and version must be non-empty and must not contain '/' or '..'", http.StatusBadRequest)
		return
	}

	dir := filepath.Join(packagesDir, name)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		logger.Error("creating package directory", zap.Error(err))
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}

	finalPath := filepath.Join(dir, version)
	tmpPath := finalPath + ".tmp"
	f, err := os.OpenFile(tmpPath, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0o644)
	if err != nil {
		logger.Error("creating package temp file", zap.Error(err))
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}

	hasher := sha256.New()
	_, copyErr := io.Copy(io.MultiWriter(f, hasher), r.Body)
	closeErr := f.Close()
	if copyErr != nil || closeErr != nil {
		os.Remove(tmpPath)
		err := copyErr
		if err == nil {
			err = closeErr
		}
		logger.Error("writing uploaded package", zap.Error(err))
		http.Error(w, "writing uploaded package: "+err.Error(), http.StatusInternalServerError)
		return
	}
	if err := os.Rename(tmpPath, finalPath); err != nil {
		os.Remove(tmpPath)
		logger.Error("finalizing uploaded package", zap.Error(err))
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}

	hashHex := hex.EncodeToString(hasher.Sum(nil))
	if err := st.recordPackage(r.Context(), name, version, hashHex); err != nil {
		logger.Error("recording package metadata", zap.String("package", name), zap.Error(err))
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}

	writeJSON(w, logger, map[string]string{"name": name, "version": version, "hash": hashHex})
}

// handleListPackages returns every uploaded package version, newest first
// within each name -- what the fleet webui's upload/push forms use to
// populate a version picker without the operator having to remember exact
// version strings by hand.
func handleListPackages(w http.ResponseWriter, r *http.Request, st *store, logger *zap.Logger) {
	packages, err := st.listPackages(r.Context())
	if err != nil {
		logger.Error("listing packages", zap.Error(err))
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	if packages == nil {
		packages = []PackageMeta{}
	}
	writeJSON(w, logger, packages)
}

// handleDownloadPackage serves the raw bytes of one uploaded package
// version -- this is the URL agents fetch via a plain HTTP GET after
// receiving a PackagesAvailable offer (per the OpAMP spec, the offer
// itself only ever carries a DownloadableFile{DownloadUrl, ContentHash},
// never the bytes inline). Gated by the same shared bearer token as OpAMP
// connections, since this endpoint is otherwise just as sensitive
// (whoever can fetch a build can also probe for others). ServeContent is
// used specifically because it supports Range requests, which the OpAMP
// spec recommends this endpoint support for resumable downloads.
func handleDownloadPackage(w http.ResponseWriter, r *http.Request, st *store, logger *zap.Logger, packagesDir, token, name string) {
	if token != "" && r.Header.Get("Authorization") != "Bearer "+token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}

	version := strings.TrimSpace(r.URL.Query().Get("version"))
	if version == "" {
		http.Error(w, "?version= is required", http.StatusBadRequest)
		return
	}
	if !isSafePackagePathComponent(name) || !isSafePackagePathComponent(version) {
		http.NotFound(w, r)
		return
	}

	pkg, err := st.getPackage(r.Context(), name, version)
	if err != nil {
		logger.Error("looking up package", zap.Error(err))
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	if pkg == nil {
		http.NotFound(w, r)
		return
	}

	f, err := os.Open(filepath.Join(packagesDir, name, version))
	if err != nil {
		logger.Error("opening package file", zap.String("package", name), zap.Error(err))
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	defer f.Close()
	info, err := f.Stat()
	if err != nil {
		logger.Error("stat-ing package file", zap.Error(err))
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}

	w.Header().Set("Content-Type", "application/octet-stream")
	http.ServeContent(w, r, name+"-"+version, info.ModTime(), f)
}

// downloadURLFor builds the URL an offered package's DownloadableFile
// points agents at -- this same fleet server's own download endpoint, on
// whatever scheme/host the operator's own push request just arrived on
// (this project has no separate public-facing hostname/cert configuration
// for the fleet server, so the incoming request is the only signal
// available for how agents should reach it back).
func downloadURLFor(r *http.Request, name, version string) string {
	scheme := "http"
	if r.TLS != nil {
		scheme = "https"
	}
	return fmt.Sprintf("%s://%s/packages/%s/download?version=%s", scheme, r.Host, url.PathEscape(name), url.QueryEscape(version))
}

type pushPackageRequest struct {
	Name    string `json:"name"`
	Version string `json:"version"`
}

// pushPackageToAgent is the Phase 4 analogue of pushConfigToAgent: record
// the push as pending, then deliver it immediately if the agent is
// connected. Same errAgentNotConnected contract -- the push stays recorded
// as pending either way.
func pushPackageToAgent(ctx context.Context, st *store, registry *connRegistry, id, name, version, hashHex string, hash []byte, downloadURL, token string) error {
	if err := st.setPendingPackage(ctx, id, name, version, hashHex); err != nil {
		return fmt.Errorf("recording pending package: %w", err)
	}
	return pushPackage(ctx, registry, id, name, version, hash, downloadURL, token)
}

// handlePushPackage looks up the named+versioned package's hash (recorded
// at upload time) and pushes it to one agent -- the request body only
// needs to name a version, never repeat its content or hash by hand.
func handlePushPackage(w http.ResponseWriter, r *http.Request, st *store, registry *connRegistry, logger *zap.Logger, id, token string) {
	var req pushPackageRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, "decoding request body: "+err.Error(), http.StatusBadRequest)
		return
	}
	if req.Name == "" || req.Version == "" {
		http.Error(w, "name and version must not be empty", http.StatusBadRequest)
		return
	}

	pkg, err := st.getPackage(r.Context(), req.Name, req.Version)
	if err != nil {
		logger.Error("looking up package", zap.Error(err))
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	if pkg == nil {
		http.Error(w, "no such package version -- upload it first via POST /packages/{name}?version=...", http.StatusNotFound)
		return
	}
	hash, err := hex.DecodeString(pkg.Hash)
	if err != nil {
		logger.Error("decoding stored package hash", zap.Error(err))
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}

	if err := pushPackageToAgent(r.Context(), st, registry, id, pkg.Name, pkg.Version, pkg.Hash, hash, downloadURLFor(r, pkg.Name, pkg.Version), token); err != nil {
		if errors.Is(err, errAgentNotConnected) {
			w.WriteHeader(http.StatusConflict)
			writeJSON(w, logger, map[string]string{"error": "agent not connected", "status": "pending"})
			return
		}
		logger.Error("pushing package", zap.String("agent", id), zap.Error(err))
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}

	w.WriteHeader(http.StatusAccepted)
	writeJSON(w, logger, map[string]string{"status": "sent", "version": pkg.Version, "hash": pkg.Hash})
}

// handleRollbackPackage re-pushes an agent's last-known-good package the
// same way handlePushPackage sends a new one -- the fleet server already
// has that version's bytes on disk from when it was originally uploaded,
// so there's nothing for the agent to have kept a local backup of.
func handleRollbackPackage(w http.ResponseWriter, r *http.Request, st *store, registry *connRegistry, logger *zap.Logger, id, token string) {
	agent, err := st.getAgent(r.Context(), id)
	if err != nil {
		logger.Error("getting agent for package rollback", zap.String("agent", id), zap.Error(err))
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	if agent == nil || agent.LastKnownGoodPackageVersion == "" {
		http.Error(w, "no last-known-good package recorded for this agent", http.StatusNotFound)
		return
	}

	hash, err := hex.DecodeString(agent.LastKnownGoodPackageHash)
	if err != nil {
		logger.Error("decoding stored last-known-good package hash", zap.Error(err))
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}

	name, version := agent.LastKnownGoodPackageName, agent.LastKnownGoodPackageVersion
	if err := pushPackageToAgent(r.Context(), st, registry, id, name, version, agent.LastKnownGoodPackageHash, hash, downloadURLFor(r, name, version), token); err != nil {
		if errors.Is(err, errAgentNotConnected) {
			w.WriteHeader(http.StatusConflict)
			writeJSON(w, logger, map[string]string{"error": "agent not connected", "status": "pending"})
			return
		}
		logger.Error("pushing package rollback", zap.String("agent", id), zap.Error(err))
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}

	w.WriteHeader(http.StatusAccepted)
	writeJSON(w, logger, map[string]string{"status": "sent", "version": version, "hash": agent.LastKnownGoodPackageHash})
}

type bulkPushPackageResult struct {
	ID       string `json:"id"`
	Hostname string `json:"hostname"`
	Status   string `json:"status"`
	Error    string `json:"error,omitempty"`
}

// handleBulkPushPackage pushes one package version to every agent
// currently carrying ?tag=..., the package analogue of
// handleBulkPushConfig.
func handleBulkPushPackage(w http.ResponseWriter, r *http.Request, st *store, registry *connRegistry, logger *zap.Logger, token string) {
	tag := r.URL.Query().Get("tag")
	if tag == "" {
		http.Error(w, "?tag= is required", http.StatusBadRequest)
		return
	}

	var req pushPackageRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, "decoding request body: "+err.Error(), http.StatusBadRequest)
		return
	}
	if req.Name == "" || req.Version == "" {
		http.Error(w, "name and version must not be empty", http.StatusBadRequest)
		return
	}

	pkg, err := st.getPackage(r.Context(), req.Name, req.Version)
	if err != nil {
		logger.Error("looking up package", zap.Error(err))
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	if pkg == nil {
		http.Error(w, "no such package version -- upload it first via POST /packages/{name}?version=...", http.StatusNotFound)
		return
	}
	hash, err := hex.DecodeString(pkg.Hash)
	if err != nil {
		logger.Error("decoding stored package hash", zap.Error(err))
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}

	all, err := st.listAgents(r.Context())
	if err != nil {
		logger.Error("listing agents for bulk package push", zap.Error(err))
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	matched := filterByTag(all, tag)
	if len(matched) == 0 {
		http.Error(w, "no agents carry tag "+tag, http.StatusNotFound)
		return
	}

	downloadURL := downloadURLFor(r, pkg.Name, pkg.Version)
	results := make([]bulkPushPackageResult, 0, len(matched))
	for _, a := range matched {
		res := bulkPushPackageResult{ID: a.ID, Hostname: a.Hostname, Status: "sent"}
		if err := pushPackageToAgent(r.Context(), st, registry, a.ID, pkg.Name, pkg.Version, pkg.Hash, hash, downloadURL, token); err != nil {
			if errors.Is(err, errAgentNotConnected) {
				res.Status = "pending"
				res.Error = "agent not connected"
			} else {
				logger.Error("bulk-pushing package", zap.String("agent", a.ID), zap.Error(err))
				res.Status = "error"
				res.Error = err.Error()
			}
		}
		results = append(results, res)
	}

	writeJSON(w, logger, map[string]any{"matched": len(matched), "results": results})
}

func writeJSON(w http.ResponseWriter, logger *zap.Logger, v any) {
	w.Header().Set("Content-Type", "application/json")
	if err := json.NewEncoder(w).Encode(v); err != nil {
		logger.Error("encoding JSON response", zap.Error(err))
	}
}
