package main

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"strings"
	"time"

	_ "modernc.org/sqlite"
)

// Agent is the inventory record for one enrolled sgcia-otelcol instance,
// keyed by its OpAMP InstanceUid. AgentDescription/Health/CustomMessage
// fields on the wire are all "may be omitted if unchanged since the last
// message" per the OpAMP spec, so upsertAgent only overwrites columns it
// was actually given a value for -- never wipes a field just because a
// later message happened not to repeat it.
type Agent struct {
	ID             string `json:"id"`
	Hostname       string `json:"hostname"`
	ServiceVersion string `json:"service_version"`
	// LocalUIAddr is the agent's own statuscfgextension loopback address
	// (e.g. "127.0.0.1:7801"), reported as an identifying attribute --
	// only reachable from the agent's own host, so the Fleet UI uses it to
	// show an SSH-tunnel drill-down hint rather than a clickable link.
	LocalUIAddr  string    `json:"local_ui_addr,omitempty"`
	LastSeen     time.Time `json:"last_seen"`
	Healthy      bool      `json:"healthy"`
	LastError    string    `json:"last_error,omitempty"`
	SnapshotJSON string    `json:"-"`
	Snapshot     any       `json:"snapshot,omitempty"`

	// Config-push state (Phase 2). PendingConfigHash is non-empty while a
	// push is outstanding (sent but not yet ACKed/NAKed by the agent).
	// LastKnownGoodConfig is what a rollback re-sends. LastConfigError is
	// the agent's own validate-failure message, distinct from LastError
	// (which is Phase 1's general-health field).
	PendingConfig       string `json:"-"`
	PendingConfigHash   string `json:"pending_config_hash,omitempty"`
	LastKnownGoodConfig string `json:"-"`
	LastKnownGoodHash   string `json:"last_known_good_hash,omitempty"`
	LastConfigError     string `json:"last_config_error,omitempty"`

	// Tags (Phase 3) are operator-assigned, free-text labels (e.g.
	// "role:collector", "env:prod") used to filter GET /agents and to
	// scope bulk config pushes. Stored as a delimited string in SQLite,
	// split/joined at this boundary -- see tagsColumn/parseTags.
	Tags []string `json:"tags"`
}

type store struct {
	db *sql.DB
}

// baseSchema is the original Phase 1 shape. CREATE TABLE IF NOT EXISTS is a
// no-op against a database that already has this table -- anything added
// to the Agent shape after Phase 1 belongs in migrations below instead,
// not here.
const baseSchema = `
CREATE TABLE IF NOT EXISTS agents (
	id              TEXT PRIMARY KEY,
	hostname        TEXT NOT NULL DEFAULT '',
	service_version TEXT NOT NULL DEFAULT '',
	local_ui_addr   TEXT NOT NULL DEFAULT '',
	last_seen       TEXT NOT NULL,
	healthy         INTEGER NOT NULL DEFAULT 1,
	last_error      TEXT NOT NULL DEFAULT '',
	snapshot_json   TEXT NOT NULL DEFAULT ''
);`

// migrations adds columns introduced after Phase 1 (currently: Phase 2's
// config-push state). Run unconditionally on every startup; sqlite has no
// "ADD COLUMN IF NOT EXISTS", so idempotency comes from swallowing the
// "duplicate column name" error a re-run produces against a database that
// already has it -- this way a fresh database (built entirely from
// baseSchema, no history) and an upgraded one converge on the same shape.
var migrations = []string{
	`ALTER TABLE agents ADD COLUMN pending_config TEXT NOT NULL DEFAULT ''`,
	`ALTER TABLE agents ADD COLUMN pending_config_hash TEXT NOT NULL DEFAULT ''`,
	`ALTER TABLE agents ADD COLUMN last_known_good_config TEXT NOT NULL DEFAULT ''`,
	`ALTER TABLE agents ADD COLUMN last_known_good_hash TEXT NOT NULL DEFAULT ''`,
	`ALTER TABLE agents ADD COLUMN last_config_error TEXT NOT NULL DEFAULT ''`,
	`ALTER TABLE agents ADD COLUMN tags TEXT NOT NULL DEFAULT ''`,
}

