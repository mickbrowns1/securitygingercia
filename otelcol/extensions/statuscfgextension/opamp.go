package statuscfgextension // import "github.com/mickbrowns1/securitygingercia/otelcol/extensions/statuscfgextension"

import (
	"context"
	"crypto/rand"
	"encoding/json"
	"fmt"
	"net/http"
	"os"
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
func startOpampReporter(cfg *Config, logger *zap.Logger, endpoint, buildVersion string, snapshotFn func() (MetricsSnapshot, error)) (*opampReporter, error) {
	if cfg.FleetServerURL == "" {
		return nil, nil
	}

	hostname, _ := os.Hostname()
	instanceUID := randomInstanceUID()

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
		protobufs.AgentCapabilities_AgentCapabilities_ReportsHealth
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
		},
	})
	if err != nil {
		cancel()
		return nil, err
	}

	reporter := &opampReporter{client: c, cancel: cancel}
	go reporter.reportLoop(startCtx, logger, snapshotFn)
	return reporter, nil
}

func (r *opampReporter) reportLoop(ctx context.Context, logger *zap.Logger, snapshotFn func() (MetricsSnapshot, error)) {
	ticker := time.NewTicker(reportInterval)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			r.reportOnce(logger, snapshotFn)
		}
	}
}

func (r *opampReporter) reportOnce(logger *zap.Logger, snapshotFn func() (MetricsSnapshot, error)) {
	snapshot, err := snapshotFn()
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

// randomInstanceUID generates a 16-byte instance identifier. The OpAMP
// spec recommends UUID v7 (so IDs sort roughly by creation time); a
// process-lifetime random ID is a simpler stand-in for Phase 1, at the
// cost of the fleet server seeing a "new" agent after every restart
// instead of recognizing the same one reconnecting.
func randomInstanceUID() types.InstanceUid {
	var id types.InstanceUid
	_, _ = rand.Read(id[:])
	return id
}
