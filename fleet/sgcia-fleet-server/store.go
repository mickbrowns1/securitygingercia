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

	// Package rollout state (Phase 4) -- the same pending/last-known-good
	// shape as config push above, except a package is identified by
	// name+version+hash rather than carrying its content inline (the
	// actual binary bytes live on disk under -packages-dir, referenced by
	// the packages table, not duplicated into this row).
	PendingPackageName          string `json:"pending_package_name,omitempty"`
	PendingPackageVersion       string `json:"pending_package_version,omitempty"`
	PendingPackageHash          string `json:"pending_package_hash,omitempty"`
	LastKnownGoodPackageName    string `json:"last_known_good_package_name,omitempty"`
	LastKnownGoodPackageVersion string `json:"last_known_good_package_version,omitempty"`
	LastKnownGoodPackageHash    string `json:"last_known_good_package_hash,omitempty"`
	LastPackageError            string `json:"last_package_error,omitempty"`

	// Restart/flap detection (Phase 5) -- LastStartedAt is the process
	// start time from the most recent snapshot report; RestartHistory is a
	// bounded (last maxRestartHistory) list of timestamps the process was
	// previously observed to have restarted at, stored the same
	// comma-delimited way Tags is. RestartCountRecent/Flapping are derived
	// at read time in scanAgents, not stored -- so the flap threshold/
	// window can change later without a migration.
	LastStartedAt      string   `json:"last_started_at,omitempty"`
	RestartHistory     []string `json:"restart_history,omitempty"`
	RestartCountRecent int      `json:"restart_count_recent,omitempty"`
	Flapping           bool     `json:"flapping,omitempty"`

	// Config-drift detection (Phase 5) -- EffectiveConfigHash is the sha256
	// of whatever config the agent last reported actually running (via
	// OpAMP's EffectiveConfig mechanism), independent of what the fleet
	// server itself pushed. ConfigDrifted is derived at read time: true
	// only once a config has actually been pushed via the fleet
	// (LastKnownGoodHash set) and the agent's own reported hash matches
	// neither that nor anything currently pending.
	EffectiveConfigHash string `json:"effective_config_hash,omitempty"`
	ConfigDrifted       bool   `json:"config_drifted,omitempty"`
}

// flapWindow/flapThreshold define "flapping": at least this many restarts
// observed within this trailing window. Read-time constants, not stored,
// so tuning them needs no migration.
const (
	flapWindow        = 10 * time.Minute
	flapThreshold     = 3
	maxRestartHistory = 10
)

