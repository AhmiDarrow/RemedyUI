#!/usr/bin/env bash
# Drives the android-demo app on a running emulator or device (installed with
# `tools/build-apk.sh install`) through what the bridge does, and keeps the evidence:
# screenshots, the accessibility tree as Android sees it, and the app's log.
#
#   tools/emulator-check.sh <evidence-dir>
#
# Needs ANDROID_HOME; uses the one device adb sees (or ANDROID_SERIAL). It finds controls by
# their accessible names, so it works at any screen size. It changes three system settings
# (animator duration scale, night mode, font scale) and puts them back at the end.
set -u
out="${1:?usage: emulator-check.sh <evidence-dir>}"
mkdir -p "$out"
export MSYS_NO_PATHCONV=1
adb="${ANDROID_HOME:?set ANDROID_HOME}/platform-tools/adb"
app=dev.remedyui.demo/${ACTIVITY_CLASS:-dev.remedyui.android.RemedyActivity}
pause() { sleep "$1" 2>/dev/null || ping -n "$(($1 + 1))" 127.0.0.1 >/dev/null; }
step() { echo "=== $*"; }
shot() { "$adb" exec-out screencap -p > "$out/$1.png"; }

# The accessibility tree, one node per line: class, name, text, focus, bounds.
tree() {
    "$adb" shell uiautomator dump /sdcard/remedyui-ui.xml >/dev/null 2>&1
    "$adb" shell cat /sdcard/remedyui-ui.xml | tr '<' '\n' | grep 'class=' |
        sed -E 's/.*text="([^"]*)".*class="([^"]*)".*content-desc="([^"]*)".*focused="([^"]*)".*bounds="([^"]*)".*/\2|\3|\1|\4|\5/'
}

# Taps the control assistive technology knows as $1, at the centre of its bounds.
tap() {
    local b
    b=$(tree | awk -F'|' -v n="$1" '$2 == n || $3 == n { print $5; exit }')
    if [ -z "$b" ]; then
        echo "no control named '$1'"
        return 1
    fi
    read -r x0 y0 x1 y1 <<<"$(echo "$b" | tr -c '0-9' ' ')"
    "$adb" shell input tap $(((x0 + x1) / 2)) $(((y0 + y1) / 2))
}

"$adb" shell am force-stop "${app%%/*}"
"$adb" logcat -c
"$adb" shell am start -n "$app" >/dev/null
pause 6
tree >/dev/null # a screen reader connecting asks the app for its tree
pause 2
shot 1-start
tree > "$out/tree-start.txt"

step "tap 'Tap me' twice"
tap "Tap me"; pause 1
tap "Tap me"; pause 1

step "focus 'Message' and type"
tap "Message"; pause 2
"$adb" shell dumpsys input_method | grep -m1 -o "mInputShown=[a-z]*"
"$adb" shell input text "hello"; pause 1
shot 2-typed-with-keyboard
step "back while the keyboard is up hides it"
"$adb" shell input keyevent KEYCODE_BACK; pause 2
"$adb" shell dumpsys input_method | grep -m1 -o "mInputShown=[a-z]*"
tree > "$out/tree-typed.txt"

step "copy (sensitive), then read the clipboard back"
tap "Copy"; pause 1
tap "Read clipboard"; pause 1

step "capture protection on: the window is SECURE and screenshots of it fail or come out black"
tap "Protect"; pause 2
"$adb" shell dumpsys window | grep -m1 -o "fl=.*SECURE[A-Z_ ]*" || echo "the window is not SECURE"
"$adb" shell screencap -p /sdcard/remedyui-shot.png && echo "screencap succeeded (Android blacks out secure windows)" ||
    echo "screencap refused the secure window"
"$adb" shell rm -f /sdcard/remedyui-shot.png
step "capture protection off"
tap "Protect"; pause 2
shot 4-unprotected

step "memory trim, then setting changes"
pid=$("$adb" shell pidof "${app%%/*}" | tr -d '\r')
"$adb" shell am send-trim-memory "$pid" RUNNING_CRITICAL; pause 1
"$adb" shell settings put global animator_duration_scale 0; pause 2
"$adb" shell cmd uimode night yes; pause 3
"$adb" shell settings put system font_scale 1.3; pause 3
shot 5-night-large-text
"$adb" shell settings put system font_scale 1.0
"$adb" shell cmd uimode night no
"$adb" shell settings put global animator_duration_scale 1; pause 3

step "lifecycle: home, then back"
"$adb" shell input keyevent KEYCODE_HOME; pause 3
"$adb" shell am start -n "$app" >/dev/null; pause 4
shot 6-resumed

step "open a link"
tap "Open link"; pause 5
"$adb" shell dumpsys activity activities | grep -m1 -o "topResumedActivity=[^ ]* [^ ]* [^ ]*"
"$adb" shell am start -n "$app" >/dev/null; pause 4

step "open a document through the system picker"
"$adb" shell 'echo "Hello from a document picked through SAF" > /sdcard/Download/remedyui-open.txt'
tap "Open file"; pause 4
tap "Show roots"; pause 2
tap "Downloads"; pause 3
tap "remedyui-open.txt"; pause 4
shot 7-after-open

step "save a document through the system picker"
tap "Save file"; pause 4
tap "Show roots"; pause 2
tap "Downloads"; pause 3
tap "SAVE"; pause 4
"$adb" shell cat /sdcard/Download/remedyui-saved.txt

step "share through the chooser (the file's name shows in its preview), then back out"
tap "Share"; pause 4
shot 7-share-chooser
tree | grep -m1 "remedyui-shared.json" || echo "the chooser does not show the shared file"
"$adb" shell input keyevent KEYCODE_BACK; pause 3
"$adb" logcat -d | grep -m1 "share ended" || echo "the share did not end"

step "screen-reader actions: click, focus, set text"
here="$(cd "$(dirname "$0")" && pwd)"
env -u MSYS_NO_PATHCONV bash "$here/a11y-check/run.sh" | grep -E "INSTRUMENTATION_RESULT|INSTRUMENTATION_CODE"

step "renderer: CPU first, then the GPU with the next surface"
"$adb" shell am force-stop "${app%%/*}"
"$adb" shell am start -n "$app" --es renderer cpu-first >/dev/null; pause 6
for i in 1 2 3; do "$adb" shell input swipe 540 1500 540 900 300; done
"$adb" shell input keyevent KEYCODE_HOME; pause 3
"$adb" shell am start -n "$app" >/dev/null; pause 5
shot 8-gpu-after-resume

step "back with nothing to dismiss leaves the app"
"$adb" shell input keyevent KEYCODE_BACK; pause 3
echo "app process after back: '$("$adb" shell pidof "${app%%/*}" | tr -d '\r')'"

"$adb" logcat -d | grep -E "RemedyUI|RustPanic|FATAL EXCEPTION" > "$out/logcat.txt"
"$adb" shell rm -f /sdcard/remedyui-ui.xml
echo "evidence in $out"
