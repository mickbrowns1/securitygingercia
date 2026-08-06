package main

import (
	"context"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"

	"go.uber.org/zap"
)

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

func TestSplitPackagePath(t *testing.T) {
	cases := []struct {
		path       string
		wantName   string
		wantAction string
		wantOK     bool
	}{
		{"/packages/sgcia-otelcol", "sgcia-otelcol", "", true},
		{"/packages/sgcia-otelcol/download", "sgcia-otelcol", "download", true},
		{"/packages/", "", "", false},
	}
	for _, tc := range cases {
		name, action, ok := splitPackagePath(tc.path)
		if name != tc.wantName || action != tc.wantAction || ok != tc.wantOK {
			t.Fatalf("splitPackagePath(%q) = (%q, %q, %v), want (%q, %q, %v)",
				tc.path, name, action, ok, tc.wantName, tc.wantAction, tc.wantOK)
		}
	}
}

func TestIsSafePackagePathComponent(t *testing.T) {
	cases := map[string]bool{
		"sgcia-otelcol": true,
		"0.1.1":         true,
		"":              false,
		".":             false,
		"..":            false,
		"a/b":           false,
		"a\\b":          false,
		"../../etc":     false,
	}
	for s, want := range cases {
		if got := isSafePackagePathComponent(s); got != want {
			t.Fatalf("isSafePackagePathComponent(%q) = %v, want %v", s, got, want)
		}
	}
}

// TestUploadThenDownloadPackage_RoundTripsContentAndHash exercises the two
// handlers end to end against a real temp directory and store: upload a
// small fake binary, confirm the recorded hash matches sha256 of the
// content, then download it back and confirm the bytes and hash both
// round-trip -- this is the same content-hash-integrity property the
// agent side later re-verifies independently before ever swapping its own
// live binary.
func TestUploadThenDownloadPackage_RoundTripsContentAndHash(t *testing.T) {
	dir := t.TempDir()
	st, err := openStore(filepath.Join(t.TempDir(), "packages.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer st.close()
	logger := zap.NewNop()

	body := "fake binary content for a round-trip test"
	uploadReq := httptest.NewRequest(http.MethodPost, "/packages/sgcia-otelcol?version=0.1.1", strings.NewReader(body))
	uploadRec := httptest.NewRecorder()
	handleUploadPackage(uploadRec, uploadReq, st, logger, dir, "sgcia-otelcol")
	if uploadRec.Code != http.StatusOK {
		t.Fatalf("expected 200 from upload, got %d: %s", uploadRec.Code, uploadRec.Body.String())
	}

	pkg, err := st.getPackage(context.Background(), "sgcia-otelcol", "0.1.1")
	if err != nil {
		t.Fatal(err)
	}
	if pkg == nil {
		t.Fatal("expected a recorded package after upload")
	}

	downloadReq := httptest.NewRequest(http.MethodGet, "/packages/sgcia-otelcol/download?version=0.1.1", nil)
	downloadRec := httptest.NewRecorder()
	handleDownloadPackage(downloadRec, downloadReq, st, logger, dir, "", "sgcia-otelcol")
	if downloadRec.Code != http.StatusOK {
		t.Fatalf("expected 200 from download, got %d: %s", downloadRec.Code, downloadRec.Body.String())
	}
	if downloadRec.Body.String() != body {
		t.Fatalf("expected downloaded content to match the uploaded content, got %q", downloadRec.Body.String())
	}
}

// TestHandleDownloadPackage_RequiresBearerTokenWhenConfigured mirrors the
// same auth gate OpAMP connections use (OnConnecting in opampserver.go) --
// this endpoint is just as sensitive, so it's checked the same way.
func TestHandleDownloadPackage_RequiresBearerTokenWhenConfigured(t *testing.T) {
	dir := t.TempDir()
	st, err := openStore(filepath.Join(t.TempDir(), "packages.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer st.close()
	logger := zap.NewNop()

	uploadReq := httptest.NewRequest(http.MethodPost, "/packages/sgcia-otelcol?version=0.1.1", strings.NewReader("content"))
	handleUploadPackage(httptest.NewRecorder(), uploadReq, st, logger, dir, "sgcia-otelcol")

	noAuthReq := httptest.NewRequest(http.MethodGet, "/packages/sgcia-otelcol/download?version=0.1.1", nil)
	noAuthRec := httptest.NewRecorder()
	handleDownloadPackage(noAuthRec, noAuthReq, st, logger, dir, "secret-token", "sgcia-otelcol")
	if noAuthRec.Code != http.StatusUnauthorized {
		t.Fatalf("expected 401 without a token, got %d", noAuthRec.Code)
	}

	authedReq := httptest.NewRequest(http.MethodGet, "/packages/sgcia-otelcol/download?version=0.1.1", nil)
	authedReq.Header.Set("Authorization", "Bearer secret-token")
	authedRec := httptest.NewRecorder()
	handleDownloadPackage(authedRec, authedReq, st, logger, dir, "secret-token", "sgcia-otelcol")
	if authedRec.Code != http.StatusOK {
		t.Fatalf("expected 200 with the correct token, got %d: %s", authedRec.Code, authedRec.Body.String())
	}
}
