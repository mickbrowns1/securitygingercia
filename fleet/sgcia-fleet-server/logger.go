package main

import (
	"context"
	"fmt"

	"go.uber.org/zap"
)

// zapOpampLogger adapts a *zap.Logger to opamp-go's client/types.Logger
// interface, which both the client and server packages use.
type zapOpampLogger struct {
	logger *zap.Logger
}

func (l zapOpampLogger) Debugf(_ context.Context, format string, v ...any) {
	l.logger.Debug(fmt.Sprintf(format, v...))
}

func (l zapOpampLogger) Errorf(_ context.Context, format string, v ...any) {
	l.logger.Error(fmt.Sprintf(format, v...))
}
