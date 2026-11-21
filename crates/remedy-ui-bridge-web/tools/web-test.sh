#!/usr/bin/env bash
# Runs the browser tests in a headless browser with wasm-bindgen-test-runner: the web
# bridge's (`tests/browser.rs`), media playback's (`remedy-ui-media-playback/tests/web.rs`, and
# `web_gpu.rs` for pictures on the GPU, `web_cursor.rs` for the pointer over a resting full
# screen) and the script engines' on a board (`remedy-ui-widgets-board/tests/web.rs`).
#
#   tools/web-test.sh [firefox|chrome]
#
# Needs the wasm32-unknown-unknown target, the wasm-bindgen CLI of the pinned version (which
# brings wasm-bindgen-test-runner), and the browser's WebDriver: geckodriver for Firefox
# (`cargo install geckodriver`), chromedriver matching the installed Chrome for Chrome. Set
# GECKODRIVER or CHROMEDRIVER to the driver's path when it is not on PATH. The runner is set
# for this one run only.
#
# `tools/webdriver.json` (handed to the runner) lets the test
# pages play sound without a click first, as a page may once its person has used it.
set -euo pipefail

crate="$(cd "$(dirname "$0")/.." && pwd)"
root="$(cd "$crate/../.." && pwd)"
browser="${1:-firefox}"
case "$browser" in
    firefox) export GECKODRIVER="${GECKODRIVER:-$(command -v geckodriver)}" ;;
    chrome) export CHROMEDRIVER="${CHROMEDRIVER:-$(command -v chromedriver)}" ;;
    *) echo "usage: $0 [firefox|chrome]" >&2; exit 2 ;;
esac
export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner
# Long enough for a software GPU to make its first device.
export WASM_BINDGEN_TEST_TIMEOUT="${WASM_BINDGEN_TEST_TIMEOUT:-120}"
export WASM_BINDGEN_TEST_WEBDRIVER_JSON="$crate/tools/webdriver.json"
cargo test --manifest-path "$crate/Cargo.toml" --target wasm32-unknown-unknown --test browser
# The media test's slow download: the clip served a chunk at a time from this machine.
port_file="$(mktemp)"
python3 "$root/crates/remedy-ui-media-playback/tools/slow-serve.py" \
    "$root/crates/remedy-ui-media-playback/tests/fixtures/clip.mp4" > "$port_file" &
slow=$!
# And the HLS clip: a playlist and its segments, served as they are.
hls_port_file="$(mktemp)"
python3 "$root/crates/remedy-ui-media-playback/tools/serve-dir.py" \
    "$root/crates/remedy-ui-media-playback/tests/fixtures/hls" > "$hls_port_file" &
hls=$!
trap 'kill $slow $hls 2>/dev/null; rm -f "$port_file" "$hls_port_file"' EXIT
for _ in $(seq 50); do [ -s "$port_file" ] && [ -s "$hls_port_file" ] && break; sleep 0.1; done
REMEDY_HLS_URL="http://127.0.0.1:$(cat "$hls_port_file")" \
REMEDY_SLOW_CLIP_URL="http://127.0.0.1:$(cat "$port_file")/clip.mp4" \
    cargo test --manifest-path "$root/crates/remedy-ui-media-playback/Cargo.toml" --target wasm32-unknown-unknown --test web
cargo test --manifest-path "$root/crates/remedy-ui-media-playback/Cargo.toml" --target wasm32-unknown-unknown --test web_gpu
cargo test --manifest-path "$root/crates/remedy-ui-media-playback/Cargo.toml" --target wasm32-unknown-unknown --test web_cursor
cargo test --manifest-path "$root/crates/remedy-ui-widgets-board/Cargo.toml" --target wasm32-unknown-unknown --test web
cargo test --manifest-path "$root/crates/remedy-ui-services-desktop/Cargo.toml" --target wasm32-unknown-unknown --test media_session_web
