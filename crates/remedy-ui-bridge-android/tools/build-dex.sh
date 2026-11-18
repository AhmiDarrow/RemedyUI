#!/usr/bin/env bash
# Rebuilds dex/classes.dex from java/ (the bridge's Java half). Run after changing the Java
# source and commit the result; building the bridge itself needs no Java tools.
#
# Needs: a JDK (javac), and ANDROID_HOME with platforms/android-<API> and build-tools/<version>.
set -euo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"
sdk="${ANDROID_HOME:?set ANDROID_HOME to the Android SDK}"
api="${ANDROID_API:-35}"
tools="${ANDROID_BUILD_TOOLS:-$(ls "$sdk/build-tools" | sort -V | tail -n 1)}"
jar="$sdk/platforms/android-$api/android.jar"
d8="$sdk/build-tools/$tools/d8"
[ -f "$d8" ] || d8="$d8.bat"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
javac --release 11 -nowarn -classpath "$jar" -d "$work/classes" \
    $(find "$here/java" -name '*.java')
# The bridge loads the dex from memory (InMemoryDexClassLoader, Android 8.0, API 26).
"$d8" --release --min-api 26 --lib "$jar" --output "$work" $(find "$work/classes" -name '*.class')
mkdir -p "$here/dex"
cp "$work/classes.dex" "$here/dex/classes.dex"
echo "wrote $here/dex/classes.dex"
