package main

import (
	"context"
	"database/sql"
	"fmt"
	"path/filepath"
	"testing"
	"time"
)

// TestOpenStore_MigratesAnExistingPhase1Database reproduces the exact bug
// caught while deploying Phase 2 against the live test fleet: CREATE TABLE
// IF NOT EXISTS is a no-op against a database that already has the
// `agents` table from Phase 1, so the Phase 2 columns would silently never
// get added without an explicit migration step. This opens a database
// containing only the original Phase 1 columns (no Phase 2 ones at all,
// not even empty-valued) and confirms openStore brings it up to date.
func TestOpenStore_MigratesAnExistingPhase1Database(t *testing.T) {
	path := filepath.Join(t.TempDir(), "phase1.db")

	seed, err := sql.Open("sqlite", path)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := seed.Exec(baseSchema); err != nil {
		t.Fatal(err)
	}
	if _, err := seed.Exec(`INSERT INTO agents (id, hostname, last_seen) VALUES ('abc', 'old-agent', '2026-01-01T00:00:00Z')`); err != nil {
		t.Fatal(err)
	}
	if err := seed.Close(); err != nil {
		t.Fatal(err)
	}

	st, err := openStore(path)
	if err != nil {
		t.Fatalf("openStore on a pre-existing Phase 1 database: %v", err)
	}
	defer st.close()

	agent, err := st.getAgent(context.Background(), "abc")
	if err != nil {
		t.Fatalf("getAgent after migration: %v", err)
	}
	if agent == nil || agent.Hostname != "old-agent" {
		t.Fatalf("expected the pre-existing row to survive migration, got %+v", agent)
	}

	// The real proof: setPendingConfig writes to a Phase 2 column that
	// didn't exist in this database until openStore's migration ran.
	if err := st.setPendingConfig(context.Background(), "abc", "new: config", "deadbeef"); err != nil {
		t.Fatalf("using a Phase 2 column after migration: %v", err)
	}
}

func TestOpenStore_IsIdempotentAcrossRepeatedOpens(t *testing.T) {
	path := filepath.Join(t.TempDir(), "repeat.db")

	for i := 0; i < 3; i++ {
		st, err := openStore(path)
		if err != nil {
			t.Fatalf("open #%d: %v", i, err)
		}
		if err := st.close(); err != nil {
			t.Fatalf("close #%d: %v", i, err)
		}
	}
}

// TestOpenStore_MigratesTagsColumnOntoAPostPhase2Database is the Phase 3
// analogue of the Phase 1 migration test above: simulates a database left
// over from a Phase 2 deployment (all Phase 1+2 columns, but no `tags`
// column at all -- run every migration except the tags one, matching what
// a real pre-Phase-3 fleet server's on-disk database looks like) and
// confirms openStore brings it up to date without losing existing data.
func TestOpenStore_MigratesTagsColumnOntoAPostPhase2Database(t *testing.T) {
	path := filepath.Join(t.TempDir(), "phase2.db")

	seed, err := sql.Open("sqlite", path)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := seed.Exec(baseSchema); err != nil {
		t.Fatal(err)
	}
	for _, m := range migrations[:len(migrations)-1] { // every migration except the tags one added for Phase 3
		if _, err := seed.Exec(m); err != nil {
			t.Fatal(err)
		}
	}
	if _, err := seed.Exec(`INSERT INTO agents (id, hostname, last_seen) VALUES ('abc', 'old-agent', '2026-01-01T00:00:00Z')`); err != nil {
		t.Fatal(err)
	}
	if err := seed.Close(); err != nil {
		t.Fatal(err)
	}

	st, err := openStore(path)
	if err != nil {
		t.Fatalf("openStore on a pre-existing Phase 2 database: %v", err)
	}
	defer st.close()

	agent, err := st.getAgent(context.Background(), "abc")
	if err != nil {
		t.Fatalf("getAgent after migration: %v", err)
	}
	if agent == nil || agent.Hostname != "old-agent" {
		t.Fatalf("expected the pre-existing row to survive migration, got %+v", agent)
	}
	if len(agent.Tags) != 0 {
		t.Fatalf("expected no tags on a freshly-migrated row, got %v", agent.Tags)
	}

	if err := st.setTags(context.Background(), "abc", []string{"env:prod"}); err != nil {
		t.Fatalf("using the tags column after migration: %v", err)
	}
}

