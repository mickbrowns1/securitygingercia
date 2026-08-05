# Manual installation

This is the step-by-step, from-source build process that
[`install.sh`](install.sh) automates for you. Use this page if you're
not on Linux/macOS (see [Windows](#windows) below), want to see or
control each step yourself, or are debugging something the script did.

For everything past "the binaries exist and are installed" --
configuring, running, the systemd service, upgrading, troubleshooting --
see [README.md](README.md).

You're building two independent binaries here: `sgcia-otelcol` (Go) and
`sgcia` (Rust). Neither depends on the other at build time.

## 1. Get the code

**Prerequisite: git.** A fresh server image often doesn't have it yet --
check with `git --version` first; if that says "command not found":

```bash
# Debian / Ubuntu
sudo apt update && sudo apt install -y git

# Fedora / RHEL / CentOS
sudo dnf install -y git

# Arch
sudo pacman -S --needed git

# macOS (this also satisfies step 3's C-compiler prerequisite, since both
# come from the same Xcode Command Line Tools install)
xcode-select --install
```

```bash
git clone https://github.com/mickbrowns1/securitygingercia.git
cd securitygingercia
```

## 2. Build `sgcia-otelcol` (the collector engine)

**Prerequisite: Go.** Check with `go version` first; if that says
`command not found`:

```bash
# Debian / Ubuntu
sudo apt update && sudo apt install -y golang-go

# Fedora / RHEL / CentOS
sudo dnf install -y golang

# Arch
sudo pacman -S --needed go

# macOS
brew install go
```

Whatever version your package manager gives you (even if it looks old --
Go 1.21+ is enough) is fine to *start* with. If your distro's package is
genuinely too old (pre-1.21) or unavailable, install directly from
[go.dev/doc/install](https://go.dev/doc/install) instead.

**Prerequisite: Node.js**, to build the embedded web UI --
`sgcia-otelcol` embeds its compiled output (`go:embed`), so this has to
happen *before* the builder step below, not after. Check with `node
--version` first; if that says `command not found`:

```bash
# Debian / Ubuntu
sudo apt update && sudo apt install -y nodejs npm

# Fedora / RHEL / CentOS
sudo dnf install -y nodejs npm

# Arch
sudo pacman -S --needed nodejs npm

# macOS
brew install node
```

```bash
cd otelcol/extensions/statuscfgextension/webui-react
npm ci
npm run build
cd ../../../..
```

This produces `webui-react/dist/`, which the builder step below
embeds into the binary -- see `webui-react/README.md` if you're
editing the UI itself, not just building it once.

Install the builder tool once, then run it against this repo's manifest:

```bash
go install go.opentelemetry.io/collector/cmd/builder@latest
cd otelcol
GOTOOLCHAIN=go1.25.12 "$(go env GOPATH)/bin/builder" --config builder-config.yaml
```

The first line auto-upgrades itself to whatever newer Go toolchain it
needs just fine. The `GOTOOLCHAIN=go1.25.12` pin on the second line
works around a real rough edge in that *same* auto-upgrade feature, hit
one step later: without it, the builder's own internal `go mod tidy`
step fails outright with `download go1.25 for linux/arm64: toolchain not
available` instead of resolving a working version on its own -- pinning
one explicitly sidesteps that. (Check [go.dev/dl](https://go.dev/dl/) if
`go1.25.12` itself is no longer current by the time you read this -- any
version satisfying this repo's `builder-config.yaml`/`go.mod`
requirements works.)

This downloads the pinned `opentelemetry-collector-contrib` receiver/
exporter/extension versions from `builder-config.yaml`, generates a small
`main.go`/`go.mod` under `otelcol/dist/`, and compiles the binary there:
`otelcol/dist/sgcia-otelcol`.

**Bumping a security patch later**: edit the `v0.x.0` version strings in
`otelcol/builder-config.yaml` to the new contrib release, then re-run the
same `builder --config builder-config.yaml` command from inside `otelcol/`.

## 3. Build `sgcia` (the dashboard/editor)

**Prerequisite: a C linker/compiler**, which Rust needs to link the final
binary regardless of whether the project itself has any C code -- a fresh
Ubuntu/Debian box has none installed by default and fails with
``error: linker `cc` not found`` on the very first `cargo build`
otherwise.

```bash
# Debian / Ubuntu
sudo apt update && sudo apt install -y build-essential

# Fedora / RHEL / CentOS
sudo dnf groupinstall -y "Development Tools"

# Arch
sudo pacman -S --needed base-devel

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

Build, from the repo root -- not `otelcol/`, which step 2 left you inside
of; this is a separate Cargo workspace at the top level:

```bash
cd ..
cargo build --release
```

If this is the first thing you're compiling on the machine and it fails
with a linker error, go back to the prerequisite above -- that's what it
means. If it instead fails with an *`undefined reference to 'main'`*
linker error partway through (rather than immediately, and rather than
the "linker `cc` not found" error above), that's a one-off flake --
usually parallel compilation contending for CPU/memory on a small VM --
not a real problem with your setup; just run `cargo build --release`
again.

The binary lands at `target/release/sgcia` (macOS/Linux) or
`target\release\sgcia.exe` (Windows).

## 4. Install both binaries

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
[README.md](README.md#running-as-a-systemd-service-linux)) only needs
*read* access to the config, which a normal `chown`ed file still allows.

## Windows

The `windows_event_log` receiver only runs on Windows (it compiles
elsewhere, but the collector refuses to start a pipeline using it on
Linux/macOS) -- if you need that receiver, build and run `sgcia-otelcol`
on an actual Windows host, not cross-compiled from Linux/macOS. That's
also why there are two example configs:
[`otelcol/config/example.yaml`](otelcol/config/example.yaml) (no
`windows_event_log`, used on Linux/macOS) and
[`otelcol/config/example-windows.yaml`](otelcol/config/example-windows.yaml)
(the same thing plus that one receiver and its pipeline) -- start from
the Windows one here, not the plain one.

In a PowerShell prompt, with [Git](https://git-scm.com/downloads/win),
[Go](https://go.dev/doc/install), [Node.js](https://nodejs.org/)
(builds the embedded web UI -- `sgcia-otelcol` embeds its compiled
output, so this has to happen before the builder step below), and the
[Rust toolchain](https://rustup.rs) installed (all four ship native
Windows installers -- no build-essential/xcode-select equivalent needed,
MSVC's linker comes with the Rust installer's prompt to also install the
Visual Studio Build Tools if you don't already have a C++ toolchain):

```powershell
git clone https://github.com/mickbrowns1/securitygingercia.git
cd securitygingercia

cd otelcol\extensions\statuscfgextension\webui-react
npm ci
npm run build
cd ..\..\..\..

go install go.opentelemetry.io/collector/cmd/builder@latest
cd otelcol
$env:GOTOOLCHAIN = "go1.25.12"   # works around a Go toolchain-resolution bug -- see step 2 above
& "$(go env GOPATH)\bin\builder.exe" --config builder-config.yaml
cd ..

cargo build --release
```

Binaries land at `otelcol\dist\sgcia-otelcol.exe` and
`target\release\sgcia.exe`. Copy them somewhere on your `PATH` (e.g.
`C:\Program Files\sgcia\`, added to the system `Path` environment
variable via *System Properties → Environment Variables*), then create
working directories:

```powershell
New-Item -ItemType Directory -Force -Path C:\ProgramData\sgcia
New-Item -ItemType Directory -Force -Path C:\ProgramData\sgcia\storage
Copy-Item otelcol\config\example-windows.yaml C:\ProgramData\sgcia\config.yaml
```

Point your config's `file_storage` extension's `directory` and
`windows_event_log`'s `channel` at real values (e.g. `channel: Security`),
then run the same way as Linux/macOS, just with `file:` paths using
Windows separators:

```powershell
$env:S1_HEC_TOKEN = "your-token"
sgcia-otelcol.exe --config "file:C:\ProgramData\sgcia\config.yaml"
```

There's no systemd equivalent bundled here -- for a real Windows
deployment, wrap the above in a
[Windows Service](https://learn.microsoft.com/en-us/windows/win32/services/services)
(e.g. via [NSSM](https://nssm.cc/) or `sc.exe create`) so it starts on
boot and restarts on failure, mirroring what the systemd unit does on
Linux.
