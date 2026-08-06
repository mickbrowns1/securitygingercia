package main

import "testing"

func TestFilterByTag_ExactMatchBothDirections(t *testing.T) {
	agents := []Agent{
		{ID: "1", Hostname: "fedora-1", Tags: []string{"role:collector", "env:staging"}},
		{ID: "2", Hostname: "fedora-2", Tags: []string{"role:collector", "env:staging"}},
		{ID: "3", Hostname: "ubuntu-1", Tags: []string{"role:collector", "env:prod"}},
		{ID: "4", Hostname: "ubuntu-2", Tags: []string{"role:collector", "env:prod"}},
	}

	staging := filterByTag(agents, "env:staging")
	if len(staging) != 2 || staging[0].Hostname != "fedora-1" || staging[1].Hostname != "fedora-2" {
		t.Fatalf("expected exactly the 2 fedora agents for env:staging, got %+v", staging)
	}

	prod := filterByTag(agents, "env:prod")
	if len(prod) != 2 || prod[0].Hostname != "ubuntu-1" || prod[1].Hostname != "ubuntu-2" {
		t.Fatalf("expected exactly the 2 ubuntu agents for env:prod, got %+v", prod)
	}
}

func TestFilterByTag_NoMatchesReturnsEmptyNotAll(t *testing.T) {
	agents := []Agent{
		{ID: "1", Hostname: "fedora-1", Tags: []string{"env:staging"}},
	}
	got := filterByTag(agents, "env:nonexistent")
	if len(got) != 0 {
		t.Fatalf("expected no matches, got %+v", got)
	}
}

func TestFilterByTag_IsExactNotSubstring(t *testing.T) {
	agents := []Agent{
		{ID: "1", Hostname: "a", Tags: []string{"env:staging-2"}},
	}
	got := filterByTag(agents, "env:staging")
	if len(got) != 0 {
		t.Fatalf("expected filterByTag to require an exact tag match, not a substring match, got %+v", got)
	}
}
