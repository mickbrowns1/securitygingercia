package statuscfgextension

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"

	"go.uber.org/zap"
)

func TestExtension_StatusAndConfigEndToEnd(t *testing.T) {
	configPath := writeTempConfig(t, sampleYAML)

	metrics := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.Write([]byte(samplePromText))
	}))
	defer metrics.Close()

	cfg := &Config{
		Endpoint:   "127.0.0.1:0",
		ConfigPath: configPath,
		MetricsURL: metrics.URL,
	}
	ext := newStatusCfgExtension(cfg, zap.NewNop(), "test")
	if err := ext.Start(context.Background(), nil); err != nil {
		t.Fatalf("Start: %v", err)
	}
	defer ext.Shutdown(context.Background())

	addr := ext.listenerAddr()
	client := &http.Client{Timeout: 2 * time.Second}

	statusResp, err := client.Get("http://" + addr + "/status")
	if err != nil {
		t.Fatalf("GET /status: %v", err)
	}
	defer statusResp.Body.Close()
	if statusResp.StatusCode != http.StatusOK {
		t.Fatalf("/status status = %d, want 200", statusResp.StatusCode)
	}
	var snapshot MetricsSnapshot
	if err := json.NewDecoder(statusResp.Body).Decode(&snapshot); err != nil {
		t.Fatalf("decoding /status body: %v", err)
	}
	if snapshot.Receivers["syslog/udp"].EventsIn != 0 {
		t.Errorf("syslog/udp events_in = %d, want 0 (not in sample metrics)", snapshot.Receivers["syslog/udp"].EventsIn)
	}
	if _, ok := snapshot.Pipelines["logs/syslog"]; !ok {
		t.Errorf("pipelines = %v, missing logs/syslog", snapshot.Pipelines)
	}

	configResp, err := client.Get("http://" + addr + "/config")
	if err != nil {
		t.Fatalf("GET /config: %v", err)
	}
	defer configResp.Body.Close()
	var configBody map[string]any
	if err := json.NewDecoder(configResp.Body).Decode(&configBody); err != nil {
		t.Fatalf("decoding /config body: %v", err)
	}
	exporters := asMap(configBody["exporters"])
	s1 := asMap(exporters["splunk_hec/sentinelone"])
	if s1["token"] != "***redacted***" {
		t.Errorf("token = %v, want ***redacted***", s1["token"])
	}
}

func TestExtension_LogsGetAndDeleteEndToEnd(t *testing.T) {
	configPath := writeTempConfig(t, sampleYAML)
	metrics := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.Write([]byte(samplePromText))
	}))
	defer metrics.Close()

	cfg := &Config{Endpoint: "127.0.0.1:0", ConfigPath: configPath, MetricsURL: metrics.URL}
	ext := newStatusCfgExtension(cfg, zap.NewNop(), "test")
	if err := ext.Start(context.Background(), nil); err != nil {
		t.Fatalf("Start: %v", err)
	}
	defer ext.Shutdown(context.Background())

	ext.buffer.Push([]LogEntry{{Body: "hello"}})

	addr := ext.listenerAddr()
	client := &http.Client{Timeout: 2 * time.Second}

	getResp, err := client.Get("http://" + addr + "/logs")
	if err != nil {
		t.Fatalf("GET /logs: %v", err)
	}
	var entries []LogEntry
	json.NewDecoder(getResp.Body).Decode(&entries)
	getResp.Body.Close()
	if len(entries) != 1 {
		t.Fatalf("got %d entries before clear, want 1", len(entries))
	}

	delReq, _ := http.NewRequest(http.MethodDelete, "http://"+addr+"/logs", nil)
	delResp, err := client.Do(delReq)
	if err != nil {
		t.Fatalf("DELETE /logs: %v", err)
	}
	delResp.Body.Close()
	if delResp.StatusCode != http.StatusNoContent {
		t.Errorf("DELETE /logs status = %d, want 204", delResp.StatusCode)
	}

	getResp2, err := client.Get("http://" + addr + "/logs")
	if err != nil {
		t.Fatalf("GET /logs after clear: %v", err)
	}
	var afterClear []LogEntry
	json.NewDecoder(getResp2.Body).Decode(&afterClear)
	getResp2.Body.Close()
	if len(afterClear) != 0 {
		t.Fatalf("got %d entries after clear, want 0", len(afterClear))
	}

	putReq, _ := http.NewRequest(http.MethodPut, "http://"+addr+"/logs", nil)
	putResp, err := client.Do(putReq)
	if err != nil {
		t.Fatalf("PUT /logs: %v", err)
	}
	putResp.Body.Close()
	if putResp.StatusCode != http.StatusMethodNotAllowed {
		t.Errorf("PUT /logs status = %d, want 405", putResp.StatusCode)
	}
}