func openStore(path string) (*store, error) {
	db, err := sql.Open("sqlite", path)
	if err != nil {
		return nil, fmt.Errorf("opening sqlite db %q: %w", path, err)
	}
	// sqlite handles exactly one writer at a time; the inventory's write
	// volume (one upsert per agent per health-report interval) never gets
	// close to contending on this, so a single shared *sql.DB is fine.
	if _, err := db.Exec(baseSchema); err != nil {
		db.Close()
		return nil, fmt.Errorf("creating schema: %w", err)
	}
	for _, m := range migrations {
		if _, err := db.Exec(m); err != nil && !strings.Contains(err.Error(), "duplicate column name") {
			db.Close()
			return nil, fmt.Errorf("running migration %q: %w", m, err)
		}
	}
	return &store{db: db}, nil
}

func (s *store) close() error {
	return s.db.Close()
}

// upsertAgentDescription records identifying attributes for an agent,
// creating the row if this is the first message seen from it.
func (s *store) upsertAgentDescription(ctx context.Context, id, hostname, serviceVersion, localUIAddr string) error {
	_, err := s.db.ExecContext(ctx, `
INSERT INTO agents (id, hostname, service_version, local_ui_addr, last_seen)
VALUES (?, ?, ?, ?, ?)
ON CONFLICT(id) DO UPDATE SET
	hostname = excluded.hostname,
	service_version = excluded.service_version,
	local_ui_addr = excluded.local_ui_addr,
	last_seen = excluded.last_seen
`, id, hostname, serviceVersion, localUIAddr, time.Now().UTC().Format(time.RFC3339))
	return err
}

// touchLastSeen updates last_seen for an agent without a full description
// (e.g. a heartbeat or a snapshot-only message), creating the row if needed.
func (s *store) touchLastSeen(ctx context.Context, id string) error {
	_, err := s.db.ExecContext(ctx, `
INSERT INTO agents (id, last_seen) VALUES (?, ?)
ON CONFLICT(id) DO UPDATE SET last_seen = excluded.last_seen
`, id, time.Now().UTC().Format(time.RFC3339))
	return err
}

func (s *store) setHealth(ctx context.Context, id string, healthy bool, lastError string) error {
	_, err := s.db.ExecContext(ctx, `
INSERT INTO agents (id, last_seen, healthy, last_error) VALUES (?, ?, ?, ?)
ON CONFLICT(id) DO UPDATE SET
	last_seen = excluded.last_seen,
	healthy = excluded.healthy,
	last_error = excluded.last_error
`, id, time.Now().UTC().Format(time.RFC3339), boolToInt(healthy), lastError)
	return err
}

func (s *store) setSnapshot(ctx context.Context, id string, snapshotJSON string) error {
	_, err := s.db.ExecContext(ctx, `
INSERT INTO agents (id, last_seen, snapshot_json) VALUES (?, ?, ?)
ON CONFLICT(id) DO UPDATE SET
	last_seen = excluded.last_seen,
	snapshot_json = excluded.snapshot_json
`, id, time.Now().UTC().Format(time.RFC3339), snapshotJSON)
	return err
}

const agentColumns = `id, hostname, service_version, local_ui_addr, last_seen, healthy, last_error, snapshot_json,
	pending_config, pending_config_hash, last_known_good_config, last_known_good_hash, last_config_error, tags`

