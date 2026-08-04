#!/usr/bin/env bash
# One-command installer for sgcia (see README.md for what each step below
# does and why -- this script just automates the "Installing" section).
#
# Usage:
#   ./install.sh                 # from inside an existing checkout
#   curl -fsSL .../install.sh | bash   # clones the repo first if needed
#
# What it does, in order: installs git/Go/a C toolchain/Rust if missing,
# builds sgcia-otelcol (via the OpenTelemetry Collector Builder) and sgcia
# (via cargo), installs both to /usr/local/bin, creates /etc/sgcia +
# /var/lib/sgcia, and (on systemd Linux) sets up and enables the sgcia
# service -- enabled, not started, since the starter config it drops in
# still has placeholder secrets/paths. It deliberately stops there --
# writing your actual config and secrets are decisions only you can make;
# see the "Configuring" section of README.md for those next steps.

set -euo pipefail

REPO_URL="https://github.com/mickbrowns1/securitygingercia.git"
GOTOOLCHAIN_PIN="go1.25.12" # see README.md's "Installing" section for why this is pinned

BOLD=""; DIM=""; RESET=""
if [ -t 1 ] && command -v tput >/dev/null 2>&1 && [ "$(tput colors 2>/dev/null || echo 0)" -ge 8 ]; then
  BOLD="$(tput bold)"; DIM="$(tput dim)"; RESET="$(tput sgr0)"
fi

step()  { printf '\n%s==>%s %s\n' "$BOLD" "$RESET" "$*"; }
info()  { printf '%s  %s%s\n' "$DIM" "$*" "$RESET"; }
die()   { printf '\nERROR: %s\n' "$*" >&2; exit 1; }
have()  { command -v "$1" >/dev/null 2>&1; }

if [ "$(id -u)" -eq 0 ]; then
  die "Don't run this script as root/with sudo -- it asks for sudo only for the specific steps that need it (installing packages, copying to /usr/local/bin). Run it as your normal user instead."
fi

OS="$(uname -s)"
DISTRO=""
if [ "$OS" = "Linux" ] && [ -r /etc/os-release ]; then
  DISTRO="$(. /etc/os-release && echo "$ID")"
fi

pkg_install() {
  # $1 = human-readable name for logging, $2.. = actual package name(s)
  local what="$1"; shift
  step "Installing $what"
  case "$OS" in
    Linux)
      case "$DISTRO" in
        ubuntu|debian)
          sudo apt-get update -y && sudo apt-get install -y "$@" ;;
        fedora|rhel|centos|rocky|almalinux)
          sudo dnf install -y "$@" ;;
        arch)
          sudo pacman -S --needed --noconfirm "$@" ;;
        *)
          die "Unrecognized Linux distro ($DISTRO). Install $what manually (see README.md's Installing section), then re-run this script." ;;
      esac
      ;;
    Darwin)
      have brew || die "Homebrew isn't installed. Install it from https://brew.sh, then re-run this script."
      brew install "$@"
      ;;
    *)
      die "Unsupported OS ($OS). See README.md's Installing section for manual steps." ;;
  esac
}

# --- 1. git + the repo itself ---

step "Checking for git"
if have git; then
  info "found: $(git --version)"
else
  case "$OS" in
    Linux)
      case "$DISTRO" in
        ubuntu|debian) pkg_install git git ;;
        fedora|rhel|centos|rocky|almalinux) pkg_install git git ;;
        arch) pkg_install git git ;;
        *) die "Unrecognized Linux distro ($DISTRO). Install git manually, then re-run this script." ;;
      esac
      ;;
    Darwin)
      step "Installing git (via Xcode Command Line Tools)"
      xcode-select --install || true
      info "If a popup appeared, finish that install, then re-run this script."
      have git || die "git still not found -- finish the Xcode Command Line Tools install above, then re-run this script."
      ;;
    *) die "Unsupported OS ($OS)." ;;
  esac
fi

step "Locating the sgcia repo"
if [ -f "./otelcol/builder-config.yaml" ]; then
  info "already inside the repo -- using $(pwd)"
