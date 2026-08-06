package statuscfgextension // import "github.com/mickbrowns1/securitygingercia/otelcol/extensions/statuscfgextension"

import "errors"

// Config configures the statuscfg extension. It is local to this
// distribution (not upstreamed to opentelemetry-collector-contrib) and
// exists purely to give the Rust dashboard/editor TUIs a /status + /config
// HTTP surface shaped like the one the retired Rust collector engine used
// to serve directly.
type Config struct {
	// Endpoint is the address the extension's own HTTP server listens on.
	Endpoint string `mapstructure:"endpoint"`

	// ConfigPath is the same YAML file passed to the collector's own
	// --config flag at startup. There is no API for reading back the
	// collector's resolved config, so this extension re-reads that file
	// itself. Required.
	ConfigPath string `mapstructure:"config_path"`

	// MetricsURL is where the collector's own Prometheus-format internal
	// telemetry is exposed (service::telemetry::metrics in the same
	// config), scraped on every /status request and reshaped into the
	// dashboard's expected snapshot shape.
	MetricsURL string `mapstructure:"metrics_url"`

	// FleetServerURL is the OpAMP WebSocket endpoint of a central
	// sgcia-fleet-server (e.g. "ws://fleet.example.com:4320/v1/opamp").
	// Empty (the default) disables fleet reporting entirely -- this is an
	// explicit opt-in, not something every install takes on.
	FleetServerURL string `mapstructure:"fleet_server_url"`

	// FleetToken is sent as a bearer token when connecting to
	// FleetServerURL. Only meaningful if FleetServerURL is set.
	FleetToken string `mapstructure:"fleet_token"`

	// FleetInstanceIDPath is where this agent persists its OpAMP instance
	// ID across restarts, so the fleet server recognizes a restarted
	// agent as the same one instead of a fresh enrollment. Only
	// meaningful if FleetServerURL is set. Empty disables persistence (a
	// new random ID is generated every process start).
	FleetInstanceIDPath string `mapstructure:"fleet_instance_id_path"`
}

func (c *Config) Validate() error {
	if c.ConfigPath == "" {
		return errors.New(`requires a non-empty "config_path"`)
	}
	return nil
}
