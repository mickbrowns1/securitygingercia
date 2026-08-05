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
RUN dnf install -y golang git gcc make rust cargo && dnf clean all

WORKDIR /src
COPY . .

# The Go otelcol distribution -- OCB (the OpenTelemetry Collector Builder)
# already generated otelcol/dist/{main.go,go.mod,go.sum,components.go} (see
# builder-config.yaml's output_path), so a plain `go build` there reproduces
# the exact binary `ocb` would.
RUN cd otelcol/dist && go build -o /out/sgcia-otelcol .

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
