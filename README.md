# Security Ginger Collect It All (sgcia)

A single-binary log collector: syslog (UDP/TCP, RFC 3164/5424), flat-file
tailing, and Windows Event Log receivers, a configurable parsing pipeline,
and HEC exporters (SentinelOne DataPipeline + generic Splunk HEC). Includes
a live monitoring dashboard and an interactive config editor, both terminal
UIs.

```
sgcia run --config x.yaml [--status-addr 127.0.0.1:7801]   # run the collector
sgcia check --config x.yaml                                 # validate config, print pipeline graph
sgcia edit --config x.yaml                                   # interactive config editor (needs a real terminal)
sgcia dashboard [--status-addr 127.0.0.1:7801]                # live monitoring dashboard (needs a real terminal)
```

## Building on Linux

Build **on the target Linux machine** rather than cross-compiling from
another OS -- this project uses `reqwest` with `rustls` (via the `ring`
crate, which has C code), and cross-compiling C dependencies needs a
matching target sysroot that's easy to get wrong. Native compilation
sidesteps that entirely.

1. Install Rust (if not already present):

   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
   source "$HOME/.cargo/env"
   ```

2. Copy this repository to the Linux machine (git clone, scp, rsync --
   whatever you'd normally use), then build:

   ```bash
   cd syslog-collector
   cargo build --release
   ```

   The binary lands at `target/release/sgcia` (a few MB, no other runtime
   dependencies -- it's a normal dynamically-linked ELF binary, not fully
   static, so it still needs glibc/libssl-equivalent system libraries
   that are already present on virtually any mainstream distro).

3. If you specifically want a fully static binary (e.g. for a minimal
   container image), build against the musl target instead. Cross-C
   dependencies still apply, so do this on a Linux machine too, not from
   macOS:

   ```bash
   rustup target add x86_64-unknown-linux-musl
   cargo build --release --target x86_64-unknown-linux-musl --bin sgcia
   # binary: target/x86_64-unknown-linux-musl/release/sgcia
   ```

### Installing the binary

```bash
sudo install -m 755 target/release/sgcia /usr/local/bin/sgcia
sudo mkdir -p /etc/sgcia /var/lib/sgcia
```

`/var/lib/sgcia` is where file-tail checkpoints and Windows Event Log
bookmarks would live if configured with paths under it (see below) --
sgcia creates the file itself, but the parent directory should exist and
be writable by whatever user runs it.

## Configuring

Start from [`configs/example.yaml`](configs/example.yaml), which documents
every receiver/operator/exporter type. Two ways to build your own:

- **Interactively**: `sgcia edit --config /etc/sgcia/config.yaml` (run
  this from an actual terminal session -- it's a TUI, so it won't work
  piped through something non-interactive like a CI job or a `&&` chain
  with redirected stdin).
- **By hand**: copy the example and edit the YAML directly, then
  `sgcia check --config /etc/sgcia/config.yaml` to validate.

### Secrets

HEC tokens are referenced in the config as `${VAR_NAME}` (e.g. `token:
${S1_HEC_TOKEN}`) and substituted from the process environment at load
time -- the literal `${...}` stays in the YAML file, so the config itself
never contains a real secret. Supply the actual value via the
environment sgcia runs under (see the systemd unit below for the
recommended way).

### Privileged ports

Standard syslog ports (UDP/TCP 514, TCP 601) are below 1024, so binding
them as a non-root user needs an explicit capability grant rather than
running sgcia as root. The systemd unit below does this via
`AmbientCapabilities=CAP_NET_BIND_SERVICE`. If you'd rather not grant that,
point `listen_address` at an unprivileged port (e.g. `0.0.0.0:5514`) and
have your network/firewall layer forward/NAT 514 to it instead.

## Running as a systemd service

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

If your config's `checkpoint_file`/`bookmark_file` paths point outside
`/var/lib/sgcia`, either move them under it or add the real directory to
`ReadWritePaths=` in the unit file -- `ProtectSystem=strict` makes the
rest of the filesystem read-only to the service.

### Monitoring and editing once it's running as a service

Both TUIs need an interactive terminal, so run them over an actual SSH
session to the box (not via `systemctl`/a script):

```bash
sgcia dashboard --status-addr 127.0.0.1:7801
sgcia edit --config /etc/sgcia/config.yaml   # then `sudo systemctl restart sgcia` to apply
```

The config editor never talks to the running process -- it edits the
YAML file offline. Changes only take effect after a restart.
