#!/usr/bin/env bash
# preflight.sh — what `cargo xtask judge` needs before cargo compiles
# anything. The judge doctor (xtask/src/judge.rs) only runs once xtask
# itself is built, which takes minutes from cold and fails with a
# compiler error, not a fix, when the C toolchain or rustup is missing.
# This script checks those first and prints one exact fix per problem.
# Nothing is installed for you.
#
# Usage: ops/judge/preflight.sh [--port-base N]
#        ops/judge/preflight.sh && cargo xtask judge
set -u

NODE_MIN=24
NPM_MIN=11
NPM_FALLBACK=10 # npm 10 still boots: `up` installs with --legacy-peer-deps
MIN_FREE_GB=6
BASE=18318

while [ $# -gt 0 ]; do
  case "$1" in
    --port-base)
      BASE="${2:-}"
      shift 2 || true
      ;;
    *) echo "usage: ops/judge/preflight.sh [--port-base N]" >&2; exit 2 ;;
  esac
done
case "$BASE" in
  '' | *[!0-9]*) echo "preflight: --port-base needs a number" >&2; exit 2 ;;
esac
if [ "$BASE" -lt 1024 ] || [ "$BASE" -gt 65534 ]; then
  echo "preflight: --port-base needs a port from 1024 to 65534" >&2
  exit 2
fi

problems=0
problem() { # $1=what $2=fix
  problems=$((problems + 1))
  printf '  ✗ %s\n    fix: %s\n' "$1" "$2"
}
warning() { # $1=what $2=fix
  printf '  ! %s\n    fix (optional): %s\n' "$1" "$2"
}
have() { command -v "$1" >/dev/null 2>&1; }
major() { # first number in $1 (v24.1.0 -> 24, 11.2.0 -> 11)
  printf '%s' "$1" | sed -E 's/^[^0-9]*([0-9]+).*/\1/'
}

os=$(uname -s)
case "$os" in
  Darwin) pkg='xcode-select --install' ;;
  Linux) pkg='sudo apt install build-essential perl git (or your distro'"'"'s equivalents)' ;;
  *) problem "unsupported OS $os (macOS or Linux only)" "run it on macOS or Linux (WSL2 works on Windows)"; pkg='' ;;
esac

echo "preflight: checking what the build needs (nothing is installed for you)"

have git || problem "git is not on PATH" "$pkg"

# C toolchain: aws-lc, SQLite and the vendored OpenSSL compile C code.
if ! have cc || ! cc --version >/dev/null 2>&1; then
  problem "no working C compiler (cc)" "$pkg"
fi
have make || problem "make is not on PATH (the vendored OpenSSL build runs it)" "$pkg"
have perl || problem "perl is not on PATH (the vendored OpenSSL build runs Configure)" "$pkg"

# Rust through rustup: rust-toolchain.toml pins 1.99.0 and only rustup
# installs it on demand. A distro cargo/rustc builds with the wrong one.
if ! have rustup; then
  if have cargo; then
    problem "cargo at $(command -v cargo) is not rustup's, so the pinned toolchain cannot install itself" \
      "install rustup (https://rustup.rs): curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
  else
    problem "Rust is not installed" \
      "install rustup (https://rustup.rs): curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
  fi
else
  rustup_dir=$(dirname "$(command -v rustup)")
  cargo_path=$(command -v cargo || true)
  if [ -z "$cargo_path" ]; then
    problem "rustup is installed but cargo is not on PATH" "add $rustup_dir to PATH (source \"\$HOME/.cargo/env\")"
  elif [ "$(dirname "$cargo_path")" != "$rustup_dir" ]; then
    problem "cargo resolves to $cargo_path, not rustup's proxy in $rustup_dir" \
      "put $rustup_dir first in PATH, or remove the distro cargo/rustc"
  fi
fi

# Node and npm (web/package.json engines).
if ! have node; then
  problem "Node.js is not on PATH" "install Node $NODE_MIN+: nvm install $NODE_MIN && nvm use $NODE_MIN"
else
  nv=$(node --version 2>/dev/null)
  nm=$(major "$nv")
  if [ -z "$nm" ] || [ "$nm" -lt "$NODE_MIN" ]; then
    problem "Node $nv is older than $NODE_MIN" "nvm install $NODE_MIN && nvm use $NODE_MIN"
  fi
fi
if ! have npm; then
  problem "npm is not on PATH" "it ships with Node $NODE_MIN+: nvm install $NODE_MIN"
else
  pv=$(npm --version 2>/dev/null)
  pm=$(major "$pv")
  if [ -z "$pm" ] || [ "$pm" -lt "$NPM_FALLBACK" ]; then
    problem "npm $pv is older than $NPM_MIN" "npm install -g npm@$NPM_MIN"
  elif [ "$pm" -lt "$NPM_MIN" ]; then
    warning "npm $pv is older than $NPM_MIN; the boot continues with npm ci --legacy-peer-deps" \
      "npm install -g npm@$NPM_MIN"
  fi
fi

# Ports: the public site on BASE, the staff console on BASE+1.
for port in "$BASE" "$((BASE + 1))"; do
  if (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then
    problem "port $port is in use on 127.0.0.1" \
      "stop whatever holds it (lsof -nP -iTCP:$port -sTCP:LISTEN), or pick other ports: --port-base 28318"
  fi
done

# Disk: release target + node_modules + both SPAs + the store.
free_kb=$(df -Pk . 2>/dev/null | awk 'NR == 2 { print $4 }')
if [ -n "$free_kb" ] && [ "$((free_kb / 1024 / 1024))" -lt "$MIN_FREE_GB" ]; then
  problem "only $((free_kb / 1024 / 1024)) GB free here; the first build needs about $MIN_FREE_GB GB" \
    "free disk space, or clone into a roomier volume"
fi

if [ ! -f Cargo.toml ] || [ ! -f web/package.json ]; then
  problem "not at the repository root" "cd into the cloned repository, then run ops/judge/preflight.sh"
fi

if [ "$problems" -gt 0 ]; then
  echo
  echo "preflight: $problems problem(s) to fix first, then run it again."
  exit 1
fi
if [ "$BASE" = 18318 ]; then
  echo "preflight: ok — next: cargo xtask judge"
else
  echo "preflight: ok — next: cargo xtask judge --port-base $BASE"
fi
