#!/usr/bin/env bash
# Builds the accessibility check (an instrumentation package, no Gradle) and runs it against
# the android-demo app on the device adb sees. Prints what each screen-reader action returned.
#
# Needs ANDROID_HOME and a JDK; the demo must be installed (tools/build-apk.sh install).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
sdk="${ANDROID_HOME:?set ANDROID_HOME}"
api="${TARGET_API:-35}"
tools="$sdk/build-tools/$(ls "$sdk/build-tools" | sort -V | tail -n 1)"
jar="$sdk/platforms/android-$api/android.jar"
exe() { if [ -f "$1.exe" ]; then echo "$1.exe"; elif [ -f "$1.bat" ]; then echo "$1.bat"; else echo "$1"; fi; }
adb="$(exe "$sdk/platform-tools/adb")"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cat > "$work/AndroidManifest.xml" <<EOF
<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="dev.remedyui.a11ycheck">
    <uses-sdk android:minSdkVersion="26" android:targetSdkVersion="$api" />
    <application android:label="RemedyUI accessibility check" />
    <instrumentation android:name=".A11yCheck" android:targetPackage="dev.remedyui.a11ycheck" />
</manifest>
EOF
javac --release 11 -nowarn -classpath "$jar" -d "$work/classes" "$here/A11yCheck.java"
"$(exe "$tools/d8")" --min-api 26 --lib "$jar" --output "$work" $(find "$work/classes" -name '*.class')
"$(exe "$tools/aapt2")" link -o "$work/base.apk" -I "$jar" --manifest "$work/AndroidManifest.xml"
(cd "$work" && "$(exe "$tools/aapt")" add -f base.apk classes.dex >/dev/null)
"$(exe "$tools/zipalign")" -f -p 4 "$work/base.apk" "$work/aligned.apk"
keytool -genkeypair -keystore "$work/k.jks" -storepass android -keypass android -alias k \
    -keyalg RSA -keysize 2048 -validity 1 -dname "CN=RemedyUI a11y check" >/dev/null 2>&1
"$(exe "$tools/apksigner")" sign --ks "$work/k.jks" --ks-pass pass:android --key-pass pass:android \
    --out "$work/check.apk" "$work/aligned.apk" 2>/dev/null
"$adb" install -r "$work/check.apk" >/dev/null
MSYS_NO_PATHCONV=1 "$adb" shell am instrument -w -r dev.remedyui.a11ycheck/.A11yCheck
MSYS_NO_PATHCONV=1 "$adb" uninstall dev.remedyui.a11ycheck >/dev/null
