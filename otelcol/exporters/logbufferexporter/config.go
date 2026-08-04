package logbufferexporter // import "github.com/mickbrowns1/securitygingercia/otelcol/exporters/logbufferexporter"

import "errors"

// Config configures the logbuffer exporter. It is local to this
// distribution (not upstreamed) -- it POSTs converted log records to the
// statuscfg extension's own HTTP server over loopback, the same way
// splunk_hec/dataset POST to their real destinations, so statuscfg can
// serve a small rolling window of recent events at GET /logs for the
// web UI. No Go-level dependency on statuscfgextension at all -- just a
// plain JSON-over-HTTP wire contract, like any other exporter.
type Config struct {
	// Endpoint is the statuscfg extension's own address in this same
	// collector, e.g. "http://127.0.0.1:7801" (matching its `endpoint`
	// field). If statuscfg isn't running (not in service.extensions, or
	// a different address), this exporter's sends simply fail like any
	// other unreachable destination -- it has no other effect on the
	// pipeline.
	Endpoint string `mapstructure:"endpoint"`
}

func (c *Config) Validate() error {
	if c.Endpoint == "" {
		return errors.New(`requires a non-empty "endpoint"`)
	}
	return nil
}