func (s *store) listAgents(ctx context.Context) ([]Agent, error) {
	rows, err := s.db.QueryContext(ctx, `SELECT `+agentColumns+` FROM agents ORDER BY hostname, id`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	return scanAgents(rows)
}

func (s *store) getAgent(ctx context.Context, id string) (*Agent, error) {
	rows, err := s.db.QueryContext(ctx, `SELECT `+agentColumns+` FROM agents WHERE id = ?`, id)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	agents, err := scanAgents(rows)
	if err != nil {
		return nil, err
	}
	if len(agents) == 0 {
		return nil, nil
	}
	return &agents[0], nil
}

func scanAgents(rows *sql.Rows) ([]Agent, error) {
	var out []Agent
	for rows.Next() {
		var a Agent
		var lastSeen, tagsCSV string
		var healthyInt int
		if err := rows.Scan(&a.ID, &a.Hostname, &a.ServiceVersion, &a.LocalUIAddr, &lastSeen, &healthyInt, &a.LastError, &a.SnapshotJSON,
			&a.PendingConfig, &a.PendingConfigHash, &a.LastKnownGoodConfig, &a.LastKnownGoodHash, &a.LastConfigError, &tagsCSV); err != nil {
			return nil, err
		}
		a.LastSeen, _ = time.Parse(time.RFC3339, lastSeen)
		a.Healthy = healthyInt != 0
		a.Tags = parseTags(tagsCSV)
		if a.SnapshotJSON != "" {
			var snap any
			if err := json.Unmarshal([]byte(a.SnapshotJSON), &snap); err == nil {
				a.Snapshot = snap
			}
		}
		out = append(out, a)
	}
	return out, rows.Err()
}

// parseTags splits the stored comma-delimited tag string back into a
// slice, dropping empty entries -- so an agent with no tags at all gets
// []string{}, not []string{""}.
func parseTags(csv string) []string {
	if csv == "" {
		return []string{}
	}
	parts := strings.Split(csv, ",")
	out := make([]string, 0, len(parts))
	for _, p := range parts {
		if p != "" {
			out = append(out, p)
		}
	}
	return out
}

// setTags full-replaces id's tag set (kubectl label --overwrite semantics,
// not incremental add/remove -- one write, no read-modify-write race).
// Callers are expected to have already normalized tags (lowercase,
// trimmed, deduped, no embedded commas).
func (s *store) setTags(ctx context.Context, id string, tags []string) error {
	_, err := s.db.ExecContext(ctx, `
INSERT INTO agents (id, last_seen, tags) VALUES (?, ?, ?)
ON CONFLICT(id) DO UPDATE SET tags = excluded.tags
`, id, time.Now().UTC().Format(time.RFC3339), strings.Join(tags, ","))
	return err
}

// setPendingConfig records a config push as outstanding for id, creating
// the row if this agent has never been seen before (a push can target an
// agent the fleet server hasn't heard from directly yet, as long as its ID
// is known -- in practice today that means "recently seen", since IDs come
// from prior OpAMP traffic, but the schema doesn't require it).
func (s *store) setPendingConfig(ctx context.Context, id, configYAML, hash string) error {
	_, err := s.db.ExecContext(ctx, `
INSERT INTO agents (id, last_seen, pending_config, pending_config_hash) VALUES (?, ?, ?, ?)
ON CONFLICT(id) DO UPDATE SET
	pending_config = excluded.pending_config,
	pending_config_hash = excluded.pending_config_hash
`, id, time.Now().UTC().Format(time.RFC3339), configYAML, hash)
	return err
}

// promoteToLastKnownGood is called when an agent reports APPLIED for the
// hash currently recorded as pending -- the just-applied config becomes
// what a future rollback would re-send.
func (s *store) promoteToLastKnownGood(ctx context.Context, id string) error {
	_, err := s.db.ExecContext(ctx, `
UPDATE agents SET
	last_known_good_config = pending_config,
	last_known_good_hash = pending_config_hash,
	pending_config = '',
	pending_config_hash = '',
	last_config_error = ''
WHERE id = ?
`, id)
	return err
}

// recordConfigFailure is called when an agent reports FAILED -- the
// pending push is discarded (never touched the agent's live config) and
// the failure reason is kept for the operator to see.
func (s *store) recordConfigFailure(ctx context.Context, id, errMsg string) error {
	_, err := s.db.ExecContext(ctx, `
UPDATE agents SET
	pending_config = '',
	pending_config_hash = '',
	last_config_error = ?
WHERE id = ?
`, errMsg, id)
	return err
}

func boolToInt(b bool) int {
	if b {
		return 1
	}
	return 0
}
