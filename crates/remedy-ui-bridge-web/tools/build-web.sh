#!/usr/bin/env bash
# Builds the bridge's demo (or another of its examples) into an installable page (a PWA)
# with `cargo remedy package --target web`, which does the same for any app: the WebAssembly
# module and its wasm-bindgen glue, an HTML page, a web app manifest, icons, and a service
# worker that keeps the page working offline. The names and icon are in ../remedy.toml.
# Optionally serves it on localhost.
#
#   tools/build-web.sh [serve]
#
# Settings (environment):
#   EXAMPLE     the example to build (default web-demo)
#   PROFILE     release (default) or dev
#   OUT         output folder (default target/remedy/web/<example>, under CARGO_TARGET_DIR when set)
#   PORT        port for `serve` (default 8080)
#
# Needs the wasm32-unknown-unknown target (`rustup target add wasm32-unknown-unknown`) and the
# wasm-bindgen CLI of the version the workspace pins (`cargo install wasm-bindgen-cli
# --version 0.2.130`). Nothing global is changed.
set -euo pipefail

crate="$(cd "$(dirname "$0")/.." && pwd)"
root="$(cd "$crate/../.." && pwd)"
example="${EXAMPLE:-web-demo}"
target_dir="${CARGO_TARGET_DIR:-$root/target}"
out="${OUT:-$target_dir/remedy/web/$example}"
args=(remedy package --target web --example "$example" --out "$out")
[ "${PROFILE:-release}" = "dev" ] && args+=(--dev)

cd "$crate"
cargo run --quiet --manifest-path "$root/crates/cargo-remedy/Cargo.toml" -- "${args[@]}"

if [ "${1:-}" = "serve" ]; then
    port="${PORT:-8080}"
    echo "serving on http://localhost:$port/ (a secure context: service workers and the clipboard work)"
    cd "$out"
    exec python -m http.server "$port" --bind 127.0.0.1
fi
