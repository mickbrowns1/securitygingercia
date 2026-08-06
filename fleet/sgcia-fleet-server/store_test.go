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
