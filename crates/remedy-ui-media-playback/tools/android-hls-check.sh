#!/usr/bin/env bash
# Plays HLS over https on the Android device or emulator adb sees, inside app_process (a real
# Java VM, which the https fetch goes through): builds examples/android-hls-check/, compiles
# tools/HlsCheck.java (a local https server with a self-signed certificate it trusts), pushes
# them with tests/fixtures/hls/aac, runs it and prints the report. No APK, no Gradle.
#
#   tools/android-hls-check.sh
#
# Settings (environment): ANDROID_HOME (required), ANDROID_NDK_HOME, TARGET
# (x86_64-linux-android, default, or aarch64-linux-android), PROFILE (dev, default, or
# release), TARGET_API (default 35). Needs a JDK. Exits non-zero when a check fails.
set -euo pipefail

crate="$(cd "$(dirname "$0")/.." && pwd)"
root="$(cd "$crate/../.." && pwd)"
sdk="${ANDROID_HOME:?set ANDROID_HOME to the Android SDK}"
ndk="${ANDROID_NDK_HOME:-$sdk/ndk/$(ls "$sdk/ndk" | sort -V | tail -n 1)}"
target="${TARGET:-x86_64-linux-android}"
profile="${PROFILE:-dev}"
api="${TARGET_API:-35}"
min_api=26
tools="$sdk/build-tools/$(ls "$sdk/build-tools" | sort -V | tail -n 1)"
jar="$sdk/platforms/android-$api/android.jar"
[ -f "$jar" ] || jar="$(ls -d "$sdk"/platforms/android-*/android.jar | sort -V | tail -n 1)"
target_dir="${CARGO_TARGET_DIR:-$root/target}"
device=/data/local/tmp/remedy-hls-check

exe() { if [ -f "$1.exe" ]; then echo "$1.exe"; elif [ -f "$1.bat" ]; then echo "$1.bat"; else echo "$1"; fi; }
host_tag=$(ls "$ndk/toolchains/llvm/prebuilt" | head -n 1)
bin="$ndk/toolchains/llvm/prebuilt/$host_tag/bin"
clang="$bin/$target$min_api-clang"
[ -f "$clang.cmd" ] && clang="$clang.cmd"
upper=$(echo "$target" | tr 'a-z-' 'A-Z_')
lower=$(echo "$target" | tr '-' '_')
export "CARGO_TARGET_${upper}_LINKER=$clang" "CC_${lower}=$clang" "AR_${lower}=$(exe "$bin/llvm-ar")"

(cd "$root" && cargo build -p remedy-ui-media-playback --example android-hls-check --target "$target" --profile "$profile")
dir="$profile"
[ "$profile" = dev ] && dir=debug

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
javac --release 11 -nowarn -classpath "$jar" -d "$work/classes" "$crate/tools/HlsCheck.java"
"$(exe "$tools/d8")" --min-api "$min_api" --lib "$jar" --output "$work" $(find "$work/classes" -name '*.class')
# The legacy PKCS12 encryption: what Android's KeyStore reads on every version.
keytool -J-Dkeystore.pkcs12.legacy -genkeypair -storetype PKCS12 -keystore "$work/k.p12" \
    -storepass android -keypass android -alias k -keyalg RSA -keysize 2048 -validity 2 \
    -dname "CN=127.0.0.1" -ext "SAN=ip:127.0.0.1" >/dev/null 2>&1
cp "$target_dir/$target/$dir/examples/libandroid_hls_check.so" "$work/"

adb="$(exe "$sdk/platform-tools/adb")"
# Git Bash on Windows would turn the device paths into Windows ones: they are left alone and
# the host's paths are converted here.
export MSYS_NO_PATHCONV=1
host() { if command -v cygpath >/dev/null; then cygpath -w "$1"; else echo "$1"; fi; }
"$adb" shell "rm -rf $device && mkdir -p $device/fixtures"
for f in classes.dex k.p12 libandroid_hls_check.so; do "$adb" push "$(host "$work/$f")" "$device/" >/dev/null; done
"$adb" push "$(host "$crate/tests/fixtures/hls/aac")" "$device/fixtures/" >/dev/null
set +e
out="$("$adb" shell "cd $device && CLASSPATH=$device/classes.dex app_process / HlsCheck \
    $device/libandroid_hls_check.so $device/fixtures $device/k.p12 android 2>&1" | tr -d '\r')"
set -e
"$adb" shell "rm -rf $device"
echo "$out"
echo "$out" | grep -q "hls-https-check: PASSED"