// PackageMeta is one uploaded version of a named package (currently only
// "sgcia-otelcol" in practice, but nothing here assumes a single name).
// The binary's actual bytes live on disk under -packages-dir/name/version;
// this row is just the metadata needed to reference and verify it.
type PackageMeta struct {
	Name       string    `json:"name"`
	Version    string    `json:"version"`
	Hash       string    `json:"hash"`
	UploadedAt time.Time `json:"uploaded_at"`
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

// packagesSchema is Phase 4's new table, tracking every uploaded version of
// every package by name -- distinct from baseSchema/migrations above (which
// only ever add columns to the pre-existing agents table). CREATE TABLE IF
// NOT EXISTS is naturally idempotent on its own, unlike ALTER TABLE ADD
// COLUMN, so this runs unconditionally alongside baseSchema rather than
// through the migrations slice.
const packagesSchema = `
CREATE TABLE IF NOT EXISTS packages (
	name        TEXT NOT NULL,
	version     TEXT NOT NULL,
	hash        TEXT NOT NULL,
	uploaded_at TEXT NOT NULL,
	PRIMARY KEY (name, version)
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
	`ALTER TABLE agents ADD COLUMN pending_package_name TEXT NOT NULL DEFAULT ''`,
	`ALTER TABLE agents ADD COLUMN pending_package_version TEXT NOT NULL DEFAULT ''`,
	`ALTER TABLE agents ADD COLUMN pending_package_hash TEXT NOT NULL DEFAULT ''`,
	`ALTER TABLE agents ADD COLUMN last_known_good_package_name TEXT NOT NULL DEFAULT ''`,
	`ALTER TABLE agents ADD COLUMN last_known_good_package_version TEXT NOT NULL DEFAULT ''`,
	`ALTER TABLE agents ADD COLUMN last_known_good_package_hash TEXT NOT NULL DEFAULT ''`,
	`ALTER TABLE agents ADD COLUMN last_package_error TEXT NOT NULL DEFAULT ''`,
	`ALTER TABLE agents ADD COLUMN last_started_at TEXT NOT NULL DEFAULT ''`,
	`ALTER TABLE agents ADD COLUMN restart_history TEXT NOT NULL DEFAULT ''`,
	`ALTER TABLE agents ADD COLUMN effective_config_hash TEXT NOT NULL DEFAULT ''`,
}

func openStore(path string) (*store, error) {
	db, err := sql.Open("sqlite", path)
	if err != nil {
		return nil, fmt.Errorf("opening sqlite db %q: %w", path, err)
	}
	// sqlite handles exactly one writer at a time; a single shared *sql.DB
	// is fine for this project's write volume, but only if it's actually
	// held to ONE underlying connection -- database/sql pools multiple
	// connections by default, and modernc.org/sqlite's default (rollback)
	// journal mode returns SQLITE_BUSY the moment two of them touch the db
	// at once. Confirmed live in Phase 4 verification: an agent's package
	// download (a slow-ish GET touching the packages table) racing an
	// OpAMP heartbeat's touchLastSeen (a write, on its own goroutine) hit
	// exactly this without the line below.
	db.SetMaxOpenConns(1)
	if _, err := db.Exec(baseSchema); err != nil {
		db.Close()
		return nil, fmt.Errorf("creating schema: %w", err)
	}
	if _, err := db.Exec(packagesSchema); err != nil {
		db.Close()
		return nil, fmt.Errorf("creating packages schema: %w", err)
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

// deleteAgent removes an agent's inventory row outright -- for clearing
// stale entries (e.g. duplicate enrollments from an agent restart before
// its instance ID was persisted, or a permanently decommissioned host).
// Returns false if no row matched. Deleting a still-connected agent isn't
// specially handled: if it's still actually running, it simply reappears
// on its next OpAMP message (upsertAgentDescription/touchLastSeen insert
// a fresh row) -- there's nothing to reconcile with the live connection
// registry, which is keyed the same way and unaffected by this.
func (s *store) deleteAgent(ctx context.Context, id string) (bool, error) {
	res, err := s.db.ExecContext(ctx, `DELETE FROM agents WHERE id = ?`, id)
	if err != nil {
		return false, err
	}
	n, err := res.RowsAffected()
	if err != nil {
		return false, err
	}
	return n > 0, nil
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
	pending_config, pending_config_hash, last_known_good_config, last_known_good_hash, last_config_error, tags,
	pending_package_name, pending_package_version, pending_package_hash,
	last_known_good_package_name, last_known_good_package_version, last_known_good_package_hash, last_package_error,
	last_started_at, restart_history, effective_config_hash`

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
		var lastSeen, tagsCSV, restartHistoryCSV string
		var healthyInt int
		if err := rows.Scan(&a.ID, &a.Hostname, &a.ServiceVersion, &a.LocalUIAddr, &lastSeen, &healthyInt, &a.LastError, &a.SnapshotJSON,
			&a.PendingConfig, &a.PendingConfigHash, &a.LastKnownGoodConfig, &a.LastKnownGoodHash, &a.LastConfigError, &tagsCSV,
			&a.PendingPackageName, &a.PendingPackageVersion, &a.PendingPackageHash,
			&a.LastKnownGoodPackageName, &a.LastKnownGoodPackageVersion, &a.LastKnownGoodPackageHash, &a.LastPackageError,
			&a.LastStartedAt, &restartHistoryCSV, &a.EffectiveConfigHash); err != nil {
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

		a.RestartHistory = parseRestartHistory(restartHistoryCSV)
		cutoff := time.Now().Add(-flapWindow)
		for _, ts := range a.RestartHistory {
			t, err := time.Parse(time.RFC3339, ts)
			if err == nil && t.After(cutoff) {
				a.RestartCountRecent++
			}
		}
		a.Flapping = a.RestartCountRecent >= flapThreshold

		a.ConfigDrifted = a.LastKnownGoodHash != "" &&
			a.EffectiveConfigHash != "" &&
			a.EffectiveConfigHash != a.LastKnownGoodHash &&
			a.EffectiveConfigHash != a.PendingConfigHash

		out = append(out, a)
	}
	return out, rows.Err()
}

