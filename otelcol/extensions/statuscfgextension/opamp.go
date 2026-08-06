package statuscfgextension // import "github.com/mickbrowns1/securitygingercia/otelcol/extensions/statuscfgextension"

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"time"

	"github.com/open-telemetry/opamp-go/client"
	"github.com/open-telemetry/opamp-go/client/types"
	"github.com/open-telemetry/opamp-go/protobufs"
	"go.uber.org/zap"
)

// snapshotCapability/snapshotMessageType must match sgcia-fleet-server's
// own constants (fleet/sgcia-fleet-server/opampserver.go) -- the two sides
// agree on this out-of-band since there's no shared Go module between them.
const (
	snapshotCapability  = "io.sgcia.snapshot"
	snapshotMessageType = "metrics_snapshot"
)

// reportInterval is how often the agent pushes a fresh health/metrics
// snapshot to the fleet server. Independent of the web UI's own polling
// intervals -- this is agent-to-server, not browser-to-agent.
const reportInterval = 15 * time.Second

// fleetReport is what actually goes out over OpAMP every reportInterval:
// the same MetricsSnapshot the local /status endpoint already serves,
// plus the structural topology graph the local /topology endpoint already
// serves (topology.go's buildTopology()). Embedding MetricsSnapshot
// flattens its fields to the top level, so the JSON shape is exactly
// {receivers, pipelines, exporters, started_at, uptime_seconds, topology}
// -- the fleet webui's Sankey view feeds this same object in as both of
// computeSankeyLayout's arguments (topology as the graph, the rest as the
// status), with no adaptation needed on either side.
type fleetReport struct {
	MetricsSnapshot
	Topology topologyGraph `json:"topology"`
}

// zapOpampLogger adapts a *zap.Logger to opamp-go's client/types.Logger.
type zapOpampLogger struct {
	logger *zap.Logger
}

func (l zapOpampLogger) Debugf(_ context.Context, format string, v ...any) {
	l.logger.Debug(fmt.Sprintf(format, v...))
}

func (l zapOpampLogger) Errorf(_ context.Context, format string, v ...any) {
	l.logger.Error(fmt.Sprintf(format, v...))
}

// opampReporter owns the OpAMP client connection to a fleet server, if
// configured, and periodically pushes this agent's health snapshot to it.
// A nil *opampReporter (returned when FleetServerURL is empty) means fleet
// reporting is simply off -- every method on it is a no-op via the guard
// in newOpampReporter's caller, not a nil-receiver trick.
type opampReporter struct {
	client client.OpAMPClient
	cancel context.CancelFunc
}

// startOpampReporter connects to cfg.FleetServerURL and begins periodic
// reporting. Returns (nil, nil) if fleet reporting isn't configured --
// this is the opt-in gate; nothing about a plain install changes.
func startOpampReporter(cfg *Config, logger *zap.Logger, endpoint, buildVersion string, reportFn func() (fleetReport, error)) (*opampReporter, error) {
	if cfg.FleetServerURL == "" {
		return nil, nil
	}

	hostname, _ := os.Hostname()
	instanceUID := loadOrCreateInstanceUID(cfg.FleetInstanceIDPath, logger)

	c := client.NewWebSocket(zapOpampLogger{logger: logger})

	if err := c.SetAgentDescription(&protobufs.AgentDescription{
		IdentifyingAttributes: []*protobufs.KeyValue{
			stringAttr("service.name", "io.sgcia.otelcol"),
			stringAttr("service.version", buildVersion),
			stringAttr("host.name", hostname),
		},
		NonIdentifyingAttributes: []*protobufs.KeyValue{
			stringAttr("sgcia.local_ui_addr", endpoint),
		},
	}); err != nil {
		return nil, err
	}
	if err := c.SetHealth(&protobufs.ComponentHealth{Healthy: true}); err != nil {
		return nil, err
	}
	if err := c.SetCustomCapabilities(&protobufs.CustomCapabilities{
		Capabilities: []string{snapshotCapability},
	}); err != nil {
		return nil, err
	}
	capabilities := protobufs.AgentCapabilities_AgentCapabilities_ReportsStatus |
		protobufs.AgentCapabilities_AgentCapabilities_ReportsHealth |
		protobufs.AgentCapabilities_AgentCapabilities_AcceptsRemoteConfig |
		protobufs.AgentCapabilities_AgentCapabilities_ReportsRemoteConfig
	if err := c.SetCapabilities(&capabilities); err != nil {
		return nil, err
	}

	header := http.Header{}
	if cfg.FleetToken != "" {
		header.Set("Authorization", "Bearer "+cfg.FleetToken)
	}

	startCtx, cancel := context.WithCancel(context.Background())
	err := c.Start(startCtx, types.StartSettings{
		OpAMPServerURL: cfg.FleetServerURL,
		InstanceUid:    instanceUID,
		Header:         header,
		Callbacks: types.Callbacks{
			OnConnect: func(_ context.Context) {
				logger.Info("connected to fleet server", zap.String("url", cfg.FleetServerURL))
			},
			OnConnectFailed: func(_ context.Context, err error) {
				logger.Warn("fleet server connection failed", zap.Error(err))
			},
			OnError: func(_ context.Context, err *protobufs.ServerErrorResponse) {
				logger.Warn("fleet server reported an error", zap.String("message", err.GetErrorMessage()))
			},
			OnMessage: func(_ context.Context, msg *types.MessageData) {
				if msg.RemoteConfig == nil {
					return
				}
				// Validation + the subprocess call can take a moment; the
				// client library's own docs recommend returning from
				// OnMessage quickly and doing this kind of work async. A
				// fresh context is used rather than the one OnMessage was
				// called with, since that one isn't guaranteed to outlive
				// OnMessage's own return.
				go handleRemoteConfig(c, cfg.ConfigPath, logger, msg.RemoteConfig)
			},
		},
	})
	if err != nil {
		cancel()
		return nil, err
	}

	reporter := &opampReporter{client: c, cancel: cancel}
	go reporter.reportLoop(startCtx, logger, reportFn)
	return reporter, nil
}

