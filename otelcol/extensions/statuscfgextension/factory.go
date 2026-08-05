package statuscfgextension // import "github.com/mickbrowns1/securitygingercia/otelcol/extensions/statuscfgextension"

import (
	"context"

	"go.opentelemetry.io/collector/component"
	"go.opentelemetry.io/collector/extension"
)

var componentType = component.MustNewType("statuscfg")

func NewFactory() extension.Factory {
	return extension.NewFactory(
		componentType,
		createDefaultConfig,
		createExtension,
		component.StabilityLevelDevelopment,
	)
}

func createDefaultConfig() component.Config {
	return &Config{
		// Matches the Rust dashboard/editor's own long-standing default
		// (`sgcia dashboard`'s --status-addr flag) so pointing them at
		// this extension instead of the retired Rust status API needs no
		// code changes on their end, just running against this binary.
		Endpoint:   "127.0.0.1:7801",
		MetricsURL: "http://localhost:8888/metrics",
	}
}

func createExtension(_ context.Context, set extension.Settings, cfg component.Config) (extension.Extension, error) {
	return newStatusCfgExtension(cfg.(*Config), set.TelemetrySettings.Logger, set.BuildInfo.Version), nil
}