// parseRestartHistory splits the stored comma-delimited restart-timestamp
// string back into a slice, mirroring parseTags -- an agent with no
// recorded restarts gets nil, not [""].
func parseRestartHistory(csv string) []string {
	if csv == "" {
		return nil
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

// recordPackage upserts metadata for one uploaded package version -- the
// binary bytes themselves are written to disk by the caller (api.go),
// this just tracks name/version/hash/uploaded_at for later pushes and
// listing. Re-uploading the same name+version overwrites its hash/time,
// matching the file it just overwrote on disk.
func (s *store) recordPackage(ctx context.Context, name, version, hash string) error {
	_, err := s.db.ExecContext(ctx, `
INSERT INTO packages (name, version, hash, uploaded_at) VALUES (?, ?, ?, ?)
ON CONFLICT(name, version) DO UPDATE SET
	hash = excluded.hash,
	uploaded_at = excluded.uploaded_at
`, name, version, hash, time.Now().UTC().Format(time.RFC3339))
	return err
}

func (s *store) listPackages(ctx context.Context) ([]PackageMeta, error) {
	rows, err := s.db.QueryContext(ctx, `SELECT name, version, hash, uploaded_at FROM packages ORDER BY name, uploaded_at DESC`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	return scanPackages(rows)
}

func (s *store) getPackage(ctx context.Context, name, version string) (*PackageMeta, error) {
	rows, err := s.db.QueryContext(ctx, `SELECT name, version, hash, uploaded_at FROM packages WHERE name = ? AND version = ?`, name, version)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	pkgs, err := scanPackages(rows)
	if err != nil {
		return nil, err
	}
	if len(pkgs) == 0 {
		return nil, nil
	}
	return &pkgs[0], nil
}

func scanPackages(rows *sql.Rows) ([]PackageMeta, error) {
	var out []PackageMeta
	for rows.Next() {
		var p PackageMeta
		var uploadedAt string
		if err := rows.Scan(&p.Name, &p.Version, &p.Hash, &uploadedAt); err != nil {
			return nil, err
		}
		p.UploadedAt, _ = time.Parse(time.RFC3339, uploadedAt)
		out = append(out, p)
	}
	return out, rows.Err()
}

// setPendingPackage records a package push as outstanding for id, mirroring
// setPendingConfig's shape -- only the name/version/hash reference is
// stored here, never the binary content itself.
func (s *store) setPendingPackage(ctx context.Context, id, name, version, hash string) error {
	_, err := s.db.ExecContext(ctx, `
INSERT INTO agents (id, last_seen, pending_package_name, pending_package_version, pending_package_hash) VALUES (?, ?, ?, ?, ?)
ON CONFLICT(id) DO UPDATE SET
	pending_package_name = excluded.pending_package_name,
	pending_package_version = excluded.pending_package_version,
	pending_package_hash = excluded.pending_package_hash
`, id, time.Now().UTC().Format(time.RFC3339), name, version, hash)
	return err
}

// promotePackageToLastKnownGood is called when an agent reports Installed
// for the version currently recorded as pending -- mirrors
// promoteToLastKnownGood.
func (s *store) promotePackageToLastKnownGood(ctx context.Context, id string) error {
	_, err := s.db.ExecContext(ctx, `
UPDATE agents SET
	last_known_good_package_name = pending_package_name,
	last_known_good_package_version = pending_package_version,
	last_known_good_package_hash = pending_package_hash,
	pending_package_name = '',
	pending_package_version = '',
	pending_package_hash = '',
	last_package_error = ''
WHERE id = ?
`, id)
	return err
}

// recordPackageFailure is called when an agent reports InstallFailed --
// mirrors recordConfigFailure.
func (s *store) recordPackageFailure(ctx context.Context, id, errMsg string) error {
	_, err := s.db.ExecContext(ctx, `
UPDATE agents SET
	pending_package_name = '',
	pending_package_version = '',
	pending_package_hash = '',
	last_package_error = ?
WHERE id = ?
`, errMsg, id)
	return err
}

// recordStartedAt is called on every snapshot report with the agent's
// current process start time. If it's unchanged since the last report,
// this is a no-op (no restart happened). If it's changed and a previous
// value was already on record, the process restarted since the last
// report -- crash, a remote-config/package push triggering a deliberate
// self-restart, or a manual one -- so the new timestamp is appended to a
// bounded restart_history (mirroring the Tags CSV-string pattern) capped
// at maxRestartHistory entries. A changed value with NO previous one on
// record is just this agent's first-ever report -- recorded as the
// baseline, not counted as a restart.
func (s *store) recordStartedAt(ctx context.Context, id, startedAt string) error {
	var lastStartedAt, historyCSV string
	row := s.db.QueryRowContext(ctx, `SELECT last_started_at, restart_history FROM agents WHERE id = ?`, id)
	if err := row.Scan(&lastStartedAt, &historyCSV); err != nil && err != sql.ErrNoRows {
		return err
	}

	if lastStartedAt == startedAt {
		return nil
	}

	newHistoryCSV := historyCSV
	if lastStartedAt != "" {
		history := append(parseRestartHistory(historyCSV), startedAt)
		if len(history) > maxRestartHistory {
			history = history[len(history)-maxRestartHistory:]
		}
		newHistoryCSV = strings.Join(history, ",")
	}

	_, err := s.db.ExecContext(ctx, `
INSERT INTO agents (id, last_seen, last_started_at, restart_history) VALUES (?, ?, ?, ?)
ON CONFLICT(id) DO UPDATE SET
	last_started_at = excluded.last_started_at,
	restart_history = excluded.restart_history
`, id, time.Now().UTC().Format(time.RFC3339), startedAt, newHistoryCSV)
	return err
}

// setEffectiveConfigHash records the sha256 of whatever config an agent
// last reported actually running, via OpAMP's EffectiveConfig mechanism --
// independent of pending_config/last_known_good_hash, which track what
// the fleet server itself pushed. ConfigDrifted (scanAgents) is the
// derived comparison between the two.
func (s *store) setEffectiveConfigHash(ctx context.Context, id, hash string) error {
	_, err := s.db.ExecContext(ctx, `
INSERT INTO agents (id, last_seen, effective_config_hash) VALUES (?, ?, ?)
ON CONFLICT(id) DO UPDATE SET effective_config_hash = excluded.effective_config_hash
`, id, time.Now().UTC().Format(time.RFC3339), hash)
	return err
}

func boolToInt(b bool) int {
	if b {
		return 1
	}
	return 0
}
