# Builds sgcia-otelcol (the OTel Collector distribution, Go) and sgcia (the
# companion dashboard/edit TUI, Rust) from this repo. Lives at the repo root
# so a plain git-context build works for any consumer, e.g.:
#
#   docker build https://github.com/mickbrowns1/securitygingercia.git#main
#
# Consumers (e.g. StrongIsland's docker-compose.yml) bind-mount their own
# collector config at /etc/sgcia/config.yaml at runtime -- this image ships
# only the binaries, no config baked in.

FROM fedora:latest AS builder

# rust/cargo from Fedora's own repos rather than rustup -- some networks'
# TLS-interception proxies break rustup's curl|sh bootstrap (cert
# verification fails against a fresh container with no corporate root CA
# installed), and Fedora ships a recent-enough rust/cargo directly.
RUN dnf install -y golang git gcc make rust cargo nodejs npm && dnf clean all

WORKDIR /src
COPY . .

# The web UI (React, webui-react/) has to be built to dist/ *before* the
# ocb step below -- otelcol/extensions/statuscfgextension/webui.go embeds
# that dist/ output via go:embed, so it must exist on disk when `go build`
# (inside ocb) runs, same ordering install.sh enforces.
RUN cd otelcol/extensions/statuscfgextension/webui-react && npm ci && npm run build

# otelcol/dist/ (main.go, go.mod, go.sum, components.go, and the compiled
# binary itself) is gitignored -- OCB (the OpenTelemetry Collector Builder)
# regenerates it from otelcol/builder-config.yaml, so it isn't in this
# checkout and has to be built fresh here. Same recipe as install.sh/
# MANUAL.md's manual build step, including the GOTOOLCHAIN pin -- without
# it, a fresh go.mod's toolchain directive can trigger Go's automatic
# toolchain download, which fails on some arch/network combinations (see
# README.md's Troubleshooting section, "toolchain not available").
RUN go install go.opentelemetry.io/collector/cmd/builder@latest
RUN mkdir -p /out && cd otelcol \
    && GOTOOLCHAIN=go1.25.12 "$(go env GOPATH)/bin/builder" --config builder-config.yaml \
    && cp dist/sgcia-otelcol /out/sgcia-otelcol

# The Rust companion (dashboard/edit TUI) -- workspace build, release profile.
RUN cargo build --release && cp target/release/sgcia /out/sgcia

FROM fedora:latest
WORKDIR /app

COPY --from=builder /out/sgcia-otelcol /usr/local/bin/sgcia-otelcol
COPY --from=builder /out/sgcia /usr/local/bin/sgcia

# 514/udp + 601/tcp: the default syslog receiver ports (see the example
# configs under otelcol/config/). 7801 (statuscfg's /status + web UI, also
# what `sgcia dashboard`/`sgcia edit` talk to) and 13133 (health_check) are
# not exposed by default -- publish them explicitly if a consumer needs to.
EXPOSE 514/udp 601/tcp

CMD ["sgcia-otelcol", "--config", "file:/etc/sgcia/config.yaml"]
