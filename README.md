# Security Ginger Collect It All (sgcia)

A single static-ish binary log collector, in the spirit of the OpenTelemetry
Collector but purpose-built and self-contained:

- **Receivers**: syslog (UDP + TCP, RFC 3164 and RFC 5424, RFC 6587
  octet-counting and non-transparent TCP framing), flat-file tailing
  (glob discovery, rotation-aware, checkpointed), Windows Event Log
  (`EvtSubscribe`-based, Windows-only at runtime).
- **Parsing pipeline**: a configurable chain of operators (regex, JSON,
  key=value, severity mapping, timestamp parsing, field add/remove/copy/
  move/rename) applied to every event before export.
- **Exporters**: SentinelOne DataPipeline HEC and generic Splunk HEC, both
  with batching, retry/backoff, and newline-delimited framing.
- **Live dashboard** (`sgcia dashboard`): a terminal UI that polls a
  running collector's local status API and shows per-receiver/pipeline/
  exporter throughput, parse errors, and export retries/failures.
- **Config editor** (`sgcia edit`): a terminal UI for browsing, adding,
  editing, and removing receivers/operators/exporters/pipelines in a YAML
  config file, with validation before every save.

## Contents

- [Installing](#installing)
- [Configuring](#configuring)
- [Using sgcia](#using-sgcia)
- [Running as a systemd service (Linux)](#running-as-a-systemd-service-linux)
- [The status API](#the-status-api)
- [Development](#development)

## Installing

### Build from source

#### 1. Prerequisites

You need a C linker/compiler installed **before** building -- Rust needs
one to link the final binary (and even build scripts, which run as their
own compiled executable), regardless of whether the project itself has
any C code. A fresh Ubuntu/Debian box, in particular, has none installed
by default and will fail with ``error: linker `cc` not found`` on the
very first `cargo build` otherwise.

```bash
# Debian / Ubuntu
sudo apt update && sudo apt install -y build-essential git

# Fedora / RHEL / CentOS
sudo dnf groupinstall -y "Development Tools"

# Arch
sudo pacman -S --needed base-devel git

# macOS (if `cc` isn't already present -- check with `xcode-select -p` first)
xcode-select --install
```

Then install a Rust toolchain, if you don't have one (this works the
same way on every platform above):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
```

#### 2. Get the code

```bash
git clone https://github.com/mickbrowns1/securitygingercia.git
cd securitygingercia
```

#### 3. Build

```bash
cargo build --release
```

If this is the first thing you're compiling on the machine and it fails
with a linker error, go back to step 1 -- that's what it means.

The binary lands at `target/release/sgcia` (macOS/Linux) or
`target\release\sgcia.exe` (Windows). It's been built and its automated
test suite run on **macOS** and **Linux**; the Windows Event Log receiver
compiles and type-checks for Windows but has not been run against a real
Windows host yet -- see the note in
[`crates/sg-receiver-winevtlog`](crates/sg-receiver-winevtlog) if you're
relying on it.

**Build on the machine you intend to run it on, rather than
cross-compiling.** This project pulls in `ring` (via `reqwest`/`rustls`
for TLS), which has C code that needs a matching cross-toolchain and
sysroot to cross-compile correctly -- native compilation sidesteps that
entirely.

If you specifically want a fully static Linux binary (e.g. for a minimal
container image), build against the musl target **on a Linux machine**:

```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl --bin sgcia
# binary: target/x86_64-unknown-linux-musl/release/sgcia
```

### Installing the binary (Linux/macOS)

```bash
sudo install -m 755 target/release/sgcia /usr/local/bin/sgcia
sudo mkdir -p /etc/sgcia /var/lib/sgcia
```

`/var/lib/sgcia` is a reasonable default place for file-tail checkpoints
and Windows Event Log bookmarks (see `checkpoint_file`/`bookmark_file` in
your config) -- sgcia creates the file itself, but the parent directory
should exist and be writable by whatever user runs it.

## Configuring

Config is a single YAML file with four top-level sections: `receivers`,
`operators`, `exporters`, and `service.pipelines` (which wires the other
three together by name). [`configs/example.yaml`](configs/example.yaml)
is a complete, documented reference covering every receiver, operator,
and exporter type.

Two ways to build your own:

- **Interactively**: `sgcia edit --config /etc/sgcia/config.yaml` -- see
  [Using sgcia](#using-sgcia) below for the keybindings. Must be run from
  an actual interactive terminal (SSH session, local shell); it won't
  work piped through something non-interactive like a CI job.
- **By hand**: copy `configs/example.yaml` and edit the YAML directly,
  then `sgcia check --config /etc/sgcia/config.yaml` to validate.

### Secrets

HEC tokens are referenced in the config as `${VAR_NAME}` (e.g. `token:
${S1_HEC_TOKEN}`) and substituted from the process environment at load
time -- the literal `${...}` stays in the YAML file on disk, so the
config itself never contains a real secret. Supply the actual value via
whatever environment sgcia runs under (an exported shell variable for a
quick test, or the `EnvironmentFile=` mechanism in the systemd unit below
for a real deployment).

### Privileged ports

Standard syslog ports (UDP/TCP 514, TCP 601) are below 1024, so binding
them as a non-root user needs an explicit capability grant. See
[Running as a systemd service](#running-as-a-systemd-service-linux) below,
or point `listen_address` at an unprivileged port (e.g. `0.0.0.0:5514`)
and have your network/firewall layer forward/NAT 514 to it instead.

## Using sgcia

```
sgcia run --config x.yaml [--status-addr 127.0.0.1:7801]   # run the collector
sgcia check --config x.yaml                                # validate config, print pipeline graph
sgcia edit --config x.yaml                                  # interactive config editor
sgcia dashboard [--status-addr 127.0.0.1:7801]              # live monitoring dashboard
```

### `sgcia check` -- validate a config

```console
$ export S1_HEC_TOKEN=... SPLUNK_HEC_TOKEN=...
$ sgcia check --config configs/example.yaml
Resolved pipeline graph:
  pipeline: logs/files
    receivers: filelog/app [filelog]
    operators: parse_timestamp [parse_timestamp]
    exporters: sentinelone_hec [sentinelone_hec]
  pipeline: logs/syslog
    receivers: syslog/udp [syslog], syslog/tcp [syslog]
    operators: parse_kv [parse_kv], extract_asa_fields [extract_asa_fields], map_severity [map_severity], parse_timestamp [parse_timestamp], add_datasource [add_datasource], move_message_to_body [move_message_to_body]
    exporters: sentinelone_hec [sentinelone_hec], splunk_hec [splunk_hec]
  pipeline: logs/windows
    receivers: windows_eventlog/security [windows_eventlog]
    operators: (none)
    exporters: sentinelone_hec [sentinelone_hec]

config OK
```

Catches structural problems (a pipeline referencing a receiver/exporter
that doesn't exist, an empty required list) and per-component validation
(bad regex, unparseable listen address, invalid URL, etc.) before you
ever try to run it.

### `sgcia run` -- run the collector

```bash
export S1_HEC_TOKEN=...           # whatever your config references
sgcia run --config /etc/sgcia/config.yaml --status-addr 127.0.0.1:7801
```

Runs in the foreground, logging to stdout, until it receives `Ctrl-C`
(SIGINT) or SIGTERM, at which point it stops accepting new input, drains
whatever's already in flight through each pipeline, and exits cleanly.
`--status-addr` is optional; omit it to skip starting the status API.

### `sgcia dashboard` -- live monitoring

```bash
sgcia dashboard --status-addr 127.0.0.1:7801
```

Polls `GET /status` on the given address once a second and renders three
tables: receivers (events in), pipelines (in/out/dropped/parse errors),
and exporters (batches sent/retried/failed, last error). If the
connection drops, it shows a red banner but keeps the last good snapshot
on screen rather than blanking. Press `q` or `Esc` to quit.

### `sgcia edit` -- interactive config editor

```bash
sgcia edit --config /etc/sgcia/config.yaml
```

Works entirely offline against the YAML file -- it never talks to a
running `sgcia run` process. Changes only take effect once you restart
the collector.

| Key | Action |
|---|---|
| `Tab` / `Shift+Tab` | Switch between Receivers / Operators / Exporters / Pipelines tabs |
| `Up` / `Down` | Move selection within the current tab |
| `Enter` | Edit the selected item |
| `a` | Add a new item (pick a type, name it, then edit its fields) |
| `d` / `Delete` | Remove the selected item (asks for confirmation if a pipeline still references it) |
| `s` | Validate and save to disk |
| `q` / `Esc` | Quit |

While editing a component's fields: `Tab`/`Shift+Tab` moves between
fields, `Left`/`Right` cycles enum-type fields (protocol, start_at,
etc.), any other key types into the focused text field, `Enter` saves
the form (back to the list), `Esc` discards changes to that component.

## Running as a systemd service (Linux)

A starter unit is at
[`packaging/systemd/sgcia.service`](packaging/systemd/sgcia.service).

```bash
# Dedicated non-root user the service runs as.
sudo useradd --system --home /var/lib/sgcia --shell /usr/sbin/nologin sgcia
sudo chown -R sgcia:sgcia /var/lib/sgcia

sudo cp packaging/systemd/sgcia.service /etc/systemd/system/sgcia.service
sudo cp packaging/systemd/sgcia.env.example /etc/sgcia/sgcia.env
sudo "$EDITOR" /etc/sgcia/sgcia.env   # fill in real HEC token(s)
sudo chown sgcia:sgcia /etc/sgcia/sgcia.env
sudo chmod 600 /etc/sgcia/sgcia.env   # secrets file, keep it non-world-readable
sudo "$EDITOR" /etc/sgcia/config.yaml # your actual config (start from configs/example.yaml)

sudo systemctl daemon-reload
sudo systemctl enable --now sgcia
systemctl status sgcia
journalctl -u sgcia -f
```

The unit grants `CAP_NET_BIND_SERVICE` so the non-root `sgcia` user can
bind privileged ports (514/601) -- remove that line if every
`listen_address` in your config is unprivileged (>= 1024). It also sets
`ProtectSystem=strict`, so if your `checkpoint_file`/`bookmark_file`
paths point outside `/var/lib/sgcia`, either move them under it or add
the real directory to `ReadWritePaths=` in the unit.

Once it's running as a service, use `sgcia dashboard` and `sgcia edit`
over an actual SSH session to the box (not through `systemctl` or a
script -- both are interactive terminal UIs):

```bash
sgcia dashboard --status-addr 127.0.0.1:7801
sgcia edit --config /etc/sgcia/config.yaml   # then `sudo systemctl restart sgcia` to apply
```

## The status API

When `sgcia run` is started with `--status-addr`, it serves two
read-only, unauthenticated endpoints on that address (bind it to
loopback unless you have another way to restrict access -- there's no
auth, and the loopback binding is the security boundary):

- `GET /status` -- a JSON metrics snapshot: `started_at`,
  `uptime_seconds`, and per-receiver/pipeline/exporter counters. This is
  what `sgcia dashboard` polls; useful directly too (`curl` it from a
  monitoring script, feed it to your own dashboard, etc).
- `GET /config` -- the resolved config as JSON, with any field named
  `token`, `password`, `secret`, `api_key`, or `apikey` replaced with
  `"***redacted***"` at any nesting depth.

```console
$ curl -s http://127.0.0.1:7801/status | jq .
{
  "started_at": "2026-07-28T20:24:24.891509Z",
  "uptime_seconds": 113,
  "receivers": { "filelog/smoke": { "events_in": 1 } },
  "pipelines": {
    "logs/smoke": {
      "events_in": 1, "events_out": 1, "events_dropped": 0,
      "events_dead_lettered": 0, "parse_errors": 0
    }
  },
  "exporters": {
    "debug_stdout": {
      "events_in": 1, "batches_sent": 1, "batches_failed": 0,
      "retries": 0, "last_error": null
    }
  }
}
```

## Development

```bash
cargo build --workspace
cargo test --workspace     # 100+ tests across every crate
cargo clippy --workspace --all-targets
```

Each crate under `crates/` is independently testable; see the module-level
doc comments (`//!` at the top of each `lib.rs`/`mod.rs`) for what each
one owns.
