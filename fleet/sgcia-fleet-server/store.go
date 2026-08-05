package main

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
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
}

type store struct {
	db *sql.DB
}

func openStore(path string) (*store, error) {
	db, err := sql.Open("sqlite", path)
	if err != nil {
		return nil, fmt.Errorf("opening sqlite db %q: %w", path, err)
	}
	// sqlite handles exactly one writer at a time; the inventory's write
	// volume (one upsert per agent per health-report interval) never gets
	// close to contending on this, so a single shared *sql.DB is fine.
	const schema = `
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
	if _, err := db.Exec(schema); err != nil {
		db.Close()
		return nil, fmt.Errorf("creating schema: %w", err)
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

func (s *store) listAgents(ctx context.Context) ([]Agent, error) {
	rows, err := s.db.QueryContext(ctx, `
SELECT id, hostname, service_version, local_ui_addr, last_seen, healthy, last_error, snapshot_json
FROM agents ORDER BY hostname, id`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	return scanAgents(rows)
}

func (s *store) getAgent(ctx context.Context, id string) (*Agent, error) {
	rows, err := s.db.QueryContext(ctx, `
SELECT id, hostname, service_version, local_ui_addr, last_seen, healthy, last_error, snapshot_json
FROM agents WHERE id = ?`, id)
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
		var lastSeen string
		var healthyInt int
		if err := rows.Scan(&a.ID, &a.Hostname, &a.ServiceVersion, &a.LocalUIAddr, &lastSeen, &healthyInt, &a.LastError, &a.SnapshotJSON); err != nil {
			return nil, err
		}
		a.LastSeen, _ = time.Parse(time.RFC3339, lastSeen)
		a.Healthy = healthyInt != 0
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

func boolToInt(b bool) int {
	if b {
		return 1
	}
	return 0
}
