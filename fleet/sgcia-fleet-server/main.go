// Command sgcia-fleet-server is the central OpAMP server for sgcia fleet
// management: it accepts outbound OpAMP connections from sgcia-otelcol
// agents, tracks them in a SQLite inventory, and serves a small read-only
// web UI + REST API over that inventory. Phase 1 only -- no remote config
// push, no groups/tags, no binary rollout.
package main

import (
	"context"
	"flag"
	"log"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"

	"go.uber.org/zap"
)

func main() {
	var (
		listen      = flag.String("listen", "0.0.0.0:4320", "address the fleet server listens on (REST API, OpAMP, web UI all share this one endpoint)")
		dbPath      = flag.String("db", "sgcia-fleet.db", "path to the SQLite inventory database")
		tokenFl     = flag.String("token", os.Getenv("SGCIA_FLEET_TOKEN"), "shared bearer token agents must present to enroll -- empty disables auth (dev only)")
		packagesDir = flag.String("packages-dir", "./packages", "directory where uploaded agent binary packages are stored on disk (metadata lives in the SQLite db; the bytes live here)")
	)
	flag.Parse()

	if err := os.MkdirAll(*packagesDir, 0o755); err != nil {
		log.Fatalf("creating packages directory %q: %v", *packagesDir, err)
	}

	logger, err := zap.NewProduction()
	if err != nil {
		log.Fatalf("building logger: %v", err)
	}
	defer logger.Sync()

	token := normalizeToken(*tokenFl)
	if token == "" {
		logger.Warn("SGCIA_FLEET_TOKEN not set -- accepting unauthenticated agent connections, do not expose this endpoint publicly like this")
	}

	st, err := openStore(*dbPath)
	if err != nil {
		logger.Fatal("opening inventory store", zap.Error(err))
	}
	defer st.close()

	registry := newConnRegistry()

	mux := http.NewServeMux()
	newAPIHandlers(mux, st, registry, logger, *packagesDir, token)
	mux.Handle("/", webUIHandler())

	connContext, err := startOpampServer(mux, st, registry, logger, token)
	if err != nil {
		logger.Fatal("starting OpAMP server", zap.Error(err))
	}

	srv := &http.Server{
		Addr:        *listen,
		Handler:     mux,
		ConnContext: connContext,
	}

	go func() {
		logger.Info("sgcia-fleet-server listening", zap.String("addr", *listen), zap.String("opamp_path", "/v1/opamp"))
		if err := srv.ListenAndServe(); err != nil && err != http.ErrServerClosed {
			logger.Fatal("server error", zap.Error(err))
		}
	}()

	stop := make(chan os.Signal, 1)
	signal.Notify(stop, os.Interrupt, syscall.SIGTERM)
	<-stop

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if err := srv.Shutdown(ctx); err != nil {
		logger.Error("shutting down", zap.Error(err))
	}
}
