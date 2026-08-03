# Security Ginger Collect It All (sgcia)

A syslog/flat-file/Windows-Event-Log collector built **on top of the
[OpenTelemetry Collector](https://github.com/open-telemetry/opentelemetry-collector-contrib)**,
plus a small companion terminal UI for editing its config and watching it
run. Two binaries:

- **`sgcia-otelcol`** -- the actual collector engine. A custom OpenTelemetry
  Collector distribution assembled by the official
  [OCB (OpenTelemetry Collector Builder)](https://github.com/open-telemetry/opentelemetry-collector/tree/main/cmd/builder)
  tool from upstream `opentelemetry-collector-contrib` components (pinned
  by version in [`otelcol/builder-config.yaml`](otelcol/builder-config.yaml))
  plus one small local extension we wrote ourselves
  ([`otelcol/extensions/statuscfgextension`](otelcol/extensions/statuscfgextension)).
  Bumping a security patch later is editing version strings in one YAML
  file and rebuilding -- there's no forked/vendored copy of contrib to
  maintain. Ships with:
  - **Receivers**: `syslog` (UDP and/or TCP, RFC 3164/5424, RFC 6587
    octet-counting framing), `file_log` (glob-based file tailing, like
    `tail -f`), `windows_event_log` (Windows only at runtime).
  - **Inline parsing**: each receiver owns its own `operators:` list (the
    `pkg/stanza` vocabulary: regex/JSON/key-value parsing, severity and
    timestamp parsing, field add/remove/copy/move) applied to every event
    before export.
  - **Exporters**: `splunk_hec` (works against both SentinelOne
    DataPipeline and generic Splunk HEC endpoints) and `debug` (prints
    events to the terminal, for testing a pipeline before wiring up a
    real destination).
- **`sgcia`** -- the Rust companion. Two subcommands:
  - **`sgcia dashboard`**: a terminal UI that polls a running
    `sgcia-otelcol` process's `/status` endpoint and shows
    per-receiver/pipeline/exporter throughput and errors.
  - **`sgcia edit`**: a terminal UI for browsing, adding, editing, and
    removing receivers/exporters/extensions/pipelines (and each
    receiver's inline operators) in a YAML config file, validated against
    the real `sgcia-otelcol validate` on every save.

## Contents

- [Installing](#installing)
- [Configuring](#configuring)
- [Using sgcia](#using-sgcia)
- [Running as a systemd service (Linux)](#running-as-a-systemd-service-linux)
- [The status endpoint](#the-status-endpoint)
- [Development](#development)

## Installing

You're building two independent binaries here: `sgcia-otelcol` (Go) and
`sgcia` (Rust). Neither depends on the other at build time.

### 1. Get the code

```bash
git clone https://github.com/mickbrowns1/securitygingercia.git
cd securitygingercia
```

### 2. Build `sgcia-otelcol` (the collector engine)

**Prerequisite: Go.** Install from
[go.dev/doc/install](https://go.dev/doc/install) if you don't have it
(`go version` to check). The OCB tool itself will pull a newer Go
toolchain automatically via Go's own toolchain-management mechanism if
your installed version is older than it needs -- you don't need to
pre-empt that.

Install the builder tool once, then run it against this repo's manifest:

```bash
go install go.opentelemetry.io/collector/cmd/builder@latest
cd otelcol
"$(go env GOPATH)/bin/builder" --config builder-config.yaml
```

This downloads the pinned `opentelemetry-collector-contrib` receiver/
exporter/extension versions from `builder-config.yaml`, generates a small
`main.go`/`go.mod` under `otelcol/dist/`, and compiles the binary there:
`otelcol/dist/sgcia-otelcol`.

**Bumping a security patch later**: edit the `v0.x.0` version strings in
`otelcol/builder-config.yaml` to the new contrib release, then re-run the
same `builder --config builder-config.yaml` command from inside `otelcol/`.

### 3. Build `sgcia` (the dashboard/editor)

**Prerequisite: a C linker/compiler**, which Rust needs to link the final
binary regardless of whether the project itself has any C code -- a fresh
Ubuntu/Debian box has none installed by default and fails with
``error: linker `cc` not found`` on the very first `cargo build`
otherwise.

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

Then install a Rust toolchain, if you don't have one (same on every
platform above). Run this as your normal user, **not** with `sudo` --
`sudo sh ...` here would install Rust for the `root` account instead of
yours, leaving you with a broken, split install once later steps run as
your normal user and can't find it.

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
```

Build, from the repo root (not `otelcol/` -- this is a separate Cargo
workspace at the top level):

```bash
cargo build --release
```

If this is the first thing you're compiling on the machine and it fails
with a linker error, go back to the prerequisite above -- that's what it
means.

The binary lands at `target/release/sgcia` (macOS/Linux) or
`target\release\sgcia.exe` (Windows).

### 4. Install both binaries

Right after building, each program exists as a file, but your computer
doesn't yet know it's a command you can type by name -- it only knows how
to find it if you tell it the exact location. Installing just means:
copy each file into a folder your computer already checks automatically.

```bash
sudo install -m 755 otelcol/dist/sgcia-otelcol /usr/local/bin/sgcia-otelcol
sudo install -m 755 target/release/sgcia /usr/local/bin/sgcia
```

`sudo` will ask for **your own login password** (not a special admin
password) -- nothing appears on screen while you type it, that's normal.

Check both worked:

```bash
sgcia-otelcol --version
sgcia --version
```

If either says `command not found`, either the file wasn't actually
built (re-check step 2 or 3's output for errors), or your terminal cached
its list of known commands before the install -- open a **brand new
terminal window** (or run `hash -r`) and try again.

Finally, create the two folders these binaries expect to use, for the
config file and working data (checkpoints/bookmarks via the
`file_storage` extension):

```bash
sudo mkdir -p /etc/sgcia /var/lib/sgcia
sudo chown "$USER" /etc/sgcia /var/lib/sgcia
```

The `chown` makes both folders writable by you directly, so you can run
`sgcia edit`/`sgcia-otelcol` as yourself without `sudo` while testing --
running as a **systemd service** under its own dedicated user later (see
below) only needs *read* access to the config, which a normal `chown`ed
file still allows.

## Configuring

Config is a single YAML file in real OpenTelemetry Collector shape:
top-level `receivers`, `exporters`, and `extensions` (each an id-keyed
map), plus `service.pipelines` wiring named receivers/exporters together.
[`otelcol/config/example.yaml`](otelcol/config/example.yaml) is a
complete, documented reference covering every receiver, exporter, and
extension type this distribution ships, including the inline `operators:`
vocabulary.

Two ways to build your own:

- **Interactively**: `sgcia edit --config /etc/sgcia/config.yaml` -- see
  [Using sgcia](#using-sgcia) below for the keybindings. Must be run from
  an actual interactive terminal (SSH session, local shell); it won't
  work piped through something non-interactive like a CI job.
- **By hand**: copy `otelcol/config/example.yaml` and edit the YAML
  directly, then `sgcia-otelcol validate --config file:/etc/sgcia/config.yaml`
  to check it.

### Secrets

HEC tokens are referenced in the config as `${VAR_NAME}` (e.g. `token:
${S1_HEC_TOKEN}`) and substituted from the process environment at load
time (via the collector's built-in `env` provider) -- the literal
`${...}` stays in the YAML file on disk, so the config itself never
contains a real secret. Two ways to supply the real value:

**Option A: a quick terminal test.** Export the variable in the same
shell session before running the collector:

```bash
export S1_HEC_TOKEN="your-sentinelone-token"
sgcia-otelcol --config file:/etc/sgcia/config.yaml
```

This only lasts for that terminal session -- close it and you'd need to
export again.

**Option B: a real deployment via systemd.** systemd reads environment
variables for the service from `/etc/sgcia/sgcia.env` (via
`EnvironmentFile=` in the unit -- see
[Running as a systemd service](#running-as-a-systemd-service-linux)
below):

```bash
sudo cp packaging/systemd/sgcia.env.example /etc/sgcia/sgcia.env
sudo "$EDITOR" /etc/sgcia/sgcia.env   # fill in your real tokens
sudo chmod 600 /etc/sgcia/sgcia.env   # secrets file, keep it non-world-readable
```

### Privileged ports

Standard syslog ports (UDP/TCP 514, TCP 601) are below 1024, so binding
them as a non-root user needs an explicit capability grant. See
[Running as a systemd service](#running-as-a-systemd-service-linux) below,
or point `udp.listen_address`/`tcp.listen_address` at an unprivileged
port (e.g. `0.0.0.0:5514`) and have your network/firewall layer
forward/NAT 514 to it instead.

## Using sgcia

```
sgcia-otelcol --config file:x.yaml                # run the collector
sgcia-otelcol validate --config file:x.yaml        # validate config, then exit
sgcia edit --config x.yaml                         # interactive config editor
sgcia dashboard [--status-addr 127.0.0.1:7801]     # live monitoring dashboard
```

### `sgcia-otelcol validate` -- validate a config

```console
$ export S1_HEC_TOKEN=...
$ sgcia-otelcol validate --config file:otelcol/config/example.yaml
$ echo $?
0
```

Catches structural problems (a pipeline referencing a receiver/exporter
that doesn't exist, an empty required list) and per-component validation
(bad regex, unparseable listen address, invalid URL, etc.) before you
ever try to run it. Prints nothing and exits 0 on success; prints the
specific problem and exits non-zero otherwise.

### `sgcia-otelcol` -- run the collector

```bash
export S1_HEC_TOKEN=...           # whatever your config references
sgcia-otelcol --config file:/etc/sgcia/config.yaml
```

Runs in the foreground, logging to stdout, until it receives `Ctrl-C`
(SIGINT) or SIGTERM, at which point it stops accepting new input, drains
whatever's already in flight through each pipeline, and exits cleanly.
The `statuscfg` extension in your config (see
[`otelcol/config/example.yaml`](otelcol/config/example.yaml)) is what
`sgcia dashboard`/`sgcia edit` talk to -- there's no separate CLI flag for
it, it's configured like any other extension.

### `sgcia dashboard` -- live monitoring

```bash
sgcia dashboard --status-addr 127.0.0.1:7801
```

Polls `GET /status` on the given address once a second and renders three
tables: receivers (events in), pipelines (in/out/dropped), and exporters
(batches sent/retried/failed, last error). If the connection drops, it
shows a red banner but keeps the last good snapshot on screen rather than
blanking. Press `q` or `Esc` to quit. `127.0.0.1:7801` is the default for
both this flag and the `statuscfg` extension's own `endpoint`, so with the
example config as-is, no flags are needed at all.

### `sgcia edit` -- interactive config editor

```bash
sgcia edit --config /etc/sgcia/config.yaml
```

Works entirely offline against the YAML file -- it never talks to a
running `sgcia-otelcol` process. Changes only take effect once you
restart the collector. Every save shells out to the real `sgcia-otelcol
validate` (so it must be installed and on `PATH`, or pointed at via the
`SGCIA_OTELCOL_BIN` environment variable) -- there's no separate
Rust-side validator to drift out of sync with the real thing.

Every screen shows help as you go: picking a type shows a one-line
description of what it does, and editing a component shows a plain-
English explanation (with an example value) for whichever field is
currently highlighted, at the bottom of the screen.

| Key | Action |
|---|---|
| `Tab` / `Shift+Tab` | Switch between Receivers / Exporters / Extensions / Pipelines tabs |
| `Up` / `Down` | Move selection within the current tab |
| `Enter` | Edit the selected item |
| `a` | Add a new item (pick a type, name it, then edit its fields) |
| `d` / `Delete` | Remove the selected item (asks for confirmation if a pipeline still references it) |
| `s` | Validate and save to disk |
| `q` / `Esc` | Quit |

While editing a component's fields: `Tab`/`Shift+Tab` moves between
fields, `Left`/`Right` cycles enum-type and true/false fields, any other
key types into the focused text field, `Esc` discards changes to that
component. A receiver's `operators` field is different: pressing `Enter`
on it opens a sub-screen for managing that receiver's inline parsing
chain (same `a`/`Enter`/`d`/`Esc` keys, one level deeper) instead of
submitting the form -- `Tab` off of it first if you meant to save the
whole component instead.

#### Walkthrough: build a minimal working config from scratch

This builds a pipeline that tails a log file and just prints what it
finds to your screen (no real destination yet) -- the fastest way to see
the whole tool work end to end before wiring up a real HEC endpoint.

1. Run `sgcia edit --config test.yaml` (a new file -- it doesn't need to
   exist yet).
2. You're on the **Receivers** tab, and it's empty. Press `a` to add one.
3. A list of receiver types appears, with a description of the
   highlighted one at the bottom. Use `Up`/`Down` to look them over, then
   press `Enter` on **file_log** (tails a file from disk).
4. It asks for an id -- type `file_log/test` and press `Enter`.
5. You're now editing its fields. Press `Tab` to move between them and
   read the help line at the bottom for each one:
   - `include`: type a real file on your machine, e.g. `/var/log/syslog`
     (Linux) or `/var/log/system.log` (macOS)
   - leave everything else at its default
   Press `Enter` to save this component (back to the Receivers list).
6. Press `Tab` to reach the **Exporters** tab, then `a` to add one. Pick
   **debug** (prints events to the screen, no setup needed) and name it
   `debug`. Press `Enter` on its field list to confirm it.
7. Press `Tab` twice more to reach the **Pipelines** tab (tabs go
   Receivers → Exporters → Extensions → Pipelines), then `a` to add one.
   Name it `test`.
8. On its field list: set `receivers` to `file_log/test` and `exporters`
   to `debug`. Press `Enter` to save.
9. Press `s` to validate and save the whole file. The status line at the
   bottom will say `saved test.yaml` if everything's valid, or explain
   what's wrong if not.
10. Press `q` to quit, then try it: `sgcia-otelcol --config file:test.yaml`
    -- you should see log lines print to your terminal as new lines get
    written to the file you picked in step 5.

From there, add `operators` to the receiver (see
[`otelcol/config/example.yaml`](otelcol/config/example.yaml) for real
examples of `regex_parser`/`key_value_parser`/`time_parser` parsing) and
swap the `debug` exporter for a real `splunk_hec` one once you're ready.

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
sudo "$EDITOR" /etc/sgcia/config.yaml # your actual config (start from otelcol/config/example.yaml)

sudo systemctl daemon-reload
sudo systemctl enable --now sgcia
systemctl status sgcia
journalctl -u sgcia -f
```

The unit grants `CAP_NET_BIND_SERVICE` so the non-root `sgcia` user can
bind privileged ports (514/601) -- remove that line if every
`listen_address` in your config is unprivileged (>= 1024). It also sets
`ProtectSystem=strict`, so if your config's `file_storage` extension's
`directory` points outside `/var/lib/sgcia`, either move it under there
or add the real directory to `ReadWritePaths=` in the unit.

Once it's running as a service, use `sgcia dashboard` and `sgcia edit`
over an actual SSH session to the box (not through `systemctl` or a
script -- both are interactive terminal UIs):

```bash
sgcia dashboard --status-addr 127.0.0.1:7801
sgcia edit --config /etc/sgcia/config.yaml   # then `sudo systemctl restart sgcia` to apply
```

## The status endpoint

The `statuscfg` extension (a small local addition, not part of upstream
contrib -- see
[`otelcol/extensions/statuscfgextension`](otelcol/extensions/statuscfgextension))
serves two read-only, unauthenticated endpoints on the address set by its
`endpoint` field (bind it to loopback unless you have another way to
restrict access -- there's no auth, and the loopback binding is the
security boundary):

- `GET /status` -- a JSON metrics snapshot: `started_at`,
  `uptime_seconds`, and per-receiver/pipeline/exporter counters, derived
  from the collector's own internal Prometheus telemetry (see the
  extension's `metrics_url` field). This is what `sgcia dashboard` polls;
  useful directly too (`curl` it from a monitoring script, feed it to
  your own dashboard, etc).
- `GET /config` -- the same config file passed to `sgcia-otelcol`'s own
  `--config` flag, re-read and served as JSON, with any field named
  `token`, `password`, `secret`, `api_key`, or `apikey` replaced with
  `"***redacted***"` at any nesting depth.

```console
$ curl -s http://127.0.0.1:7801/status | jq .
{
  "started_at": "2026-08-03T12:51:24.878639-04:00",
  "uptime_seconds": 19,
  "receivers": { "syslog/tcp": { "events_in": 1 } },
  "pipelines": {
    "logs/syslog": { "events_in": 1, "events_out": 1, "events_dropped": 0 }
  },
  "exporters": {
    "splunk_hec/sentinelone": {
      "events_in": 1, "batches_sent": 1, "batches_failed": 0,
      "retries": 0, "last_error": null
    }
  }
}
```

`batches_sent`/`batches_failed`/`retries`/`last_error` are a best-effort
approximation derived from the collector's own record-count telemetry
(OTel doesn't expose a per-pipeline metric or a literal "last error
message" at the default telemetry level) -- see the comments in
`otelcol/extensions/statuscfgextension/extension.go` for exactly what's
approximated and why.

## Development

Two independent things to build/test, matching the two binaries above.

**The collector engine** (`otelcol/`, Go -- `extensions/statuscfgextension`
is its own Go module, independent of the generated `dist/` module):

```bash
(cd otelcol/extensions/statuscfgextension && go test ./...)

cd otelcol
"$(go env GOPATH)/bin/builder" --config builder-config.yaml
./dist/sgcia-otelcol validate --config file:config/example.yaml
```

**The dashboard/editor** (repo root, Rust):

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
```

`crates/collector/src/editor`'s own test suite includes an integration
test that runs the real `sgcia-otelcol validate` (skipped automatically,
not failed, if that binary hasn't been built yet at the conventional
`otelcol/dist/sgcia-otelcol` path or via `SGCIA_OTELCOL_BIN`) -- build the
Go side first if you want that coverage included.
