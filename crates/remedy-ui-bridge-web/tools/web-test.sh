#!/usr/bin/env bash
# Runs the browser tests in a headless browser with wasm-bindgen-test-runner: the web
# bridge's (`tests/browser.rs`; and `remedy-ui/tests/web_assets.rs` for what a page fetches or
# reads rather than carries: fonts, the system font, language names, plain storage items), the HTTP client's (`remedy-ui-platform/tests/services_net/web.rs`), images
# from addresses (`remedy-ui-media/tests/media_image/web.rs`), media playback's (`remedy-ui-media-playback/tests/web.rs`, and
# `web_gpu.rs` for pictures on the GPU, `web_cursor.rs` for the pointer over a resting full
# screen, `web_mse.rs` and `web_mse_gpu.rs` for the media element through Media Source
# Extensions) and the script engines' on a board (`remedy-ui-widgets/tests/board/web.rs`).
#
# The media element's tests run twice: on the runner's own page (a secure context, so they
# ask for the element), and on a page served from this machine's LAN address, which is not
# one (no WebCodecs, no WebGPU), where the player must choose the element by itself and the
# bridge draws with WebGL2. `REMEDY_LAN_ADDRESS` names the address when the one found (the
# address of the route out) is not the one to use; `REMEDY_LAN_ADDRESS=none` skips that run.
#
#   tools/web-test.sh [firefox|chrome]
#
# Needs the wasm32-unknown-unknown target, the wasm-bindgen CLI of the pinned version (which
# brings wasm-bindgen-test-runner), and the browser's WebDriver: geckodriver for Firefox
# (`cargo install geckodriver`), chromedriver matching the installed Chrome for Chrome. Set
# GECKODRIVER or CHROMEDRIVER to the driver's path when it is not on PATH. The runner is set
# for this one run only.
#
# `tools/webdriver.json` (handed to the runner) lets the test pages play sound without a
# click first, as a page may once its person has used it, and open windows (the router's
# new-tab links) without one.
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
# The fallback faces fetched from beside the page: remedy-ui-fonts' `fonts/` stands in for the
# folder `cargo remedy package` writes.
fonts_port_file="$(mktemp)"
python3 "$root/crates/remedy-ui-media-playback/tools/serve-dir.py" "$root/crates/remedy-ui-fonts" > "$fonts_port_file" &
fonts=$!
for _ in $(seq 50); do [ -s "$fonts_port_file" ] && break; sleep 0.1; done
REMEDY_FONTS_URL="http://127.0.0.1:$(cat "$fonts_port_file")/" \
    cargo test --manifest-path "$root/crates/remedy-ui/Cargo.toml" --target wasm32-unknown-unknown --test web_assets
kill $fonts 2>/dev/null; rm -f "$fonts_port_file"
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
# And the HTTP client's test server: JSON, errors, echoes and a slow answer.
net_port_file="$(mktemp)"
python3 "$root/crates/remedy-ui-platform/services-net/tools/test-serve.py" > "$net_port_file" &
net=$!
# And images from another origin: with CORS, without, through a proxy, and a trickle to abort.
img_port_file="$(mktemp)"
python3 "$root/crates/remedy-ui-media/media-image/tools/test-serve.py" > "$img_port_file" &
img=$!
trap 'kill $slow $hls $net $img 2>/dev/null; rm -f "$port_file" "$hls_port_file" "$net_port_file" "$img_port_file"' EXIT
for _ in $(seq 50); do [ -s "$port_file" ] && [ -s "$hls_port_file" ] && [ -s "$net_port_file" ] && [ -s "$img_port_file" ] && break; sleep 0.1; done
REMEDY_NET_URL="http://127.0.0.1:$(cat "$net_port_file")" \
    cargo test --manifest-path "$root/crates/remedy-ui-platform/services-net/Cargo.toml" --target wasm32-unknown-unknown --test web
