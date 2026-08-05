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
    DataPipeline and generic Splunk HEC endpoints), `dataset` (SentinelOne
    Singularity Data Lake, formerly Scalyr/DataSet), and `debug` (prints
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

- [Deployment checklist](#deployment-checklist)
- [Installing](#installing)
- [Configuring](#configuring)
- [Using sgcia](#using-sgcia)
- [Running as a systemd service (Linux)](#running-as-a-systemd-service-linux)
- [Verifying the deployment](#verifying-the-deployment)
- [Upgrading](#upgrading)
- [Uninstalling](#uninstalling)
- [The status endpoint](#the-status-endpoint)
- [The web UI](#the-web-ui)
- [Troubleshooting](#troubleshooting)
- [Development](#development)
- [Manual installation (MANUAL.md)](MANUAL.md)

## Deployment checklist

Everything below in one ordered list, for a fresh production box. Each
step links to the detailed section if you need it.

1. [Install](#installing) -- one command builds and installs both
   binaries (or see [MANUAL.md](MANUAL.md) to build from source by hand).
2. [Write your config](#configuring) (`/etc/sgcia/config.yaml`), starting from [`otelcol/config/example.yaml`](otelcol/config/example.yaml) -- either by hand or with `sgcia edit`.
3. [Supply secrets](#secrets) via `/etc/sgcia/sgcia.env` (HEC tokens, `chmod 600`).
4. If you're listening on standard syslog ports (514/601): plan for [privileged ports](#privileged-ports) (the systemd unit already handles this) and open them in your [firewall](#firewall--network-access) if senders are on other hosts.
5. `sgcia-otelcol validate --config file:/etc/sgcia/config.yaml` -- confirm the config is valid *before* wiring up the service.
6. [Install and start the systemd service](#running-as-a-systemd-service-linux) (or run it directly in the foreground for a quick test).
7. [Verify it's actually working](#verifying-the-deployment) -- service is active, `/status` responds, a real test event makes it through end to end.
8. Point monitoring/alerting at [`GET /status`](#the-status-endpoint) if you want automated health checks beyond `systemctl status`.

Not on Linux, or not using systemd? Steps 1-5 and 7 are platform-agnostic
(see the [Windows](MANUAL.md#windows) notes in MANUAL.md); you'll just
run `sgcia-otelcol` under whatever process supervisor your platform uses
instead of step 6's systemd unit.

## Installing

One command handles it: installs git/Go/a C toolchain/Rust if they're
missing, builds both binaries, installs them to `/usr/local/bin`, and
creates `/etc/sgcia` + `/var/lib/sgcia` with a starter config copied in.
Works on Linux (Debian/Ubuntu, Fedora/RHEL/CentOS, Arch) and macOS:

```bash
curl -fsSL https://raw.githubusercontent.com/mickbrowns1/securitygingercia/main/install.sh | bash
```

(Or, if you've already cloned the repo: `./install.sh` from the repo
root.) It asks for your login password only for the specific steps that
need `sudo` (installing OS packages, copying into `/usr/local/bin`).

Not on Linux/macOS (see [Windows](MANUAL.md#windows)), or want to build
from source and see/control each step yourself? The full manual
process -- exact prerequisites, what each step does and why -- is in
[MANUAL.md](MANUAL.md).

Two things left before it's actually collecting anything for real:

### Configure your pipelines through the UI

```bash
sgcia edit --config /etc/sgcia/config.yaml
```

An interactive terminal UI for adding, editing, and removing receivers,
exporters, extensions, and pipelines, validated against the real
`sgcia-otelcol validate` on every save -- see
[Configuring](#configuring) below for the YAML shape it's editing and
[Using sgcia](#using-sgcia) for the full keybinding reference.

### Supply secrets via the env file

HEC tokens and similar are referenced in the config as `${VAR_NAME}`
and substituted from the environment at load time -- never written into
the YAML file itself. For a real (systemd) deployment, that's one file:
`/etc/sgcia/sgcia.env`.

**If you used `install.sh`** (on a systemd Linux host), this file
already exists -- it creates and enables the service for you, including
this file with placeholder tokens. Just edit it:

```bash
sudo "$EDITOR" /etc/sgcia/sgcia.env   # fill in your real tokens
```

**Otherwise** (built manually via [MANUAL.md](MANUAL.md), or setting up
the systemd service yourself), create it from the example first:

```bash
sudo cp ~/securitygingercia/packaging/systemd/sgcia.env.example /etc/sgcia/sgcia.env  # adjust the path if you cloned elsewhere
sudo "$EDITOR" /etc/sgcia/sgcia.env   # fill in your real tokens
sudo chown sgcia:sgcia /etc/sgcia/sgcia.env
sudo chmod 600 /etc/sgcia/sgcia.env   # secrets file, keep it non-world-readable
```

See [Secrets](#secrets) below for a quick-terminal-test alternative and
how this file gets read by the systemd unit.

## Configuring

Config is a single YAML file in real OpenTelemetry Collector shape. This
is the one thing to internalize before anything else here will make
sense:

```yaml
receivers:        # id-keyed map -- one entry per input source
  <type>/<name>:
    ...fields for that type...
    operators:    # optional inline parsing chain (all three receiver types support it)
      - type: <operator-type>
        ...fields for that operator...

exporters:        # id-keyed map -- one entry per destination
  <type>/<name>:
    ...fields for that type...

extensions:       # id-keyed map -- cross-cutting add-ons (storage, health, status)
  <type>:
    ...fields for that type...

service:
  extensions: [<extension-id>, ...]   # required to actually activate anything defined above -- see note below
  pipelines:
    logs/<name>:                      # "logs/" is required here, not a free choice -- see note below
      receivers: [<receiver-id>, ...]
      exporters: [<exporter-id>, ...]
```

Every `<type>/<name>` id follows the OTel convention: the part before the
first `/` selects which component type it is (e.g. `syslog`, `file_log`,
`splunk_hec`) -- there's no separate `type:` field, the id *is* the type
selector; the part after `/` is just a label you choose, used to tell two
instances of the same type apart (e.g. a `syslog/udp` and a `syslog/tcp`
receiver side by side) and to reference the component from
`service.pipelines`.

**Pipeline ids are the one place this pattern doesn't apply.** A
receiver/exporter/extension id's prefix can be any of that category's
real component types, and the part after `/` is just your own label. A
*pipeline* id's prefix instead has to be a recognized **signal name**
-- `logs`, `metrics`, or `traces` (this distro only ever uses `logs`,
since everything here is log pipelines) -- not a component type, and not
optional. Naming a pipeline just `test` instead of `logs/test` produces
a confusing `cannot unmarshal the configuration ... unknown pipeline
signal: "test"` error on save, since `sgcia-otelcol validate` only
checks each component's own fields and doesn't explain this rule.

**`service.extensions` is easy to forget and the failure is confusing
when you do**: defining something under `extensions:` only describes it;
nothing actually *runs* until its id is also listed in
`service.extensions`. A config missing that list still passes
`sgcia-otelcol validate` (validate only checks each component's own
fields, not whether it's wired in) but then fails at actual startup with
something like `storage extension 'file_storage' not found` the moment a
receiver tries to use it. If you're using `sgcia edit`, this is handled
for you automatically -- it derives `service.extensions` from whatever
you define under `extensions:` on every save, so you'll never see this.
Writing YAML by hand, you have to list it yourself.

A complete, minimal, runnable example -- tail one file, print events to
the terminal, no secrets or network ports needed:

```yaml
# Save as e.g. /etc/sgcia/config.yaml, then:
#   sgcia-otelcol validate --config file:/etc/sgcia/config.yaml
#   sgcia-otelcol --config file:/etc/sgcia/config.yaml

receivers:
  file_log/app:
    include: ["/var/log/myapp/*.log"]
    start_at: end
    operators:
      - type: add
        field: attributes.sourcetype
        value: myapp

exporters:
  debug:
    verbosity: detailed

service:
  pipelines:
    logs/app:
      receivers: [file_log/app]
      exporters: [debug]
```

### Available components

| Category | Type | What it does | Key fields |
|---|---|---|---|
| Receiver | `syslog` | Listens for syslog over UDP and/or TCP | `protocol`, `udp.listen_address`/`tcp.listen_address`, `enable_octet_counting` |
| Receiver | `file_log` | Tails files matching a glob, like `tail -f` | `include`, `exclude`, `start_at`, `storage` |
| Receiver | `windows_event_log` | Reads a Windows Event Log channel (**Windows only** -- fails to start on Linux/macOS, see [Windows](MANUAL.md#windows)). Left out of `example.yaml` for exactly that reason -- it's only in [`example-windows.yaml`](otelcol/config/example-windows.yaml) | `channel`, `query`, `start_at`, `storage` |
| Exporter | `splunk_hec` | Sends to a Splunk-compatible HEC endpoint, including SentinelOne DataPipeline | `endpoint`, `token`, `otel_attrs_to_hec_metadata.*` |
| Exporter | `dataset` | Sends to SentinelOne Singularity Data Lake (formerly Scalyr/DataSet). **Alpha stability upstream** | `dataset_url`, `api_key`, `server_host.*` |
| Exporter | `debug` | Prints events to the terminal -- for testing a pipeline before wiring up a real destination | `verbosity` |
| Exporter | `logbuffer` | Feeds the web UI's log viewer (see [The web UI](#the-web-ui)) -- not a real destination, loopback-only, safe to add to every logs pipeline | `endpoint` |
| Extension | `file_storage` | Persists a receiver's read position across restarts (referenced by a receiver's `storage` field) | `directory`, `create_directory` |
| Extension | `health_check` | Simple HTTP health-check endpoint for this collector process | `endpoint` |
| Extension | `statuscfg` | Serves `/status`, `/config`, `/topology`, `/logs`, and the web UI for `sgcia dashboard`/`sgcia edit`/a browser to poll | `endpoint`, `config_path`, `metrics_url` |

Every receiver's optional `operators:` list draws from the same
`pkg/stanza` vocabulary: `regex_parser`, `json_parser`,
`key_value_parser`, `severity_parser`, `time_parser` (parsers), and
`add`/`remove`/`copy`/`move` (field manipulation) -- see
[`otelcol/config/example.yaml`](otelcol/config/example.yaml) for a
complete, real example of each, including two syslog pipelines side by
side: an RFC 3164 one (modeling a Cisco ASA, with regex extraction and
severity mapping for its `%ASA-X-XXXXX:`-style messages) and an RFC
5424 one (modeling a modern Linux host via rsyslog, whose already-
structured envelope needs no regex at all) -- see [the RFC
3164/5424 mismatch entry](#troubleshooting) if you're not sure which
your own sender actually speaks.

#### Source templates

`sgcia edit`'s Receivers tab has a `T` key (alongside `a` for a bare
component) that browses a curated library of 18 source templates,
grouped by category -- network/security devices (Cisco ASA, Cisco
Catalyst/IOS, Cisco Meraki, Ubiquiti/UniFi, HAProxy, CEF), generic
(RFC 5424 syslog, plain file tailing, JSON logs, W3C/IIS extended logs,
nginx access logs), Windows (a DHCP server audit log, and Active
Directory via the Event Log), databases (MySQL, PostgreSQL, SQL
Server), and messaging/big data (Kafka, Hadoop) -- each producing a
ready-to-use receiver, including its `operators:` chain, from a couple
of typed parameters (a listen address, a file glob, a Windows channel)
instead of making you hand-write a `regex_parser` from scratch. Pick
one, name the new receiver, fill in its params, review the generated
YAML, then confirm to insert it -- same `sgcia-otelcol validate` on
save as anything else in this editor, so a template can't silently
produce an invalid config. Some (Cisco ASA, CEF, the W3C/nginx/DHCP/
SQL Server formats) match a fixed, documented message grammar exactly;
others (Cisco Meraki, HAProxy, MySQL/PostgreSQL/Kafka/Hadoop's log4j-
style formats) cover the common default but may need adjusting for
your specific export/configuration settings -- each template's own
description in the
picker says which.

`dataset` routes events differently than `splunk_hec` -- it has no
config-driven attribute-to-metadata mapping at all (no
`otel_attrs_to_hec_metadata` equivalent). The only routing metadata it
reads is a literal `serverHost` attribute, checked in order: the event's
own `serverHost` attribute, then the resource's `serverHost`/`host.name`,
then `server_host.server_host` in the exporter's own config, then (if
`server_host.use_hostname` is `true`) this collector's own OS hostname.
To set it per-event, add a plain `serverHost` attribute via an `add`
operator on the receiver, same as `sourcetype` for `splunk_hec` -- see
the `file_log/app` receiver in
[`otelcol/config/example.yaml`](otelcol/config/example.yaml). If
`server_host.use_hostname` is `false`, `server_host.server_host` becomes
required -- the exporter fails at startup (not `validate`) without it.

Two ways to build your own:

- **Interactively**: `sgcia edit --config /etc/sgcia/config.yaml` -- see
  [Using sgcia](#using-sgcia) below for the keybindings. Must be run from
  an actual interactive terminal (SSH session, local shell); it won't
  work piped through something non-interactive like a CI job.
- **By hand**: start from the minimal example above or copy
  `otelcol/config/example.yaml`, edit the YAML directly, then
  `sgcia-otelcol validate --config file:/etc/sgcia/config.yaml` to check
  it.

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
sudo cp ~/securitygingercia/packaging/systemd/sgcia.env.example /etc/sgcia/sgcia.env  # adjust the path if you cloned elsewhere
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

### Firewall / network access

Binding a port is not the same as being reachable from other hosts --
if the log senders (routers, firewalls, other servers) are anywhere but
`localhost`, open the port in the host firewall too, matching whatever
you set as `listen_address` in your config:

```bash
# ufw (Debian/Ubuntu)
sudo ufw allow 514/udp
sudo ufw allow 601/tcp

# firewalld (RHEL/Fedora/CentOS)
sudo firewall-cmd --permanent --add-port=514/udp
sudo firewall-cmd --permanent --add-port=601/tcp
sudo firewall-cmd --reload
```

If the box is in a cloud VPC (AWS security group, Azure NSG, GCP
firewall rule, etc.), that network-level allow-list is separate from and
in addition to the host firewall -- both have to permit the traffic.
Deliberately **don't** open the `statuscfg`/`health_check` extension
ports (`7801`/`13133` by default) beyond loopback unless you have your
own auth/network control in front of them -- see
[The status endpoint](#the-status-endpoint).

## Using sgcia

```
sgcia-otelcol --config file:/etc/sgcia/config.yaml            # run the collector
sgcia-otelcol validate --config file:/etc/sgcia/config.yaml   # validate config, then exit
sgcia edit --config /etc/sgcia/config.yaml                    # interactive config editor
sgcia dashboard [--status-addr 127.0.0.1:7801]                # live monitoring dashboard
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

The same `statuscfg.endpoint` also serves a small browser-based
dashboard with a log viewer and topology diagram alongside the health
view -- see [The web UI](#the-web-ui).

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
currently highlighted, at the bottom of the screen. Press `?` from any
browsing/list screen (top-level tabs, pick-a-type, the operators list)
for a full keybinding reference (the same tables below), in case you
forget one of these mid-session -- it's not intercepted while a text
field is focused, so `?` still types normally there instead.

#### Navigating the top-level list (Receivers / Exporters / Extensions / Pipelines)

| Key | Action |
|---|---|
| `Tab` / `Shift+Tab` | Switch between Receivers / Exporters / Extensions / Pipelines tabs |
| `Up` / `Down` | Move selection within the current tab |
| `Enter` | Edit the selected item |
| `a` | Add a new item (pick a type, name it, then edit its fields) |
| `T` (Receivers tab only) | Add a receiver from a curated source template instead -- see below |
| `d` / `Delete` | Remove the selected item (asks for confirmation if a pipeline still references it) |
| `s` | Validate and save to disk |
| `?` | Show the full keybinding reference |
| `q` / `Esc` | Quit |

#### Editing a component's fields (the form screen)

| Key | Action |
|---|---|
| `Tab` / `Shift+Tab` | Move to the next/previous field |
| `Left` / `Right` | Cycle an enum-type or true/false field's value |
| `Enter` | Save the form and go back to the list -- **except** on a receiver's `operators` field, where `Enter` instead opens the operator-list sub-screen (see below); `Tab` off of it first if you meant to submit the whole form |
| `Esc` | Discard changes to this component, back to the list |
| any other key | Types into the focused text field |

#### Managing a receiver's inline `operators:` list

Reached by pressing `Enter` on a receiver's `operators` field.

| Key | Action |
|---|---|
| `Up` / `Down` | Move selection within the list |
| `a` | Add a new operator (pick a type, then edit its fields) |
| `Enter` | Edit the selected operator |
| `d` / `Delete` | Remove the selected operator |
| `Esc` | Back to the receiver's own form (focus moves off `operators`, so a follow-up `Enter` there submits the whole component instead of reopening this list) |

#### Adding a receiver from a source template

Reached by pressing `T` on the Receivers tab (see [Source
templates](#source-templates) above for the full list) -- a shortcut for
the common cases that skips hand-writing an `operators:` chain from
scratch.

| Key | Action |
|---|---|
| `Up` / `Down` | Move selection within the template list (grouped by category) |
| `Enter` | Pick the highlighted template, or (later) confirm the params/review step |
| `Esc` | Back one step (params -> back to the list; review -> back to params, keeping what you typed) |

Flow: pick a template -> name the new receiver (pre-filled with a
sensible default id, editable) -> fill in its few params -> review the
generated receiver as YAML -> `Enter` to insert it, same as any other
receiver from here on (editable via the normal form/operators screens,
validated the same way on save).

#### Editing a text field's contents

This is [`tui-input`](https://github.com/sayanarijit/tui-input)'s standard
readline-style editing, available in every text field (not just an
`sgcia`-specific convention) -- worth knowing well since it's the same in
any field, on any screen:

| Key | Action |
|---|---|
| `Left` / `Ctrl+B` | Move cursor back one character |
| `Right` / `Ctrl+F` | Move cursor forward one character |
| `Ctrl+Left` / `Alt+B` | Move cursor back one word |
| `Ctrl+Right` / `Alt+F` | Move cursor forward one word |
| `Home` / `Ctrl+A` | Move cursor to the start of the field |
| `End` / `Ctrl+E` | Move cursor to the end of the field |
| `Backspace` / `Ctrl+H` | Delete the character before the cursor |
| `Delete` | Delete the character under/after the cursor |
| `Ctrl+W` / `Alt+D` / `Alt+Backspace` | Delete the word before the cursor |
| `Ctrl+Delete` | Delete the word after the cursor |
| `Ctrl+K` | Delete from the cursor to the end of the field |
| `Ctrl+U` | **Clear the entire field**, not just cursor-to-start -- the whole thing, wherever the cursor is |
| `Ctrl+Y` | Paste back the text most recently removed by `Ctrl+U`/`Ctrl+W`/`Ctrl+K`/`Ctrl+Delete` |

Newly-added fields often start pre-filled with a placeholder or default
value (so the form is never blank) -- `Ctrl+U` is the fastest way to
clear one before typing your real value, rather than editing around the
placeholder. Note: pasting text into the terminal (as opposed to typing)
and mouse clicks/selection aren't supported by the underlying input
widget -- type or use the keys above.

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
   Name it `logs/test` -- **the `logs/` prefix is required**, not just a
   label choice like it is for receivers/exporters: OTel Collector
   requires every pipeline id to start with a recognized signal name
   (`logs`, `metrics`, or `traces`; this distro only ever uses `logs`).
   Naming it just `test` produces a confusing
   `cannot unmarshal the configuration ... unknown pipeline signal`
   error on save.
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

**Already ran `install.sh`?** It already created the `sgcia` user,
installed the unit, ran `systemctl enable`, and created
`/etc/sgcia/sgcia.env` (placeholder tokens) for you -- skip straight to
editing `sgcia.env`/`config.yaml` below, then `sudo systemctl start
sgcia`. The block below is still safe to run again except for its first
line (`useradd` errors, harmlessly, on a user that already exists) and
the two `cp` lines (harmless overwrites of files already in place, but
re-copying `sgcia.env.example` would clobber tokens you've already set).

A starter unit is at
[`packaging/systemd/sgcia.service`](packaging/systemd/sgcia.service).

```bash
# Dedicated non-root user the service runs as.
sudo useradd --system --home /var/lib/sgcia --shell /usr/sbin/nologin sgcia
sudo chown -R sgcia:sgcia /var/lib/sgcia

# Adjust ~/securitygingercia below if you cloned somewhere else --
# install.sh clones there by default (see the note under Installing).
sudo cp ~/securitygingercia/packaging/systemd/sgcia.service /etc/systemd/system/sgcia.service
sudo cp ~/securitygingercia/packaging/systemd/sgcia.env.example /etc/sgcia/sgcia.env
sudo "$EDITOR" /etc/sgcia/sgcia.env   # fill in real HEC token(s)
sudo chown sgcia:sgcia /etc/sgcia/sgcia.env
sudo chmod 600 /etc/sgcia/sgcia.env   # secrets file, keep it non-world-readable
sudo "$EDITOR" /etc/sgcia/config.yaml  # your actual config (start from otelcol/config/example.yaml)
sudo chgrp sgcia /etc/sgcia/config.yaml  # group (not owner!) read access for the service -- keeps it
sudo chmod 640 /etc/sgcia/config.yaml    # editable as yourself (no sudo) via `sgcia edit` afterward too

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

If a `file_log` receiver points at a file owned by some other system
group -- e.g. `/var/log/syslog`, owned by `syslog:adm` on Debian/Ubuntu --
add `sgcia` to that group too, or the receiver just logs a `permission
denied` on every poll instead of actually failing to start (the service
itself still shows `active (running)`, so this is easy to miss unless
you check `journalctl`):

```bash
sudo usermod -aG adm sgcia   # group name varies by distro/file -- check `ls -la` on the file itself
sudo systemctl restart sgcia
```

Once it's running as a service, use `sgcia dashboard` and `sgcia edit`
over an actual SSH session to the box (not through `systemctl` or a
script -- both are interactive terminal UIs):

```bash
sgcia dashboard --status-addr 127.0.0.1:7801
sgcia edit --config /etc/sgcia/config.yaml   # then `sudo systemctl restart sgcia` to apply
```

## Verifying the deployment

Four checks, in order of how much they actually prove:

1. **The service is up:**

   ```bash
   systemctl is-active sgcia   # should print "active"
   journalctl -u sgcia -n 50 --no-pager   # no repeating errors
   ```

2. **The status endpoint responds** (proves the process is healthy, not
   just "started"):

   ```bash
   curl -sf http://127.0.0.1:7801/status && echo OK
   ```

   `curl: (7) Failed to connect` here means either the process didn't
   really start (recheck step 1) or your config's `statuscfg.endpoint`
   isn't `127.0.0.1:7801`. You can't use `GET /config` to check that
   remotely for the same reason you can't reach `/status` -- read the
   config file directly on the box instead.

3. **A real event makes it all the way through.** Send a test line from
   an actual sender (or simulate one from the collector box itself) and
   confirm the counters move. `logger` below is the util-linux version
   (standard on Linux; macOS's built-in `logger` doesn't support
   sending over the network) -- match `-P`/`-d` (UDP) or `-T` (TCP) to
   whichever protocol/port your receiver actually listens on, and
   include `--rfc3164`: newer `logger` versions default to RFC 5424,
   which `protocol: rfc3164` (the example config's default) doesn't
   recognize at all -- the whole raw RFC 5424 line lands unparsed in
   `body` with `severity: UNSPECIFIED` instead of erroring, which is
   easy to mistake for "it's basically working":

   ```bash
   # from the collector box, simulating a remote sender over the network
   # (UDP, matching syslog/udp's default 0.0.0.0:514 in the example config):
   logger -n 127.0.0.1 -P 514 -d --rfc3164 "deployment verification test"
   sleep 2
   curl -s http://127.0.0.1:7801/status | jq '.receivers, .pipelines, .exporters'
   ```

   `receivers.<id>.events_in` and `pipelines.<name>.events_in` should
   both have incremented by 1. If they haven't, the receiver isn't
   reachable (check [Firewall / network access](#firewall--network-access)
   and that you're testing the right protocol/port) or isn't matching
   your `protocol`/framing settings.

4. **It reaches the real destination.** Check `pipelines.<name>.events_out`
   and `exporters.<id>.batches_sent` incremented too (not just
   `events_in`), and `exporters.<id>.last_error` is `null`. If
   `batches_failed` is climbing instead, the HEC endpoint/token is wrong --
   check `journalctl -u sgcia` for the actual HTTP error, and confirm
   `/etc/sgcia/sgcia.env` has the right token and the service was
   restarted after you last edited it (`EnvironmentFile=` is only read at
   process start).

## Upgrading

**The collector engine**, to pick up a security patch or new contrib
release: edit the `v0.x.0` version strings in
`otelcol/builder-config.yaml` to the new release (contrib and the
builder tool itself release in lockstep, so match both to the same
version), rebuild, and swap the binary in:

```bash
cd otelcol
GOTOOLCHAIN=go1.25.12 "$(go env GOPATH)/bin/builder" --config builder-config.yaml
./dist/sgcia-otelcol validate --config file:/etc/sgcia/config.yaml   # against your real config, before touching the service
sudo install -m 755 dist/sgcia-otelcol /usr/local/bin/sgcia-otelcol
sudo systemctl restart sgcia
journalctl -u sgcia -f   # watch it come back up cleanly
```

**The dashboard/editor**, after pulling new commits (from the repo root
-- `cd ..` first if you just did the collector-engine upgrade above,
which leaves you inside `otelcol/`):

```bash
cd ..
git pull
cargo build --release
sudo install -m 755 target/release/sgcia /usr/local/bin/sgcia
```

`sgcia` isn't managed by the systemd unit (only `sgcia-otelcol` runs as a
service), so there's nothing to restart -- just re-run `sgcia dashboard`/
`sgcia edit` next time you use them.

## Uninstalling

```bash
sudo systemctl disable --now sgcia
sudo rm /etc/systemd/system/sgcia.service
sudo systemctl daemon-reload

sudo rm /usr/local/bin/sgcia-otelcol /usr/local/bin/sgcia

# Only if you don't want to keep the config/checkpoints for a future reinstall:
sudo rm -rf /etc/sgcia /var/lib/sgcia
sudo userdel sgcia
```

## The status endpoint

The `statuscfg` extension (a small local addition, not part of upstream
contrib -- see
[`otelcol/extensions/statuscfgextension`](otelcol/extensions/statuscfgextension))
serves several read-only, unauthenticated HTTP endpoints on the address
set by its `endpoint` field (bind it to loopback unless you have another
way to restrict access -- there's no auth, and the loopback binding is
the security boundary):

- `GET /status` -- a JSON metrics snapshot: `started_at`,
  `uptime_seconds`, and per-receiver/pipeline/exporter counters, derived
  from the collector's own internal Prometheus telemetry (see the
  extension's `metrics_url` field). This is what `sgcia dashboard` and
  the web UI's Health view poll; useful directly too (`curl` it from a
  monitoring script, feed it to your own dashboard, etc).
- `GET /config` -- the same config file passed to `sgcia-otelcol`'s own
  `--config` flag, re-read and served as JSON, with any field named
  `token`, `password`, `secret`, `api_key`, or `apikey` replaced with
  `"***redacted***"` at any nesting depth.
- `GET /topology` -- a JSON graph (`nodes`/`edges`) of every configured
  receiver, exporter, and pipeline, and how they connect, derived purely
  from `service.pipelines` in the config above (not live data -- this
  collector only has a logs signal, so there's no runtime call graph to
  draw from). Powers the web UI's Topology view.
- `GET /logs?q=&severity=&attr_key=&attr_value=` -- the current contents
  of an in-memory, ~500-event rolling buffer of actual log record
  content (timestamp, severity, body, attributes, resource), optionally
  filtered by a case-insensitive substring (`q`), an exact severity
  match (`severity`), and/or an exact (not substring) match of
  `attr_key` against either the attributes or resource map
  (`attr_value`) -- this last pair is what powers the web UI's
  click-a-badge-to-correlate feature. Empty unless at least one pipeline
  has a `logbuffer` exporter (see [Available
  components](#available-components) and [The web UI](#the-web-ui)
  below) -- nothing is captured otherwise.
- `DELETE /logs` -- clears the buffer (204, empty body). Shared,
  server-side state: this affects every viewer of the web UI, not just
  whoever called it. Powers the web UI's "Clear buffer" button.
- `GET /` -- the embedded web UI itself (see [The web UI](#the-web-ui)).

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

A pipeline's own `events_out`/`events_dropped` are exact only if none of
its exporters are shared with another pipeline -- OTel's default
telemetry labels exporter counters by exporter alone, never by
`(pipeline, exporter)`, so there's no way to know exactly how much of a
*shared* exporter's total came from any one specific pipeline. When one
is shared, each pipeline's share is estimated proportionally to its own
`events_in` relative to the other pipelines using that same exporter
(falling back to an even split if none of them have any events in yet).
Exact in the common case of one exporter per pipeline; an approximation,
not a measurement error, once pipelines fan out to the same
destinations.

**Unlike `/config`, `/logs` is not redacted** -- it's the actual
converted log record content (body, attributes, resource) for whatever
pipelines have a `logbuffer` exporter attached. `/config` only ever
exposes structure and (redacted) settings; `/logs` exposes real event
data. Keep `statuscfg.endpoint` bound to loopback, same as today, and
treat adding a `logbuffer` exporter as a deliberate choice per pipeline
if that pipeline's log content is sensitive.

## The web UI

Point a browser at `http://<statuscfg.endpoint>/` (e.g.
`http://127.0.0.1:7801/` if you haven't changed the default, or from a
remote machine via an SSH tunnel: `ssh -L 7801:127.0.0.1:7801
user@host`, since the endpoint itself stays loopback-only) for a small
embedded dashboard -- plain HTML/CSS/vanilla JS served straight out of
the `sgcia-otelcol` binary (`otelcol/extensions/statuscfgextension/webui/`),
no separate process, no build step, no Node. Three views:

- **Health** -- uptime plus per-receiver/pipeline/exporter counters,
  polling `/status` every few seconds. The same data `sgcia dashboard`
  shows in a terminal, in a browser instead. Each row's relative volume
  (that row's count against the busiest row in the same table) renders
  as a visible meter bar, so a noisy receiver/pipeline/exporter stands
  out without reading every number. A pipeline with `events_dropped`/
  `parse_errors` above zero, or an exporter with `batches_failed` above
  zero, gets a red left-edge stripe; an exporter with a `last_error` also
  gets a small `!` badge -- hover it for the actual error message and
  when it happened, without needing `journalctl` on the box.
- **Logs** -- a live-updating table of actual log record content from
  the `/logs` buffer, with a search box (matches body/attributes/
  resource) and a severity filter. Every attribute/resource value also
  renders as a clickable badge -- click one to filter the view to only
  other events sharing that exact key=value, a quick way to answer
  "what else happened on this host/session?" without typing a query.
  **Export** downloads whatever's currently shown (query, severity, and
  correlation filter all included) as a JSON file. **Clear buffer**
  empties it via `DELETE /logs` -- shared state, so this affects every
  viewer, not just whoever clicked it (confirmed before it happens for
  exactly that reason). Empty until at least one pipeline has a
  `logbuffer` exporter attached (see [Available
  components](#available-components) above) -- add one via `sgcia edit`
  or by hand, same as any other exporter.
- **Topology** -- a receiver → pipeline → exporter Sankey diagram,
  combining the structural graph from `/topology` with live counts from
  `/status`: node height and ribbon width are real numbers, not
  decoration. Every node and edge here is sized off `events_in`, even
  for pipelines -- a pipeline's `events_out` is the *sum* of what it
  sent to every exporter it feeds (real fan-out replication means each
  exporter gets the pipeline's entire output, not a fraction, so with
  two exporters `events_out` is roughly double the pipeline's actual
  throughput). That's the right number for the Pipelines table's
  bandwidth accounting above, but sizing *this* diagram off it would
  make a pipeline's own node -- and each of its outbound edges --
  balloon by however many exporters it happens to feed. `events_in`
  passes straight through from receiver to pipeline unchanged, and an
  exporter's own `events_in` is a real counter that already reflects
  everything that arrived from every pipeline feeding it -- so every
  node and ribbon is an apples-to-apples, real number from `/status`,
  not an estimate, and a receiver, its pipeline, and that pipeline's
  exporters all read the same size when nothing else is sharing them.
  Each pipeline gets its own base color, carried unshaded onto its node
  border and every outbound (exporter-side) ribbon, so a pipeline stays
  traceable through crossings instead of blurring into a single wash of
  blue. Inbound (receiver-side) ribbons get a shade variant of that same
  base color per receiver, so two receivers feeding one pipeline (e.g.
  syslog/udp and syslog/tcp both into logs/syslog) don't render as one
  indistinguishable ribbon.
  Box height grows on its own when a column only has a few nodes to
  show (shrinking back down once a column gets crowded). Incoming
  ribbons (e.g. two receivers feeding one pipeline -- a real merge of
  distinct data) stack and center as a group within the node they land
  on, rather than clinging to its top edge. Outgoing ribbons don't
  stack at all -- every exporter attached to a pipeline gets that
  pipeline's *entire* output, not a share of it, so all of a node's
  outbound ribbons fan out from the same point at its vertical center
  instead of competing for space as if they were additive.
  Every node and ribbon also shows a percentage alongside its raw
  count, of total ingested logs (every receiver's `events_in` summed) --
  a receiver's share passes straight through to its pipeline unchanged,
  and an exporter's percentage is the true cumulative sum of every
  pipeline feeding it, so percentages stay legible and (bar rounding)
  never exceed 100% anywhere in the diagram.

Keyboard shortcuts (press `?` anywhere in the UI for a reminder): `h`/
`l`/`t` jump to Health/Logs/Topology, `/` focuses the Logs search box,
`n`/`p` jump to the next/previous ERROR entry in the Logs table, and
`Esc` clears the current search/correlation filter (or closes the `?`
overlay if it's open).

This is a read-only companion to `sgcia dashboard`/`sgcia edit`, not a
replacement -- the TUIs still work exactly as before, and the web UI is
just another way to look at the same loopback-only endpoints.

## Troubleshooting

- **`command not found` for `sgcia` or `sgcia-otelcol`.** Either the
  binary wasn't actually built (recheck the build step's output for
  errors) or your terminal cached its list of known commands before the
  install -- open a **brand new terminal window** (or run `hash -r`) and
  try again. As a fallback that always works: use the full path instead,
  e.g. `./target/release/sgcia` or `./otelcol/dist/sgcia-otelcol`.

- **`bind: address already in use`** on `514`/`601`/`7801`/`8888`/`13133`
  (or whatever you set): something else on the box is already listening
  on that port. `sudo lsof -i :514` (or the port in question) to see
  what -- a leftover previous run of the collector that didn't shut down
  cleanly, the system's own syslog daemon (`rsyslogd`/`syslog-ng`, common
  on the standard 514/601), or an unrelated service (Docker Desktop/
  OrbStack and similar tools are known to grab a wide range of local
  ports). Either stop the conflicting process or pick a different port
  in your config for whichever component collided.

- **`listen ... permission denied`** on a port below 1024 (514, 601):
  you're not running with `CAP_NET_BIND_SERVICE`/root. Under systemd,
  confirm the unit's `AmbientCapabilities=CAP_NET_BIND_SERVICE` line
  wasn't removed (see [systemd](#running-as-a-systemd-service-linux)).
  Running by hand as a normal user for a quick test: either use `sudo`
  (not recommended long-term) or point `listen_address` at an
  unprivileged port instead (see [Privileged ports](#privileged-ports)).

- **`statuscfg: listening on ...: address already in use` but only the
  `statuscfg`/`health_check` extension fails, receivers start fine.**
  Same root cause as the general port conflict above, just isolated to
  that one extension -- change its `endpoint` field and, if you're using
  `sgcia dashboard`, pass the matching `--status-addr`.

- **`reading config_path "..." : no such file or directory`** from the
  `statuscfg` extension at startup: its `config_path` field is a
  *relative* path, resolved against the collector process's current
  working directory at the moment it was started -- not relative to the
  config file's own location. Either use an absolute path for
  `config_path`, or always start `sgcia-otelcol` from the same directory
  (the systemd unit's `WorkingDirectory=/var/lib/sgcia` handles this for
  the service; if running by hand, `cd` there first or use an absolute
  path).

- **`requires a non-empty "token"`/`api_key is required`** from
  `sgcia-otelcol validate` or `sgcia-otelcol` itself, even though you've
  already filled in `/etc/sgcia/sgcia.env`: that file is only read
  automatically by the **systemd unit** (`EnvironmentFile=` in the unit
  file) -- running either command directly in your own shell doesn't
  source it. Either `sudo systemctl start sgcia` (the real path, since
  the unit reads it correctly on its own), or source it into your
  current shell first if you specifically want to run/validate by hand:
  `set -a; source /etc/sgcia/sgcia.env; set +a`.

- **A `syslog` receiver accepts events with no error, but `body` is the
  entire raw line (priority/timestamp/hostname and all) and
  `severity` comes back `UNSPECIFIED`.** The sender and the receiver's
  `protocol` field disagree about which syslog RFC is on the wire --
  most commonly, a newer `logger` defaults to RFC 5424 while `protocol:
  rfc3164` (the example config's default) expects RFC 3164, so the
  parser doesn't recognize the envelope at all and passes the whole
  line through unparsed instead of erroring. Confirm what your sender
  actually emits (`logger --help` lists `--rfc3164`/`--rfc5424`), and
  match `protocol` to it, or force the sender to the format your
  receiver expects (e.g. `logger ... --rfc3164`).

- **`directory must exist: ... no such file or directory`** from the
  `file_storage` extension: add `create_directory: true` to it (see
  [`otelcol/config/example.yaml`](otelcol/config/example.yaml)), or
  create the directory yourself first.

- **`storage extension 'file_storage' not found`** (or similar, naming
  whichever extension) at startup, even though it's right there under
  `extensions:` in your config: it's defined but not *activated* -- add
  its id to `service.extensions` (see the note in
  [Configuring](#configuring)). This passes `sgcia-otelcol validate`
  fine (validate doesn't check that extensions are wired in, only that
  each one's own fields are well-formed) and only fails at real startup,
  which is what makes it confusing.

- **`cannot unmarshal the configuration ... unknown pipeline signal:
  "..."`** on save (this happens with **or** without `sudo` -- it's a
  config-content problem, not a permissions one): a pipeline under
  `service.pipelines` is named without the required signal prefix, e.g.
  `test` instead of `logs/test`. Unlike receiver/exporter/extension ids,
  where the part before `/` is a component type and the part after is
  any label you like, a pipeline id's prefix must literally be `logs`,
  `metrics`, or `traces` -- see the note in
  [Configuring](#configuring). Rename the pipeline (in `sgcia edit`,
  you'll need to delete it and re-add it with the corrected name, since
  the id is set once at creation).

- **`it was not get server host: when UseHostName is False, then
  ServerHost has to be set`** at startup, from a `dataset` exporter: same
  "passes `validate`, fails at real startup" shape as the extension issue
  above. Either leave `server_host.use_hostname: true` (the default), or
  set `server_host.server_host` to a real fallback value if you turn it
  off -- see the note in [Configuring](#configuring).

- **`download go1.25 for linux/arm64: toolchain not available`** while
  building `sgcia-otelcol` by hand: see the note under
  [MANUAL.md's step 2](MANUAL.md#2-build-sgcia-otelcol-the-collector-engine)
  -- pin a concrete toolchain version with `GOTOOLCHAIN=go1.25.12` for
  that one command rather than relying on Go's automatic resolution
  (`install.sh` already does this for you).

- **`windows eventlog receiver is only supported on Windows`**: exactly
  what it says -- a pipeline using `windows_event_log` will fail to
  start the collector on Linux/macOS even though the binary builds fine
  there. If you started from `example.yaml` (the Linux/macOS sample)
  this shouldn't come up -- it doesn't have that pipeline. It means
  either your config was hand-written to include `windows_event_log`
  outside of Windows, or you started from
  [`example-windows.yaml`](otelcol/config/example-windows.yaml) on a
  non-Windows host by mistake. Drop that pipeline for non-Windows hosts,
  or run this component specifically on a Windows host (see
  [Windows](MANUAL.md#windows)).

- **`sgcia edit` says `couldn't run 'sgcia-otelcol validate'`** on save:
  it shells out to the real `sgcia-otelcol` binary to validate (there's
  no separate Rust-side validator) and couldn't find it. Either install
  it on `PATH` (see [Installing](#installing)), or point at it directly
  with `SGCIA_OTELCOL_BIN=/path/to/sgcia-otelcol sgcia edit --config ...`.

- **`sgcia dashboard` shows a red "connection failed" banner**: it can't
  reach the `statuscfg` extension's `endpoint`. Confirm `sgcia-otelcol`
  is actually running (`systemctl status sgcia`), that its config's
  `statuscfg.endpoint` matches the `--status-addr` you passed (both
  default to `127.0.0.1:7801`), and that nothing else grabbed that port
  first (see the port-conflict entries above).

## Development

Two independent things to build/test, matching the two binaries above.

**The collector engine** (`otelcol/`, Go -- `extensions/statuscfgextension`
is its own Go module, independent of the generated `dist/` module):

```bash
(cd otelcol/extensions/statuscfgextension && go test ./...)

cd otelcol
GOTOOLCHAIN=go1.25.12 "$(go env GOPATH)/bin/builder" --config builder-config.yaml
./dist/sgcia-otelcol validate --config file:config/example.yaml
```

**The dashboard/editor** (repo root -- `cd ..` first if you just ran the
Go commands above, which leave you inside `otelcol/`):

```bash
cd ..
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
```

`crates/collector/src/editor`'s own test suite includes an integration
test that runs the real `sgcia-otelcol validate` (skipped automatically,
not failed, if that binary hasn't been built yet at the conventional
`otelcol/dist/sgcia-otelcol` path or via `SGCIA_OTELCOL_BIN`) -- build the
Go side first if you want that coverage included.
