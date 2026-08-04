package logbufferexporter // import "github.com/mickbrowns1/securitygingercia/otelcol/exporters/logbufferexporter"

import (
	"context"

	"go.opentelemetry.io/collector/component"
	"go.opentelemetry.io/collector/exporter"
	"go.opentelemetry.io/collector/exporter/exporterhelper"
)

var componentType = component.MustNewType("logbuffer")

func NewFactory() exporter.Factory {
	return exporter.NewFactory(
		componentType,
		createDefaultConfig,
		exporter.WithLogs(createLogsExporter, component.StabilityLevelDevelopment),
	)
}

func createDefaultConfig() component.Config {
	return &Config{Endpoint: "http://127.0.0.1:7801"}
}

func createLogsExporter(ctx context.Context, set exporter.Settings, cfg component.Config) (exporter.Logs, error) {
	c := cfg.(*Config)
	e := newExporter(c, set)
	return exporterhelper.NewLogs(ctx, set, cfg, e.consumeLogs, exporterhelper.WithTimeout(exporterhelper.NewDefaultTimeoutConfig()))
}
