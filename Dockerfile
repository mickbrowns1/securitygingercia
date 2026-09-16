# Builds sgcia-otelcol (the OTel Collector distribution, Go) and sgcia (the
# companion dashboard/edit TUI, Rust) from this repo. Lives at the repo root
# so a plain git-context build works for any consumer, e.g.:
#
#   docker build https://github.com/mickbrowns1/securitygingercia.git#main
#
# Consumers (e.g. StrongIsland's docker-compose.yml) bind-mount their own
# collector config at /etc/sgcia/config.yaml at runtime -- this image ships
# only the binaries, no config baked in.
#
# The final stage runs as a non-root, fixed-UID (10001) user -- anything
# bind-mounted in (config.yaml, or a file_storage extension's checkpoint
# directory) needs to be readable/writable by that UID, not just root.

FROM fedora:latest AS builder

# rust/cargo from Fedora's own repos rather than rustup -- some networks'
# TLS-interception proxies break rustup's curl|sh bootstrap (cert
# verification fails against a fresh container with no corporate root CA
# installed), and Fedora ships a recent-enough rust/cargo directly.
RUN dnf install -y golang git gcc make rust cargo nodejs npm && dnf clean all

# Pin npm to a known-good version rather than trusting whatever
# fedora:latest's nodejs/npm package happens to resolve to today -- npm has
# a longstanding, timing-sensitive "Exit handler never called!" bug
# (npm/cli#4028) that comes and goes across releases and is more
# reproducible under emulation/resource-constrained containers. Unpinned,
# a `dnf` update rolling fedora:latest forward to a regressed npm build
# could break this Dockerfile with no change to this repo's own code.
RUN npm install -g npm@10.9.8

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
#
# The builder tool itself is pinned to the SAME release line as the
# component versions in builder-config.yaml (v0.157.0), not @latest --
# @latest previously broke this build silently: OCB v0.161.0 bumped its own
# core go.opentelemetry.io/collector/{otelcol,service} dependency to
# require go >= 1.26, which the GOTOOLCHAIN pin below (go1.25.12) then
# rejected. Pinning both to the same v0.157.0 line keeps them in lockstep --
# bump both together, deliberately, when you bump builder-config.yaml.
RUN go install go.opentelemetry.io/collector/cmd/builder@v0.157.0
RUN mkdir -p /out && cd otelcol \
    && GOTOOLCHAIN=go1.25.12 "$(go env GOPATH)/bin/builder" --config builder-config.yaml \
    && cp dist/sgcia-otelcol /out/sgcia-otelcol

# The Rust companion (dashboard/edit TUI) -- workspace build, release profile.
RUN cargo build --release && cp target/release/sgcia /out/sgcia

FROM fedora:latest
WORKDIR /app

# Non-root, fixed-UID system user -- mirrors the systemd deployment's own
# sgcia:sgcia convention (packaging/systemd/sgcia.service, install.sh). A
# fixed UID/GID (rather than an auto-assigned one) keeps bind-mounted host
# directories' ownership predictable across rebuilds.
RUN groupadd --system --gid 10001 sgcia \
    && useradd --system --uid 10001 --gid sgcia --home-dir /var/lib/sgcia --shell /usr/sbin/nologin sgcia

COPY --from=builder /out/sgcia-otelcol /usr/local/bin/sgcia-otelcol
COPY --from=builder /out/sgcia /usr/local/bin/sgcia

# Docker analogue of the systemd unit's AmbientCapabilities=CAP_NET_BIND_SERVICE
# (see packaging/systemd/sgcia.service) -- grants just enough to bind the
# privileged syslog ports below without running as root. Applied here, in the
# final stage after the COPY above, not in the builder stage before it: a
# capability set on a file isn't reliably preserved across a multi-stage
# COPY, so setting it post-copy sidesteps that risk entirely.
RUN dnf install -y libcap && dnf clean all \
    && setcap 'cap_net_bind_service=+ep' /usr/local/bin/sgcia-otelcol

USER sgcia

# 514/udp + 601/tcp: the default syslog receiver ports (see the example
# configs under otelcol/config/). 7801 (statuscfg's /status + web UI, also
# what `sgcia dashboard`/`sgcia edit` talk to) and 13133 (health_check) are
# not exposed by default -- publish them explicitly if a consumer needs to.
EXPOSE 514/udp 601/tcp

CMD ["sgcia-otelcol", "--config", "file:/etc/sgcia/config.yaml"]
