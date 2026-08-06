package main

import (
	"context"
	"database/sql"
	"path/filepath"
	"testing"
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
