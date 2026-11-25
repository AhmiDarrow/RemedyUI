#!/usr/bin/env bash
# Runs the isolated-decoding tests (tests/isolated.rs) on the Android device or emulator adb
# sees, inside an app whose helpers are the bridge's isolated decoding service: builds
# examples/android-isolated-check/, packages it with the bridge's dex (the service) and an
# instrumentation (tools/IsolatedCheck.java), installs it, runs it, prints the report and
# uninstalls it. No Gradle, as tools/build-apk.sh in the Android bridge.
#
#   tools/android-check.sh [test-name-filter...]
#
# Settings (environment): ANDROID_HOME (required), ANDROID_NDK_HOME, TARGET
# (x86_64-linux-android, default, or aarch64-linux-android), PROFILE (dev, default, or
# release), TARGET_API (default 35). Needs a JDK. Exits non-zero when a test fails.
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
app=dev.remedyui.isolatedcheck
lib=android_isolated_check

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

(cd "$root" && cargo build -p remedy-ui-media --features media-isolated --example android-isolated-check --target "$target" --profile "$profile")
dir="$profile"
[ "$profile" = dev ] && dir=debug

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/lib/$abi"
cp "$target_dir/$target/$dir/examples/lib$lib.so" "$work/lib/$abi/"
"$(exe "$bin/llvm-strip")" --strip-debug "$work/lib/$abi/lib$lib.so"
# The bridge's dex holds the decoding service; the instrumentation goes in a second dex.
cp "$bridge/dex/classes.dex" "$work/classes.dex"
javac --release 11 -nowarn -classpath "$jar" -d "$work/classes" "$crate/tools/IsolatedCheck.java"
mkdir -p "$work/check"
"$(exe "$tools/d8")" --min-api "$min_api" --lib "$jar" --output "$work/check" $(find "$work/classes" -name '*.class')
cp "$work/check/classes.dex" "$work/classes2.dex"

# INTERNET only so the app's own connection (the test's control) can reach its listener;
# the isolated helpers get no permissions whatever the app has.
cat > "$work/AndroidManifest.xml" <<EOF
<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="$app">
    <uses-sdk android:minSdkVersion="$min_api" android:targetSdkVersion="$api" />
    <uses-permission android:name="android.permission.INTERNET" />
    <application android:label="RemedyUI isolated decoding check" android:hasCode="true">
        <service android:name="dev.remedyui.android.Decoder"
            android:isolatedProcess="true"
            android:exported="false" />
    </application>
    <instrumentation android:name=".IsolatedCheck" android:targetPackage="$app" />
</manifest>
EOF
"$(exe "$tools/aapt2")" link -o "$work/base.apk" -I "$jar" --manifest "$work/AndroidManifest.xml"
(cd "$work" && "$(exe "$tools/aapt")" add -f base.apk classes.dex classes2.dex "lib/$abi/lib$lib.so" >/dev/null)
"$(exe "$tools/zipalign")" -f -p 4 "$work/base.apk" "$work/aligned.apk"
keytool -genkeypair -keystore "$work/k.jks" -storepass android -keypass android -alias k \
    -keyalg RSA -keysize 2048 -validity 1 -dname "CN=RemedyUI isolated check" >/dev/null 2>&1
"$(exe "$tools/apksigner")" sign --ks "$work/k.jks" --ks-pass pass:android --key-pass pass:android \
    --out "$work/check.apk" "$work/aligned.apk" 2>/dev/null

adb="$(exe "$sdk/platform-tools/adb")"
"$adb" install -r "$work/check.apk" >/dev/null
# (Git Bash on Windows would turn the device paths into Windows ones.)
out="$(MSYS_NO_PATHCONV=1 "$adb" shell am instrument -w -e filter "'$*'" "$app/.IsolatedCheck" | tr -d '\r')"
"$adb" uninstall "$app" >/dev/null
echo "$out"
echo "$out" | grep -q "isolated-check: PASSED"