REMEDY_IMAGE_URL="http://127.0.0.1:$(cat "$img_port_file")" \
    cargo test --manifest-path "$root/crates/remedy-ui-media/media-image/Cargo.toml" --target wasm32-unknown-unknown --test web
REMEDY_HLS_URL="http://127.0.0.1:$(cat "$hls_port_file")" \
REMEDY_SLOW_CLIP_URL="http://127.0.0.1:$(cat "$port_file")/clip.mp4" \
    cargo test --manifest-path "$root/crates/remedy-ui-media-playback/Cargo.toml" --target wasm32-unknown-unknown --test web
cargo test --manifest-path "$root/crates/remedy-ui-media-playback/Cargo.toml" --target wasm32-unknown-unknown --test web_gpu
cargo test --manifest-path "$root/crates/remedy-ui-media-playback/Cargo.toml" --target wasm32-unknown-unknown --test web_cursor
REMEDY_HLS_URL="http://127.0.0.1:$(cat "$hls_port_file")" \
    cargo test --manifest-path "$root/crates/remedy-ui-media-playback/Cargo.toml" --target wasm32-unknown-unknown --test web_mse
# A software GPU draws a debug build's frames slowly: the GPU test is given longer.
REMEDY_HLS_URL="http://127.0.0.1:$(cat "$hls_port_file")" WASM_BINDGEN_TEST_TIMEOUT=400 \
    cargo test --manifest-path "$root/crates/remedy-ui-media-playback/Cargo.toml" --target wasm32-unknown-unknown --test web_mse_gpu
# The same from the LAN address: the page and the HLS clip both served there.
lan="${REMEDY_LAN_ADDRESS:-$(python3 -c 'import socket; s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.connect(("192.0.2.1", 9)); print(s.getsockname()[0])' 2>/dev/null || true)}"
if [ -n "$lan" ] && [ "$lan" != none ] && [ "${lan#127.}" = "$lan" ]; then
    lan_hls_file="$(mktemp)"
    python3 "$root/crates/remedy-ui-media-playback/tools/serve-dir.py" \
        "$root/crates/remedy-ui-media-playback/tests/fixtures/hls" "$lan" > "$lan_hls_file" &
    lan_hls=$!
    trap 'kill $slow $hls $net $img $lan_hls 2>/dev/null; rm -f "$port_file" "$hls_port_file" "$net_port_file" "$img_port_file" "$lan_hls_file"' EXIT
    for _ in $(seq 50); do [ -s "$lan_hls_file" ] && break; sleep 0.1; done
    free_port() { python3 -c 'import socket, sys; s = socket.socket(); s.bind((sys.argv[1], 0)); print(s.getsockname()[1])' "$lan"; }
    for t in web_mse web_mse_gpu; do
        REMEDY_MSE_INSECURE=1 REMEDY_HLS_URL="http://$lan:$(cat "$lan_hls_file")" \
        WASM_BINDGEN_TEST_ADDRESS="$lan:$(free_port)" WASM_BINDGEN_TEST_TIMEOUT=400 \
            cargo test --manifest-path "$root/crates/remedy-ui-media-playback/Cargo.toml" --target wasm32-unknown-unknown --test "$t"
    done
else
    echo "no LAN address (REMEDY_LAN_ADDRESS): skipping the media element on a page that is not secure"
fi
cargo test --manifest-path "$root/crates/remedy-ui-widgets/widgets-board/Cargo.toml" --target wasm32-unknown-unknown --test web
cargo test --manifest-path "$root/crates/remedy-ui-platform/services-desktop/Cargo.toml" --target wasm32-unknown-unknown --test media_session_web
# The Screen Wake Lock: held, let go, asked again when the page shows; and a player's hold.
cargo test --manifest-path "$root/crates/remedy-ui-platform/services-desktop/Cargo.toml" --target wasm32-unknown-unknown --test keep_awake_web
cargo test --manifest-path "$root/crates/remedy-ui-media-playback/Cargo.toml" --target wasm32-unknown-unknown --features keep-awake --test web_keep_awake
