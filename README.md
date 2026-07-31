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