elif [ -f "./securitygingercia/otelcol/builder-config.yaml" ]; then
  info "found an existing checkout at ./securitygingercia -- using it"
  cd securitygingercia
  git pull --ff-only || info "couldn't fast-forward -- continuing with what's on disk"
else
  step "Cloning the repo"
  git clone "$REPO_URL" securitygingercia
  cd securitygingercia
fi
REPO_ROOT="$(pwd)"

# --- 2. Go + build sgcia-otelcol ---

step "Checking for Go"
if have go; then
  info "found: $(go version)"
else
  case "$OS" in
    Linux)
      case "$DISTRO" in
        ubuntu|debian) pkg_install Go golang-go ;;
        fedora|rhel|centos|rocky|almalinux) pkg_install Go golang ;;
        arch) pkg_install Go go ;;
      esac
      ;;
    Darwin) pkg_install Go go ;;
  esac
fi
have go || die "Go still not found after install -- see README.md's Installing section."

step "Installing the OpenTelemetry Collector Builder tool"
go install go.opentelemetry.io/collector/cmd/builder@latest
BUILDER="$(go env GOPATH)/bin/builder"
[ -x "$BUILDER" ] || die "builder tool didn't install where expected ($BUILDER)."

step "Building sgcia-otelcol (this downloads and compiles the collector components -- can take a minute or two)"
(
  cd "$REPO_ROOT/otelcol"
  GOTOOLCHAIN="$GOTOOLCHAIN_PIN" "$BUILDER" --config builder-config.yaml
)
[ -x "$REPO_ROOT/otelcol/dist/sgcia-otelcol" ] || die "sgcia-otelcol didn't build -- check the output above for errors."
info "built: $REPO_ROOT/otelcol/dist/sgcia-otelcol"

# --- 3. C toolchain + Rust + build sgcia ---

step "Checking for a C toolchain (cc)"
if have cc; then
  info "found: $(cc --version | head -n1)"
else
  case "$OS" in
    Linux)
      case "$DISTRO" in
        ubuntu|debian) pkg_install "a C toolchain" build-essential ;;
        fedora|rhel|centos|rocky|almalinux)
          step "Installing a C toolchain"
          sudo dnf groupinstall -y "Development Tools"
          ;;
        arch) pkg_install "a C toolchain" base-devel ;;
      esac
      ;;
    Darwin)
      step "Installing a C toolchain (via Xcode Command Line Tools)"
      xcode-select --install || true
      have cc || die "cc still not found -- finish the Xcode Command Line Tools install above, then re-run this script."
      ;;
  esac
fi

step "Checking for Rust (cargo)"
if have cargo; then
  info "found: $(cargo --version)"
