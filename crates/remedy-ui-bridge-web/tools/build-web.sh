#!/usr/bin/env bash
# Builds a RemedyUI app for the browser into an installable page (a PWA): the WebAssembly
# module and its wasm-bindgen glue, an HTML page, a web app manifest, an icon, and a service
# worker that keeps the page working offline. Optionally serves it on localhost.
#
#   tools/build-web.sh [serve]
#
# Settings (environment):
#   EXAMPLE     the example to build (default web-demo)
#   APP_NAME    the installed app's name (default "RemedyUI demo")
#   SHORT_NAME  its short name (default "RemedyUI")
#   THEME       the theme color for the browser's chrome (default #1e2a24)
#   PROFILE     cargo profile (default release)
#   OUT         output folder (default target/web/<example>, under CARGO_TARGET_DIR when set)
#   PORT        port for `serve` (default 8080)
#
# Needs the wasm32-unknown-unknown target (`rustup target add wasm32-unknown-unknown`) and the
# wasm-bindgen CLI of the version the workspace pins (`cargo install wasm-bindgen-cli
# --version 0.2.130`). Nothing global is changed.
set -euo pipefail

crate="$(cd "$(dirname "$0")/.." && pwd)"
root="$(cd "$crate/../.." && pwd)"
example="${EXAMPLE:-web-demo}"
name="${APP_NAME:-RemedyUI demo}"
short="${SHORT_NAME:-RemedyUI}"
theme="${THEME:-#1e2a24}"
profile="${PROFILE:-release}"
target_dir="${CARGO_TARGET_DIR:-$root/target}"
out="${OUT:-$target_dir/web/$example}"

want="$(grep -A2 '^name = "wasm-bindgen"$' "$root/Cargo.lock" | sed -n 's/^version = "\(.*\)"$/\1/p' | head -n 1)"
have="$(wasm-bindgen --version 2>/dev/null | awk '{print $2}')"
if [ "$want" != "$have" ]; then
    echo "wasm-bindgen CLI $have does not match the pinned $want: cargo install wasm-bindgen-cli --version $want" >&2
    exit 1
fi

cargo build --manifest-path "$crate/Cargo.toml" --example "$example" \
    --target wasm32-unknown-unknown --profile "$profile"
dir="$profile"
[ "$profile" = "dev" ] && dir=debug
wasm="$target_dir/wasm32-unknown-unknown/$dir/examples/${example//-/_}.wasm"

rm -rf "$out"
mkdir -p "$out"
wasm-bindgen --target web --no-typescript --out-name app --out-dir "$out" "$wasm"

# The page, its manifest and icon, with the app's names filled in.
esc() { printf '%s' "$1" | sed -e 's/[\/&]/\\&/g'; }
for f in index.html manifest.webmanifest; do
    sed -e "s/@NAME@/$(esc "$name")/g" -e "s/@SHORT@/$(esc "$short")/g" -e "s/@THEME@/$(esc "$theme")/g" \
        "$crate/tools/page/$f" > "$out/$f"
done
cp "$crate/tools/page/icon.svg" "$out/icon.svg"

# The service worker caches exactly this build: its cache is named after the build's
# contents, so a new build replaces the old cache when the page next loads.
files=(index.html manifest.webmanifest icon.svg app.js app_bg.wasm)
version="$(cat "${files[@]/#/$out/}" | sha256sum | cut -c1-16)"
list="$(printf '"./%s",' "${files[@]}")"
sed -e "s/@VERSION@/$version/g" -e "s|@FILES@|[\"./\",${list%,}]|g" "$crate/tools/page/sw.js" > "$out/sw.js"

echo "built $out (cache $version)"
if [ "${1:-}" = "serve" ]; then
    port="${PORT:-8080}"
    echo "serving on http://localhost:$port/ (a secure context: service workers and the clipboard work)"
    cd "$out"
    exec python -m http.server "$port" --bind 127.0.0.1
fi
