#!/usr/bin/env bash
# Builds a RemedyUI app for Android into a signed APK, with no Gradle: cargo with the NDK's
# clang as linker, aapt2 for the manifest, zipalign and apksigner. Optionally installs and
# starts it on the device or emulator adb sees.
#
#   tools/build-apk.sh [install|run]
#
# Settings (environment):
#   ANDROID_HOME     the Android SDK (platforms, build-tools, ndk)            (required)
#   ANDROID_NDK_HOME the NDK (default: the newest under $ANDROID_HOME/ndk)
#   TARGET           Rust target: x86_64-linux-android (emulators, default) or aarch64-linux-android
#   EXAMPLE          the example to package (default android-demo)
#   APP_ID           the package name (default dev.remedyui.demo)
#   APP_LABEL        the launcher label (default "RemedyUI demo")
#   PROFILE          cargo profile (default release)
#   MIN_API          lowest Android API level (default 26: the bridge loads its Java half
#                    with InMemoryDexClassLoader); TARGET_API (default 35)
#
# Everything is set for this one cargo run only: no global cargo or shell configuration
# changes. Output goes to target/android/ (under CARGO_TARGET_DIR when set).
set -euo pipefail

crate="$(cd "$(dirname "$0")/.." && pwd)"
root="$(cd "$crate/../.." && pwd)"
sdk="${ANDROID_HOME:?set ANDROID_HOME to the Android SDK}"
ndk="${ANDROID_NDK_HOME:-$sdk/ndk/$(ls "$sdk/ndk" | sort -V | tail -n 1)}"
target="${TARGET:-x86_64-linux-android}"
example="${EXAMPLE:-android-demo}"
app_id="${APP_ID:-dev.remedyui.demo}"
label="${APP_LABEL:-RemedyUI demo}"
profile="${PROFILE:-release}"
min_api="${MIN_API:-26}"
target_api="${TARGET_API:-35}"
tools="$sdk/build-tools/$(ls "$sdk/build-tools" | sort -V | tail -n 1)"
jar="$sdk/platforms/android-$target_api/android.jar"
target_dir="${CARGO_TARGET_DIR:-$root/target}"

case "$target" in
    x86_64-linux-android) abi=x86_64 ;;
    aarch64-linux-android) abi=arm64-v8a ;;
    *) echo "unsupported TARGET $target" >&2; exit 2 ;;
esac

host_tag=$(ls "$ndk/toolchains/llvm/prebuilt" | head -n 1)
bin="$ndk/toolchains/llvm/prebuilt/$host_tag/bin"
clang="$bin/$target$min_api-clang"
[ -f "$clang.cmd" ] && clang="$clang.cmd"
clangxx="$bin/$target$min_api-clang++"
[ -f "$clangxx.cmd" ] && clangxx="$clangxx.cmd"
exe() { if [ -f "$1.exe" ]; then echo "$1.exe"; elif [ -f "$1.bat" ]; then echo "$1.bat"; else echo "$1"; fi; }
ar="$(exe "$bin/llvm-ar")"

# Cargo and cc read these as CARGO_TARGET_<TRIPLE>_LINKER, CC_<triple>, CXX_<triple> and AR_<triple>.
upper=$(echo "$target" | tr 'a-z-' 'A-Z_')
lower=$(echo "$target" | tr '-' '_')
export "CARGO_TARGET_${upper}_LINKER=$clang"
export "CC_${lower}=$clang"
export "CXX_${lower}=$clangxx"
export "AR_${lower}=$ar"

(cd "$root" && cargo build -p remedy-ui-bridge-android --example "$example" --target "$target" --profile "$profile")
dir="$profile"
[ "$profile" = dev ] && dir=debug
lib_name=$(echo "$example" | tr '-' '_')
so="$target_dir/$target/$dir/examples/lib$lib_name.so"

out="$target_dir/android"
stage="$out/stage-$example"
rm -rf "$stage"
mkdir -p "$stage/lib/$abi"
cp "$so" "$stage/lib/$abi/"
"$(exe "$bin/llvm-strip")" --strip-debug "$stage/lib/$abi/lib$lib_name.so"

cat > "$stage/AndroidManifest.xml" <<EOF
<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android"
    package="$app_id" android:versionCode="1" android:versionName="0.1.0">
    <uses-sdk android:minSdkVersion="$min_api" android:targetSdkVersion="$target_api" />
    <application android:label="$label" android:hasCode="false">
        <!-- The configuration changes RemedyUI follows in place, instead of the activity
             being recreated: the bridge reports them as settings and sizes. -->
        <activity android:name="android.app.NativeActivity"
            android:exported="true"
            android:configChanges="orientation|screenSize|screenLayout|smallestScreenSize|keyboard|keyboardHidden|navigation|uiMode|fontScale|locale|layoutDirection|density"
            android:windowSoftInputMode="adjustResize"
            android:theme="@android:style/Theme.DeviceDefault.NoActionBar">
            <meta-data android:name="android.app.lib_name" android:value="$lib_name" />
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
        </activity>
    </application>
</manifest>
EOF

"$(exe "$tools/aapt2")" link -o "$stage/base.apk" -I "$jar" --manifest "$stage/AndroidManifest.xml"
(cd "$stage" && "$(exe "$tools/aapt")" add -f base.apk "lib/$abi/lib$lib_name.so" >/dev/null)
"$(exe "$tools/zipalign")" -f -p 4 "$stage/base.apk" "$stage/aligned.apk"

# A debug key of this build's own (never committed), made once.
ks="$out/debug.keystore"
if [ ! -f "$ks" ]; then
    keytool -genkeypair -keystore "$ks" -storepass android -keypass android -alias debug \
        -keyalg RSA -keysize 2048 -validity 10000 -dname "CN=RemedyUI debug" >/dev/null
fi
apk="$out/$example.apk"
"$(exe "$tools/apksigner")" sign --ks "$ks" --ks-pass pass:android --key-pass pass:android \
    --out "$apk" "$stage/aligned.apk"
rm -rf "$stage"
echo "built $apk"

adb="$(exe "$sdk/platform-tools/adb")"
case "${1:-}" in
    install) "$adb" install -r "$apk" ;;
    run)
        "$adb" install -r "$apk"
        "$adb" shell am start -n "$app_id/android.app.NativeActivity"
        ;;
    "") ;;
    *) echo "unknown step $1 (install or run)" >&2; exit 2 ;;
esac