func (r *opampReporter) reportLoop(ctx context.Context, logger *zap.Logger, reportFn func() (fleetReport, error)) {
	ticker := time.NewTicker(reportInterval)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			r.reportOnce(logger, reportFn)
		}
	}
}

func (r *opampReporter) reportOnce(logger *zap.Logger, reportFn func() (fleetReport, error)) {
	snapshot, err := reportFn()
	if err != nil {
		if healthErr := r.client.SetHealth(&protobufs.ComponentHealth{Healthy: false, LastError: err.Error()}); healthErr != nil {
			logger.Warn("setting unhealthy status", zap.Error(healthErr))
		}
		return
	}
	if err := r.client.SetHealth(&protobufs.ComponentHealth{Healthy: true}); err != nil {
		logger.Warn("setting health", zap.Error(err))
	}

	data, err := json.Marshal(snapshot)
	if err != nil {
		logger.Warn("marshaling snapshot for fleet report", zap.Error(err))
		return
	}
	if _, err := r.client.SendCustomMessage(&protobufs.CustomMessage{
		Capability: snapshotCapability,
		Type:       snapshotMessageType,
		Data:       data,
	}); err != nil {
		logger.Warn("sending snapshot to fleet server", zap.Error(err))
	}
}

// validateTimeout bounds the self-validate subprocess -- validate is fast;
// this only guards against a hang.
const validateTimeout = 30 * time.Second

// restartDelay gives SetRemoteConfigStatus(APPLIED) a moment to actually
// flush over the still-open WebSocket before the self-SIGTERM below tears
// the connection down.
const restartDelay = 2 * time.Second

// handleRemoteConfig implements Phase 2's core safety property: a pushed
// config is written to a temp file and validated against this same binary
// (`sgcia-otelcol validate`) before it ever touches the live config file.
// Only a config that validates gets written (atomically) and triggers a
// restart; anything else is reported back as failed and the running
// service is never touched.
func handleRemoteConfig(c client.OpAMPClient, configPath string, logger *zap.Logger, remoteCfg *protobufs.AgentRemoteConfig) {
	hash := remoteCfg.GetConfigHash()
	file := remoteCfg.GetConfig().GetConfigMap()[""]
	if file == nil {
		reportConfigStatus(c, logger, hash, false, "no config file found in the pushed ConfigMap (expected a single entry keyed by an empty string)")
		return
	}

	self, err := os.Executable()
	if err != nil {
		reportConfigStatus(c, logger, hash, false, "locating own binary to validate against: "+err.Error())
		return
	}

	if err := validateAndApply(configPath, self, file.GetBody()); err != nil {
		logger.Warn("rejecting pushed config", zap.Error(err))
		reportConfigStatus(c, logger, hash, false, err.Error())
		return
	}

	logger.Info("applied pushed config, restarting to pick it up")
	reportConfigStatus(c, logger, hash, true, "")
	time.AfterFunc(restartDelay, func() {
		_ = syscall.Kill(os.Getpid(), syscall.SIGTERM)
	})
}