func TestSetTags_FullReplaceSemantics(t *testing.T) {
	st, err := openStore(filepath.Join(t.TempDir(), "tags.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer st.close()
	ctx := context.Background()

	if err := st.setTags(ctx, "a1", []string{"role:collector", "env:staging"}); err != nil {
		t.Fatal(err)
	}
	agent, err := st.getAgent(ctx, "a1")
	if err != nil {
		t.Fatal(err)
	}
	if len(agent.Tags) != 2 {
		t.Fatalf("expected 2 tags, got %v", agent.Tags)
	}

	// A second setTags call must fully replace the set, not merge with it.
	if err := st.setTags(ctx, "a1", []string{"env:prod"}); err != nil {
		t.Fatal(err)
	}
	agent, err = st.getAgent(ctx, "a1")
	if err != nil {
		t.Fatal(err)
	}
	if len(agent.Tags) != 1 || agent.Tags[0] != "env:prod" {
		t.Fatalf("expected setTags to fully replace the previous set, got %v", agent.Tags)
	}
}

func TestDeleteAgent(t *testing.T) {
	st, err := openStore(filepath.Join(t.TempDir(), "delete.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer st.close()
	ctx := context.Background()

	if err := st.touchLastSeen(ctx, "stale-1"); err != nil {
		t.Fatal(err)
	}

	deleted, err := st.deleteAgent(ctx, "stale-1")
	if err != nil {
		t.Fatal(err)
	}
	if !deleted {
		t.Fatal("expected deleteAgent to report a row was deleted")
	}

	agent, err := st.getAgent(ctx, "stale-1")
	if err != nil {
		t.Fatal(err)
	}
	if agent != nil {
		t.Fatalf("expected the agent to be gone after deletion, got %+v", agent)
	}

	deleted, err = st.deleteAgent(ctx, "never-existed")
	if err != nil {
		t.Fatal(err)
	}
	if deleted {
		t.Fatal("expected deleteAgent to report false for an id that never existed")
	}
}

// TestOpenStore_MigratesPackageColumnsOntoAPostPhase3Database is the Phase
// 4 analogue of the tags migration test above: simulates a database left
// over from a Phase 3 deployment (every migration up to and including
// tags, but none of Phase 4's package columns) and confirms openStore
// brings it up to date without losing existing data.
func TestOpenStore_MigratesPackageColumnsOntoAPostPhase3Database(t *testing.T) {
	path := filepath.Join(t.TempDir(), "phase3.db")

	seed, err := sql.Open("sqlite", path)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := seed.Exec(baseSchema); err != nil {
		t.Fatal(err)
	}
	phase3Cutoff := 6 // pending_config..tags, in migrations' declared order
	for _, m := range migrations[:phase3Cutoff] {
		if _, err := seed.Exec(m); err != nil {
			t.Fatal(err)
		}
	}
	if _, err := seed.Exec(`INSERT INTO agents (id, hostname, last_seen) VALUES ('abc', 'old-agent', '2026-01-01T00:00:00Z')`); err != nil {
		t.Fatal(err)
	}
	if err := seed.Close(); err != nil {
		t.Fatal(err)
	}

	st, err := openStore(path)
	if err != nil {
		t.Fatalf("openStore on a pre-existing Phase 3 database: %v", err)
	}
	defer st.close()

	agent, err := st.getAgent(context.Background(), "abc")
	if err != nil {
		t.Fatalf("getAgent after migration: %v", err)
	}
	if agent == nil || agent.Hostname != "old-agent" {
		t.Fatalf("expected the pre-existing row to survive migration, got %+v", agent)
	}

	if err := st.setPendingPackage(context.Background(), "abc", "sgcia-otelcol", "0.1.1", "deadbeef"); err != nil {
		t.Fatalf("using a Phase 4 column after migration: %v", err)
	}
}

func TestPackageRolloutLifecycle(t *testing.T) {
	st, err := openStore(filepath.Join(t.TempDir(), "packages.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer st.close()
	ctx := context.Background()

	if err := st.recordPackage(ctx, "sgcia-otelcol", "0.1.1", "hash1"); err != nil {
		t.Fatal(err)
	}
	pkg, err := st.getPackage(ctx, "sgcia-otelcol", "0.1.1")
	if err != nil {
		t.Fatal(err)
	}
	if pkg == nil || pkg.Hash != "hash1" {
		t.Fatalf("expected a recorded package with hash1, got %+v", pkg)
	}

	if err := st.setPendingPackage(ctx, "a1", "sgcia-otelcol", "0.1.1", "hash1"); err != nil {
		t.Fatal(err)
	}
	agent, err := st.getAgent(ctx, "a1")
	if err != nil {
		t.Fatal(err)
	}
	if agent.PendingPackageVersion != "0.1.1" || agent.PendingPackageHash != "hash1" {
		t.Fatalf("expected pending package state, got %+v", agent)
	}

	if err := st.promotePackageToLastKnownGood(ctx, "a1"); err != nil {
		t.Fatal(err)
	}
	agent, err = st.getAgent(ctx, "a1")
	if err != nil {
		t.Fatal(err)
	}
	if agent.PendingPackageVersion != "" || agent.LastKnownGoodPackageVersion != "0.1.1" || agent.LastKnownGoodPackageHash != "hash1" {
		t.Fatalf("expected promotion to last-known-good and cleared pending, got %+v", agent)
	}

	if err := st.setPendingPackage(ctx, "a1", "sgcia-otelcol", "0.1.2", "hash2"); err != nil {
		t.Fatal(err)
	}
	if err := st.recordPackageFailure(ctx, "a1", "boom: exec failed"); err != nil {
		t.Fatal(err)
	}
	agent, err = st.getAgent(ctx, "a1")
	if err != nil {
		t.Fatal(err)
	}
	if agent.PendingPackageVersion != "" || agent.LastPackageError != "boom: exec failed" {
		t.Fatalf("expected pending cleared and failure recorded, got %+v", agent)
	}
	if agent.LastKnownGoodPackageVersion != "0.1.1" {
		t.Fatalf("expected last-known-good to remain untouched by a failure, got %+v", agent)
	}
}

// TestOpenStore_MigratesPhase5ColumnsOntoAPostPhase4Database is the Phase
// 5 analogue of the package-columns migration test above: simulates a
// database left over from a Phase 4 deployment (every migration up to and
// including last_package_error, but none of Phase 5's restart/drift
// columns) and confirms openStore brings it up to date without losing
// existing data.
func TestOpenStore_MigratesPhase5ColumnsOntoAPostPhase4Database(t *testing.T) {
	path := filepath.Join(t.TempDir(), "phase4.db")

	seed, err := sql.Open("sqlite", path)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := seed.Exec(baseSchema); err != nil {
		t.Fatal(err)
	}
	phase4Cutoff := 13 // pending_config..last_package_error, in migrations' declared order
	for _, m := range migrations[:phase4Cutoff] {
		if _, err := seed.Exec(m); err != nil {
			t.Fatal(err)
		}
	}
	if _, err := seed.Exec(`INSERT INTO agents (id, hostname, last_seen) VALUES ('abc', 'old-agent', '2026-01-01T00:00:00Z')`); err != nil {
		t.Fatal(err)
	}
	if err := seed.Close(); err != nil {
		t.Fatal(err)
	}

	st, err := openStore(path)
	if err != nil {
		t.Fatalf("openStore on a pre-existing Phase 4 database: %v", err)
	}
	defer st.close()

	agent, err := st.getAgent(context.Background(), "abc")
	if err != nil {
		t.Fatalf("getAgent after migration: %v", err)
	}
	if agent == nil || agent.Hostname != "old-agent" {
		t.Fatalf("expected the pre-existing row to survive migration, got %+v", agent)
	}

	if err := st.recordStartedAt(context.Background(), "abc", "2026-01-01T00:00:00Z"); err != nil {
		t.Fatalf("using a Phase 5 column after migration: %v", err)
	}
}

func TestParseRestartHistory(t *testing.T) {
	cases := []struct {
		csv  string
		want []string
	}{
		{"", nil},
		{"2026-01-01T00:00:00Z", []string{"2026-01-01T00:00:00Z"}},
		{"2026-01-01T00:00:00Z,2026-01-01T00:05:00Z", []string{"2026-01-01T00:00:00Z", "2026-01-01T00:05:00Z"}},
	}
	for _, tc := range cases {
		got := parseRestartHistory(tc.csv)
		if len(got) != len(tc.want) {
			t.Fatalf("parseRestartHistory(%q) = %v, want %v", tc.csv, got, tc.want)
		}
		for i := range got {
			if got[i] != tc.want[i] {
				t.Fatalf("parseRestartHistory(%q) = %v, want %v", tc.csv, got, tc.want)
			}
		}
	}
}

func TestRecordStartedAt_FirstReportIsBaselineNotRestart(t *testing.T) {
	st, err := openStore(filepath.Join(t.TempDir(), "restart.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer st.close()
	ctx := context.Background()

	if err := st.recordStartedAt(ctx, "a1", "2026-01-01T00:00:00Z"); err != nil {
		t.Fatal(err)
	}
	agent, err := st.getAgent(ctx, "a1")
	if err != nil {
		t.Fatal(err)
	}
	if agent.LastStartedAt != "2026-01-01T00:00:00Z" {
		t.Fatalf("expected the baseline started_at to be recorded, got %q", agent.LastStartedAt)
	}
	if len(agent.RestartHistory) != 0 {
		t.Fatalf("expected no restart history from a first-ever report, got %v", agent.RestartHistory)
	}
	if agent.Flapping {
		t.Fatal("expected a brand-new agent to not be flagged as flapping")
	}
}

func TestRecordStartedAt_UnchangedIsNoop(t *testing.T) {
	st, err := openStore(filepath.Join(t.TempDir(), "restart.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer st.close()
	ctx := context.Background()

	if err := st.recordStartedAt(ctx, "a1", "2026-01-01T00:00:00Z"); err != nil {
		t.Fatal(err)
	}
	if err := st.recordStartedAt(ctx, "a1", "2026-01-01T00:00:00Z"); err != nil {
		t.Fatal(err)
	}
	agent, err := st.getAgent(ctx, "a1")
	if err != nil {
		t.Fatal(err)
	}
	if len(agent.RestartHistory) != 0 {
		t.Fatalf("expected an unchanged started_at to record no restart, got %v", agent.RestartHistory)
	}
}

func TestRecordStartedAt_ChangedRecordsARestart(t *testing.T) {
	st, err := openStore(filepath.Join(t.TempDir(), "restart.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer st.close()
	ctx := context.Background()

	if err := st.recordStartedAt(ctx, "a1", "2026-01-01T00:00:00Z"); err != nil {
		t.Fatal(err)
	}
	if err := st.recordStartedAt(ctx, "a1", "2026-01-01T00:05:00Z"); err != nil {
		t.Fatal(err)
	}
	agent, err := st.getAgent(ctx, "a1")
	if err != nil {
		t.Fatal(err)
	}
	if agent.LastStartedAt != "2026-01-01T00:05:00Z" {
		t.Fatalf("expected last_started_at to be updated, got %q", agent.LastStartedAt)
	}
	if len(agent.RestartHistory) != 1 || agent.RestartHistory[0] != "2026-01-01T00:05:00Z" {
		t.Fatalf("expected exactly one recorded restart, got %v", agent.RestartHistory)
	}
	// These timestamps are years in the past relative to the real clock,
	// so none should count toward the flapping window regardless.
	if agent.RestartCountRecent != 0 || agent.Flapping {
		t.Fatalf("expected old restarts to not count as recent, got recent=%d flapping=%v", agent.RestartCountRecent, agent.Flapping)
	}
}

func TestRecordStartedAt_HistoryIsCappedAtMaxRestartHistory(t *testing.T) {
	st, err := openStore(filepath.Join(t.TempDir(), "restart.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer st.close()
	ctx := context.Background()

	if err := st.recordStartedAt(ctx, "a1", "t0"); err != nil {
		t.Fatal(err)
	}
	for i := 1; i <= 12; i++ {
		if err := st.recordStartedAt(ctx, "a1", fmt.Sprintf("t%d", i)); err != nil {
			t.Fatal(err)
		}
	}
	agent, err := st.getAgent(ctx, "a1")
	if err != nil {
		t.Fatal(err)
	}
	if len(agent.RestartHistory) != maxRestartHistory {
		t.Fatalf("expected history capped at %d entries, got %d: %v", maxRestartHistory, len(agent.RestartHistory), agent.RestartHistory)
	}
	if agent.RestartHistory[0] != "t3" {
		t.Fatalf("expected the oldest entries to be trimmed off, got history starting at %q", agent.RestartHistory[0])
	}
}

func TestScanAgents_RecentRestartsTriggerFlapping(t *testing.T) {
	st, err := openStore(filepath.Join(t.TempDir(), "restart.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer st.close()
	ctx := context.Background()

	now := time.Now()
	if err := st.recordStartedAt(ctx, "a1", now.Add(-9*time.Minute).Format(time.RFC3339)); err != nil {
		t.Fatal(err)
	}
	for i := 1; i <= 3; i++ {
		ts := now.Add(-time.Duration(9-2*i) * time.Minute).Format(time.RFC3339)
		if err := st.recordStartedAt(ctx, "a1", ts); err != nil {
			t.Fatal(err)
		}
	}
	agent, err := st.getAgent(ctx, "a1")
	if err != nil {
		t.Fatal(err)
	}
	if agent.RestartCountRecent < flapThreshold {
		t.Fatalf("expected at least %d recent restarts, got %d (history=%v)", flapThreshold, agent.RestartCountRecent, agent.RestartHistory)
	}
	if !agent.Flapping {
		t.Fatal("expected 3+ restarts within the flap window to be flagged as flapping")
	}
}

// TestConfigDrifted_GuardConditions walks through every guard the derived
// ConfigDrifted field depends on: no drift is ever flagged before a
// config has been pushed via the fleet at all, none is flagged while the
// agent's reported effective config matches a pending (not-yet-confirmed)
// push, real drift IS flagged once it matches neither, and it resolves
// again once a completed push's hash catches up.
func TestConfigDrifted_GuardConditions(t *testing.T) {
	st, err := openStore(filepath.Join(t.TempDir(), "drift.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer st.close()
	ctx := context.Background()

	if err := st.setEffectiveConfigHash(ctx, "a1", "hash-x"); err != nil {
		t.Fatal(err)
	}
	agent, err := st.getAgent(ctx, "a1")
	if err != nil {
		t.Fatal(err)
	}
	if agent.ConfigDrifted {
		t.Fatal("expected no drift flag when no config has ever been pushed via the fleet")
	}

	if err := st.setPendingConfig(ctx, "a1", "config v1", "hash-x"); err != nil {
		t.Fatal(err)
	}
	if err := st.promoteToLastKnownGood(ctx, "a1"); err != nil {
		t.Fatal(err)
	}
	agent, err = st.getAgent(ctx, "a1")
	if err != nil {
		t.Fatal(err)
	}
	if agent.ConfigDrifted {
		t.Fatalf("expected no drift when effective config matches last-known-good, got %+v", agent)
	}

	if err := st.setPendingConfig(ctx, "a1", "config v2", "hash-y"); err != nil {
		t.Fatal(err)
	}
	if err := st.setEffectiveConfigHash(ctx, "a1", "hash-y"); err != nil {
		t.Fatal(err)
	}
	agent, err = st.getAgent(ctx, "a1")
	if err != nil {
		t.Fatal(err)
	}
	if agent.ConfigDrifted {
		t.Fatal("expected no drift flag when effective config matches a pending (not-yet-confirmed) push")
	}

	if err := st.setEffectiveConfigHash(ctx, "a1", "hash-hand-edited"); err != nil {
		t.Fatal(err)
	}
	agent, err = st.getAgent(ctx, "a1")
	if err != nil {
		t.Fatal(err)
	}
	if !agent.ConfigDrifted {
		t.Fatal("expected a drift flag once effective config matches neither last-known-good nor pending")
	}

	if err := st.setEffectiveConfigHash(ctx, "a1", "hash-y"); err != nil {
		t.Fatal(err)
	}
	if err := st.promoteToLastKnownGood(ctx, "a1"); err != nil {
		t.Fatal(err)
	}
	agent, err = st.getAgent(ctx, "a1")
	if err != nil {
		t.Fatal(err)
	}
	if agent.ConfigDrifted {
		t.Fatalf("expected drift to resolve once the promoted push matches the reported effective config, got %+v", agent)
	}
}

func TestParseTags(t *testing.T) {
	cases := []struct {
		csv  string
		want []string
	}{
		{"", []string{}},
		{"env:prod", []string{"env:prod"}},
		{"env:prod,role:collector", []string{"env:prod", "role:collector"}},
	}
	for _, tc := range cases {
		got := parseTags(tc.csv)
		if len(got) != len(tc.want) {
			t.Fatalf("parseTags(%q) = %v, want %v", tc.csv, got, tc.want)
		}
		for i := range got {
			if got[i] != tc.want[i] {
				t.Fatalf("parseTags(%q) = %v, want %v", tc.csv, got, tc.want)
			}
		}
	}
}
