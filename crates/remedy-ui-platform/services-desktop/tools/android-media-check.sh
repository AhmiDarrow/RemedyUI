#!/usr/bin/env bash
# The media controls on the Android device or emulator adb sees: builds
# examples/android-media-check/, packages it with the Android bridge's dex (MediaControls)
# and an instrumentation (tools/MediaCheck.java), installs it and starts it. While its
# MediaSession is open the script reads it back from `dumpsys media_session` (title,
# subtitle, state, position, actions) and the media notification from `dumpsys notification`,
# presses every button with `cmd media_session dispatch`, and checks each press reached Rust
# as its MediaCommand. Uninstalls the app at the end. No Gradle, as tools/build-apk.sh.
#
#   tools/android-media-check.sh
#
# Settings (environment): ANDROID_HOME (required), ANDROID_NDK_HOME, TARGET
# (x86_64-linux-android, default, or aarch64-linux-android), PROFILE (dev, default, or
# release), TARGET_API (default 35). Needs a JDK. Exits non-zero when a check fails.
set -euo pipefail

crate="$(cd "$(dirname "$0")/.." && pwd)"
root="$(cd "$crate/../.." && pwd)"
bridge="$root/crates/remedy-ui-platform/bridge-android"
sdk="${ANDROID_HOME:?set ANDROID_HOME to the Android SDK}"
ndk="${ANDROID_NDK_HOME:-$sdk/ndk/$(ls "$sdk/ndk" | sort -V | tail -n 1)}"
target="${TARGET:-x86_64-linux-android}"
profile="${PROFILE:-dev}"
api="${TARGET_API:-35}"
min_api=29
tools="$sdk/build-tools/$(ls "$sdk/build-tools" | sort -V | tail -n 1)"
jar="$sdk/platforms/android-$api/android.jar"
target_dir="${CARGO_TARGET_DIR:-$root/target}"
app=dev.remedyui.mediacheck
lib=android_media_check
title="RemedyUI media check"

case "$target" in
    x86_64-linux-android) abi=x86_64 ;;
    aarch64-linux-android) abi=arm64-v8a ;;
    *) echo "unsupported TARGET $target" >&2; exit 2 ;;
esac
exe() { if [ -f "$1.exe" ]; then echo "$1.exe"; elif [ -f "$1.bat" ]; then echo "$1.bat"; else echo "$1"; fi; }
host_tag=$(ls "$ndk/toolchains/llvm/prebuilt" | head -n 1)
bin="$ndk/toolchains/llvm/prebuilt/$host_tag/bin"
clang="$bin/$target$min_api-clang"
[ -f "$clang.cmd" ] && clang="$clang.cmd"
upper=$(echo "$target" | tr 'a-z-' 'A-Z_')
lower=$(echo "$target" | tr '-' '_')
export "CARGO_TARGET_${upper}_LINKER=$clang" "CC_${lower}=$clang" "AR_${lower}=$(exe "$bin/llvm-ar")"

(cd "$root" && cargo build -p remedy-ui-platform --features services-desktop --example android-media-check --target "$target" --profile "$profile")
dir="$profile"
[ "$profile" = dev ] && dir=debug

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/lib/$abi"
cp "$target_dir/$target/$dir/examples/lib$lib.so" "$work/lib/$abi/"
"$(exe "$bin/llvm-strip")" --strip-debug "$work/lib/$abi/lib$lib.so"
# The bridge's dex holds MediaControls; the instrumentation goes in a second dex.
cp "$bridge/dex/classes.dex" "$work/classes.dex"
javac --release 11 -nowarn -classpath "$jar" -d "$work/classes" "$crate/tools/MediaCheck.java"
mkdir -p "$work/check"
"$(exe "$tools/d8")" --min-api "$min_api" --lib "$jar" --output "$work/check" $(find "$work/classes" -name '*.class')
cp "$work/check/classes.dex" "$work/classes2.dex"

cat > "$work/AndroidManifest.xml" <<EOF
<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="$app">
    <uses-sdk android:minSdkVersion="$min_api" android:targetSdkVersion="$api" />
    <uses-permission android:name="android.permission.POST_NOTIFICATIONS" />
    <application android:label="RemedyUI media check" android:hasCode="true" />
    <instrumentation android:name=".MediaCheck" android:targetPackage="$app" />