// validateAndApply writes body to a temp file next to configPath (same
// directory, so the final rename is atomic), validates it by shelling out
// to `self validate --config file:<tmp>` (self is normally this same
// running binary's own path, via os.Executable() -- a parameter here so
// tests can point it at a fake script instead), and only on success
// renames it over the live config. Never partially applies: either the
// live file ends up byte-identical to a config that just validated, or it
// isn't touched at all.
func validateAndApply(configPath, self string, body []byte) error {
	tmpPath := configPath + ".tmp"
	if err := writeFileSynced(tmpPath, body); err != nil {
		return fmt.Errorf("writing candidate config: %w", err)
	}
	defer os.Remove(tmpPath) // no-op once renamed away on the success path

	ctx, cancel := context.WithTimeout(context.Background(), validateTimeout)
	defer cancel()
	cmd := exec.CommandContext(ctx, self, "validate", "--config", "file:"+tmpPath)
	var stdout, stderr strings.Builder
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		return errors.New(validateErrorMessage(self, stdout.String(), stderr.String(), err))
	}

	if err := os.Rename(tmpPath, configPath); err != nil {
		return fmt.Errorf("applying validated config: %w", err)
	}
	return nil
}

// validateErrorMessage picks the most useful available explanation for why
// `validate` failed: its stderr, falling back to stdout, falling back to a
// generic message naming the exit status -- mirroring the same fallback
// chain crates/collector/src/editor/model.rs's validate_with_binary uses
// for the equivalent Rust-side check.
func validateErrorMessage(self, stdout, stderr string, cmdErr error) string {
	if msg := strings.TrimSpace(stderr); msg != "" {
		return msg
	}
	if msg := strings.TrimSpace(stdout); msg != "" {
		return msg
	}
	return fmt.Sprintf("%s validate exited with %v", self, cmdErr)
}

func writeFileSynced(path string, data []byte) error {
	f, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0o640)
	if err != nil {
		return err
	}
	defer f.Close()
	if _, err := f.Write(data); err != nil {
		return err
	}
	return f.Sync()
}

func reportConfigStatus(c client.OpAMPClient, logger *zap.Logger, hash []byte, applied bool, errMsg string) {
	status := protobufs.RemoteConfigStatuses_RemoteConfigStatuses_FAILED
	if applied {
		status = protobufs.RemoteConfigStatuses_RemoteConfigStatuses_APPLIED
	}
	if err := c.SetRemoteConfigStatus(&protobufs.RemoteConfigStatus{
		LastRemoteConfigHash: hash,
		Status:               status,
		ErrorMessage:         errMsg,
	}); err != nil {
		logger.Warn("reporting remote config status to fleet server", zap.Error(err))
	}
}

func (r *opampReporter) stop() {
	if r == nil {
		return
	}
	r.cancel()
	stopCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if err := r.client.Stop(stopCtx); err != nil {
		// Best-effort on shutdown -- nothing useful to do with this error.
		_ = err
	}
}

func stringAttr(key, value string) *protobufs.KeyValue {
	return &protobufs.KeyValue{
		Key:   key,
		Value: &protobufs.AnyValue{Value: &protobufs.AnyValue_StringValue{StringValue: value}},
	}
}

// loadOrCreateInstanceUID returns a stable OpAMP instance ID for this
// agent, persisted at path (hex-encoded) so a restart reconnects as the
// same agent instead of enrolling as a new one every time. An empty path,
// or any failure to read/write it, falls back to a fresh random ID for
// this run only -- persistence is a nice-to-have, not something worth
// failing extension startup over.
func loadOrCreateInstanceUID(path string, logger *zap.Logger) types.InstanceUid {
	if path == "" {
		return randomInstanceUID()
	}

	if data, err := os.ReadFile(path); err == nil {
		if id, err := decodeInstanceUID(strings.TrimSpace(string(data))); err == nil {
			return id
		}
		logger.Warn("fleet instance ID file has unexpected contents, generating a new one", zap.String("path", path))
	}

	id := randomInstanceUID()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		logger.Warn("could not create directory to persist fleet instance ID -- this agent will look like a new enrollment on every restart", zap.String("path", path), zap.Error(err))
		return id
	}
	if err := os.WriteFile(path, []byte(hex.EncodeToString(id[:])), 0o600); err != nil {
		logger.Warn("could not persist fleet instance ID -- this agent will look like a new enrollment on every restart", zap.String("path", path), zap.Error(err))
	}
	return id
}

func decodeInstanceUID(s string) (types.InstanceUid, error) {
	var id types.InstanceUid
	b, err := hex.DecodeString(s)
	if err != nil || len(b) != len(id) {
		return id, errors.New("invalid instance UID encoding")
	}
	copy(id[:], b)
	return id, nil
}

// randomInstanceUID generates a fresh 16-byte instance identifier. The
// OpAMP spec recommends UUID v7 (so IDs sort roughly by creation time);
// plain random bytes are good enough here since nothing in this project
// relies on that ordering property.
func randomInstanceUID() types.InstanceUid {
	var id types.InstanceUid
	_, _ = rand.Read(id[:])
	return id
}