else
  step "Installing Rust (as $USER, not root)"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  # shellcheck disable=SC1090
  [ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
  export PATH="$HOME/.cargo/bin:$PATH"
fi
have cargo || die "cargo still not found after install -- open a new terminal (so it picks up rustup's PATH changes) and re-run this script."

step "Building sgcia (this can take a couple of minutes the first time)"
(cd "$REPO_ROOT" && cargo build --release)
[ -x "$REPO_ROOT/target/release/sgcia" ] || die "sgcia didn't build -- check the output above for errors."
info "built: $REPO_ROOT/target/release/sgcia"

# --- 4. Install both binaries + create working directories ---

step "Installing both binaries to /usr/local/bin (will ask for your login password)"
sudo install -m 755 "$REPO_ROOT/otelcol/dist/sgcia-otelcol" /usr/local/bin/sgcia-otelcol
sudo install -m 755 "$REPO_ROOT/target/release/sgcia" /usr/local/bin/sgcia

step "Creating /etc/sgcia and /var/lib/sgcia"
sudo mkdir -p /etc/sgcia /var/lib/sgcia
sudo chown "$USER" /etc/sgcia /var/lib/sgcia

if [ ! -f /etc/sgcia/config.yaml ]; then
  step "Copying the example config to /etc/sgcia/config.yaml as a starting point"
  cp "$REPO_ROOT/otelcol/config/example.yaml" /etc/sgcia/config.yaml
  # The example ships a dev-relative config_path (correct only when run
  # from inside otelcol/, per README.md's Configuring section) -- rewrite
  # it to point at this file's real installed location, since the
  # statuscfg extension re-reads this exact path at startup regardless of
  # the process's own working directory.
  sed 's|config_path: .*|config_path: /etc/sgcia/config.yaml|' /etc/sgcia/config.yaml > /etc/sgcia/config.yaml.tmp
  mv /etc/sgcia/config.yaml.tmp /etc/sgcia/config.yaml
  info "edit it (or run 'sgcia edit --config /etc/sgcia/config.yaml') before going further -- it references"
  info "placeholder secrets and example hosts that won't work as-is."
else
  info "/etc/sgcia/config.yaml already exists -- leaving it alone"
fi

# --- 5. systemd service (Linux only -- enabled, not started) ---

SERVICE_SET_UP=0
if have systemctl && [ -d /run/systemd/system ]; then
  step "Setting up the sgcia systemd service"
  id sgcia >/dev/null 2>&1 || sudo useradd --system --home /var/lib/sgcia --shell /usr/sbin/nologin sgcia
  sudo chown -R sgcia:sgcia /var/lib/sgcia
  # Group (not owner!) read access for the service -- keeps config.yaml
  # editable as yourself, without sudo, via `sgcia edit` afterward too.
  sudo chgrp sgcia /etc/sgcia/config.yaml
  sudo chmod 640 /etc/sgcia/config.yaml
  sudo cp "$REPO_ROOT/packaging/systemd/sgcia.service" /etc/systemd/system/sgcia.service
  if [ ! -f /etc/sgcia/sgcia.env ]; then
    sudo cp "$REPO_ROOT/packaging/systemd/sgcia.env.example" /etc/sgcia/sgcia.env
    sudo chown sgcia:sgcia /etc/sgcia/sgcia.env
    sudo chmod 600 /etc/sgcia/sgcia.env
  fi
  sudo systemctl daemon-reload
  sudo systemctl enable sgcia
  SERVICE_SET_UP=1
  info "installed and enabled (starts automatically on every future boot) -- not started yet,"
  info "since the config and /etc/sgcia/sgcia.env above still have placeholder secrets/paths."
  info "Once you've edited both for real: sudo systemctl start sgcia"
else
  info "no systemd detected -- skipping service setup (see README.md's systemd section for"
  info "the manual equivalent, or MANUAL.md for non-systemd platforms)"
fi

step "Done"
"$REPO_ROOT/otelcol/dist/sgcia-otelcol" --version || true
"$REPO_ROOT/target/release/sgcia" --version || true
cat <<EOF

Both binaries are installed:
  sgcia-otelcol --version
  sgcia --version

Next steps (see README.md for details on each):
  1. Edit /etc/sgcia/config.yaml -- by hand, or interactively with:
       sgcia edit --config /etc/sgcia/config.yaml
EOF
if [ "$SERVICE_SET_UP" -eq 1 ]; then
  cat <<EOF
  2. Edit /etc/sgcia/sgcia.env (already created, placeholder tokens --
     the systemd service reads it automatically, no copying needed):
       sudo \$EDITOR /etc/sgcia/sgcia.env
EOF
else
  cat <<EOF
  2. Put real secrets (HEC tokens, etc.) somewhere your config's \${VAR_NAME}
     references can read them -- see the "Secrets" section of README.md.
EOF
fi
cat <<EOF
  3. Check the config is valid (sgcia.env isn't read outside systemd --
     source it first if validate complains your tokens are still empty):
       sgcia-otelcol validate --config file:/etc/sgcia/config.yaml
EOF
if [ "$SERVICE_SET_UP" -eq 1 ]; then
  cat <<EOF
  4. Start the systemd service (already enabled for future boots):
       sudo systemctl start sgcia
       journalctl -u sgcia -f
EOF
else
  cat <<EOF
  4. Run it directly to try it out:
       sgcia-otelcol --config file:/etc/sgcia/config.yaml
     ...or set it up as a service for a real deployment -- see the
     "Running as a systemd service" section of README.md (Linux) or
     MANUAL.md (other platforms).
EOF
fi
cat <<EOF
  5. Watch it run:
       sgcia dashboard --status-addr 127.0.0.1:7801
     ...or open http://127.0.0.1:7801/ in a browser once it's running.
EOF