</manifest>
EOF
"$(exe "$tools/aapt2")" link -o "$work/base.apk" -I "$jar" --manifest "$work/AndroidManifest.xml"
(cd "$work" && "$(exe "$tools/aapt")" add -f base.apk classes.dex classes2.dex "lib/$abi/lib$lib.so" >/dev/null)
"$(exe "$tools/zipalign")" -f -p 4 "$work/base.apk" "$work/aligned.apk"
keytool -genkeypair -keystore "$work/k.jks" -storepass android -keypass android -alias k \
    -keyalg RSA -keysize 2048 -validity 1 -dname "CN=RemedyUI media check" >/dev/null 2>&1
"$(exe "$tools/apksigner")" sign --ks "$work/k.jks" --ks-pass pass:android --key-pass pass:android \
    --out "$work/check.apk" "$work/aligned.apk" 2>/dev/null

adb="$(exe "$sdk/platform-tools/adb")"
pause() { sleep "$1" 2>/dev/null || ping -n "$(($1 + 1))" 127.0.0.1 >/dev/null; }
"$adb" install -r "$work/check.apk" >/dev/null
MSYS_NO_PATHCONV=1 "$adb" shell pm grant "$app" android.permission.POST_NOTIFICATIONS || true
MSYS_NO_PATHCONV=1 "$adb" shell am instrument -w "$app/.MediaCheck" | tr -d '\r' > "$work/report.txt" &
runner=$!

fail=0
check() { if echo "$2" | grep -q -- "$3"; then echo "ok: $1"; else echo "FAILED: $1 (no '$3')"; fail=1; fi; }

# The session as the system holds it: this app's, once it shows the item.
session=""
for _ in $(seq 1 60); do
    MSYS_NO_PATHCONV=1 "$adb" shell dumpsys media_session | tr -d '\r' > "$work/sessions.txt"
    session="$(grep -A12 "package=$app" "$work/sessions.txt" || true)"
    echo "$session" | grep -q "$title" && break
    pause 1
done
echo "--- dumpsys media_session ($app)"
if [ -n "$session" ]; then echo "$session"; else cat "$work/sessions.txt"; fi
check "the session is active" "$session" "active=true"
check "the title and subtitle show" "$session" "$title, S1 E3"
check "it plays" "$session" "state=PLAYING(3)"
check "at 30 s" "$session" "position=30000"
# PLAY_PAUSE, SEEK_TO, FAST_FORWARD, SKIP_TO_NEXT, SKIP_TO_PREVIOUS, REWIND, PLAY, PAUSE, STOP.
check "every button is offered" "$session" "actions=895,"
MSYS_NO_PATHCONV=1 "$adb" shell dumpsys notification --noredact | tr -d '\r' > "$work/notifications.txt"
notes="$(grep -A30 "pkg=$app" "$work/notifications.txt" || true)"
check "the media notification shows" "$notes" "MediaStyle\|template=android.app.Notification\$MediaStyle"

for key in play pause play-pause next previous fast-forward rewind stop; do
    MSYS_NO_PATHCONV=1 "$adb" shell cmd media_session dispatch "$key" | tr -d '\r'
    pause 1
done
wait "$runner" || true
if [ -n "${EVIDENCE:-}" ]; then mkdir -p "$EVIDENCE" && cp "$work/sessions.txt" "$work/notifications.txt" "$EVIDENCE/"; fi
"$adb" uninstall "$app" >/dev/null

report="$(cat "$work/report.txt")"
echo "--- the instrumentation"
echo "$report"
check "Play arrives" "$report" "^Play$"
check "Pause arrives" "$report" "^Pause$"
check "Next arrives" "$report" "^Next$"
check "Previous arrives" "$report" "^Previous$"
check "fast-forward arrives as SeekBy(10.0)" "$report" "^SeekBy(10.0)$"
check "rewind arrives as SeekBy(-10.0)" "$report" "^SeekBy(-10.0)$"
check "Stop arrives" "$report" "^Stop$"
# play-pause: Android's MediaSession turns it into Play or Pause by the state shown.
[ "$(echo "$report" | grep -c -E '^(Play|Pause)$')" -ge 3 ] && echo "ok: play-pause arrives" || { echo "FAILED: play-pause"; fail=1; }
[ "$fail" = 0 ] && echo "media-check: PASSED" || echo "media-check: FAILED"
exit "$fail"
