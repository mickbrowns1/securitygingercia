package logbufferexporter

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"testing"

	"go.opentelemetry.io/collector/component"
	"go.opentelemetry.io/collector/exporter"
	"go.opentelemetry.io/collector/pdata/plog"
	"go.uber.org/zap"
)

func TestConsumeLogs_PostsConvertedEntriesToTheConfiguredEndpoint(t *testing.T) {
	var received []logEntry
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/internal/logs" {
			t.Errorf("path = %s, want /internal/logs", r.URL.Path)
		}
		body, err := io.ReadAll(r.Body)
		if err != nil {
			t.Fatal(err)
		}
		if err := json.Unmarshal(body, &received); err != nil {
			t.Fatal(err)
		}
		w.WriteHeader(http.StatusOK)
	}))
	defer server.Close()

	cfg := &Config{Endpoint: server.URL}
	set := exporterSettingsForTest()
	e := newExporter(cfg, set)

	logs := plog.NewLogs()
	rl := logs.ResourceLogs().AppendEmpty()
	rl.Resource().Attributes().PutStr("host.name", "test-host")
	lr := rl.ScopeLogs().AppendEmpty().LogRecords().AppendEmpty()
	lr.Body().SetStr("hello world")
	lr.SetSeverityText("INFO")
	lr.Attributes().PutStr("sourcetype", "myapp")

	if err := e.consumeLogs(context.Background(), logs); err != nil {
		t.Fatalf("consumeLogs: %v", err)
	}

	if len(received) != 1 {
		t.Fatalf("received %d entries, want 1", len(received))
	}
	got := received[0]
	if got.Body != "hello world" {
		t.Errorf("Body = %q, want %q", got.Body, "hello world")
	}
	if got.Severity != "INFO" {
		t.Errorf("Severity = %q, want %q", got.Severity, "INFO")
	}
	if got.Attributes["sourcetype"] != "myapp" {
		t.Errorf("Attributes[sourcetype] = %q, want %q", got.Attributes["sourcetype"], "myapp")
	}
	if got.Resource["host.name"] != "test-host" {
		t.Errorf("Resource[host.name] = %q, want %q", got.Resource["host.name"], "test-host")
	}
}

func TestConsumeLogs_EmptyLogsSendsNoRequest(t *testing.T) {
	called := false
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		called = true
		w.WriteHeader(http.StatusOK)
	}))
	defer server.Close()

	e := newExporter(&Config{Endpoint: server.URL}, exporterSettingsForTest())
	if err := e.consumeLogs(context.Background(), plog.NewLogs()); err != nil {
		t.Fatalf("consumeLogs: %v", err)
	}
	if called {
		t.Error("expected no HTTP request for empty logs")
	}
}

func TestConsumeLogs_FallsBackToSeverityNumberWhenTextIsEmpty(t *testing.T) {
	var received []logEntry
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		json.Unmarshal(body, &received)
		w.WriteHeader(http.StatusOK)
	}))
	defer server.Close()

	e := newExporter(&Config{Endpoint: server.URL}, exporterSettingsForTest())
	logs := plog.NewLogs()
	lr := logs.ResourceLogs().AppendEmpty().ScopeLogs().AppendEmpty().LogRecords().AppendEmpty()
	lr.Body().SetStr("no severity text set")
	lr.SetSeverityNumber(plog.SeverityNumberError)

	if err := e.consumeLogs(context.Background(), logs); err != nil {
		t.Fatalf("consumeLogs: %v", err)
	}
	if len(received) != 1 || received[0].Severity != plog.SeverityNumberError.String() {
		t.Fatalf("got %+v, want severity %q", received, plog.SeverityNumberError.String())
	}
}

func TestConfig_ValidateRejectsEmptyEndpoint(t *testing.T) {
	cfg := &Config{}
	if err := cfg.Validate(); err == nil {
		t.Error("expected an error for empty endpoint")
	}
}

func exporterSettingsForTest() exporter.Settings {
	return exporter.Settings{
		ID:                component.MustNewID("logbuffer"),
		TelemetrySettings: component.TelemetrySettings{Logger: zap.NewNop()},
		BuildInfo:         component.NewDefaultBuildInfo(),
	}
}
