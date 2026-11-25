#!/usr/bin/env bash
# Drives the android-demo app's notifications and foreground work on a running emulator or
# device (installed with `tools/build-apk.sh install`), the way a person would: the
# permission prompts, the notification shade, its buttons, pressing a notification, swiping
# one away, the app going to the background during a "call". Keeps the evidence: what
# Android's notification and activity services report, screenshots, and the app's log.
#
#   tools/notify-check.sh <evidence-dir>
#
# Settings (environment): ACTIVITY_CLASS, the activity the APK runs in (default
# dev.remedyui.android.RemedyActivity; android.app.NativeActivity for an APK built with
# ACTIVITY=native, where foreground work is expected to be refused). Needs ANDROID_HOME; uses
# the one device adb sees (or ANDROID_SERIAL).
set -u
out="${1:?usage: notify-check.sh <evidence-dir>}"
mkdir -p "$out"
export MSYS_NO_PATHCONV=1
adb="${ANDROID_HOME:?set ANDROID_HOME}/platform-tools/adb"
pkg=dev.remedyui.demo
activity="${ACTIVITY_CLASS:-dev.remedyui.android.RemedyActivity}"
app="$pkg/$activity"
pause() { sleep "$1" 2>/dev/null || ping -n "$(($1 + 1))" 127.0.0.1 >/dev/null; }
step() { echo "=== $*"; }
shot() { "$adb" exec-out screencap -p > "$out/$1.png"; }
sdk=$("$adb" shell getprop ro.build.version.sdk | tr -d '\r')
echo "device API level $sdk, activity $activity"

# The screen's nodes, one per line: class, name, text, focus, bounds.
tree() {
    "$adb" shell uiautomator dump /sdcard/remedyui-ui.xml >/dev/null 2>&1
    "$adb" shell cat /sdcard/remedyui-ui.xml | tr '<' '\n' | grep 'class=' |
        sed -E 's/.*text="([^"]*)".*class="([^"]*)".*content-desc="([^"]*)".*focused="([^"]*)".*bounds="([^"]*)".*/\2|\3|\1|\4|\5/'
}

# The bounds of the first node whose name or text is $1 (case-insensitive).
bounds() {
    tree | awk -F'|' -v n="$1" 'tolower($2) == tolower(n) || tolower($3) == tolower(n) { print $5; exit }'
}

# Taps the node named $1 at the centre of its bounds.
tap() {
    local b
    b=$(bounds "$1")
    if [ -z "$b" ]; then
        echo "no control named '$1'"
        return 1
    fi
    read -r x0 y0 x1 y1 <<<"$(echo "$b" | tr -c '0-9' ' ')"
    "$adb" shell input tap $(((x0 + x1) / 2)) $(((y0 + y1) / 2))
}

# Swipes the node named $1 off to the right (dismissing a notification).
swipe_away() {
    local b
    b=$(bounds "$1")
    if [ -z "$b" ]; then
        echo "no notification showing '$1'"
        return 1
    fi
    read -r x0 y0 x1 y1 <<<"$(echo "$b" | tr -c '0-9' ' ')"
    local y=$(((y0 + y1) / 2)) w
    w=$("$adb" shell wm size | grep -o '[0-9]*x' | tail -n 1 | tr -d x)
    # Slow enough that the shade takes it as a drag, not a tap.
    "$adb" shell input swipe $((w / 5)) "$y" $((w - 80)) "$y" 600
}

# Opens up the notification showing $1 (a quiet one is shown folded, its buttons hidden):
# a finger drawn down over it.
expand() {
    local b
    b=$(bounds "$1")
    if [ -z "$b" ]; then
        echo "no notification showing '$1'"
        return 1
    fi
    read -r x0 y0 x1 y1 <<<"$(echo "$b" | tr -c '0-9' ' ')"
    "$adb" shell input swipe $(((x0 + x1) / 2)) "$y0" $(((x0 + x1) / 2)) $((y1 + 400)) 400
    pause 2
}

# What the app logged since the last mark.
applog() { "$adb" logcat -d -s RemedyUI:I | grep -v "^---" ; }
# Android's records of the app's notifications: tag, flags, channel, actions.
notifications() {
    "$adb" shell dumpsys notification --noredact | tr -d '\r' |
        awk -v p="$pkg" '/NotificationRecord\(/ { show = index($0, "pkg=" p) > 0 } show' |
        grep -E "NotificationRecord|channel=|actions=|flags=|largeIcon|android.title=|android.text=|  [0-9]: " || true
}
services() {
    "$adb" shell dumpsys activity services "$pkg" | tr -d '\r' |
        grep -E "ServiceRecord|isForeground|foregroundId|foregroundServiceType|types=" || echo "(no services running)"
}
top() { "$adb" shell dumpsys activity activities | tr -d '\r' | grep -m1 -E "topResumedActivity|mResumedActivity"; }
shade() { "$adb" shell cmd statusbar expand-notifications; pause 2; }
close_shade() { "$adb" shell cmd statusbar collapse; pause 1; }

# From a known state: the shade closed, no dialog up, the app stopped (which also takes its
# notifications away).
"$adb" shell cmd statusbar collapse
"$adb" shell input keyevent KEYCODE_HOME
"$adb" shell am force-stop "$pkg"
"$adb" shell pm revoke "$pkg" android.permission.POST_NOTIFICATIONS 2>/dev/null
"$adb" shell pm revoke "$pkg" android.permission.RECORD_AUDIO 2>/dev/null
"$adb" logcat -c
"$adb" shell am start -n "$app" >/dev/null
pause 7
tree >/dev/null
shot n1-start

step "ask for notifications (a prompt on Android 13 and later; answered at once before)"
tap "Allow notifications"; pause 3
if [ "$sdk" -ge 33 ]; then
    shot n2-notification-prompt
    tap "Allow"; pause 2
fi
applog | grep "notifications permission"

step "notify: a notification with two buttons and a picture; again, which replaces it"
tap "Notify"; pause 2
tap "Notify"; pause 2
applog | grep "notify:"
notifications | tee "$out/dumpsys-notification-shown.txt"
echo "records under tag remedyui:7: $(grep -c 'tag=remedyui:7' "$out/dumpsys-notification-shown.txt")"
shade
shot n3-shade
tree > "$out/tree-shade.txt"

step "press the 'Reply' button in the shade"
tap "Reply"; pause 5
applog | grep "notification 7 answered"
close_shade

step "the app to the background, then press the notification: the app comes forward"
"$adb" shell input keyevent KEYCODE_HOME; pause 2
top
shade
tap "A message for you"; pause 3
top
applog | grep -E "notification 7 answered: Activated|event Notification"
shot n4-forward-after-press

step "swipe a notification away: dismissed"
tap "Notify"; pause 2
shade
swipe_away "A message for you"; pause 6
applog | grep "Dismissed"
close_shade

step "'Mark read' makes the app withdraw it"
tap "Notify"; pause 2
shade
tap "Mark read"; pause 5
close_shade
applog | grep -E "answered: Action\(\"mark-read\"\)|withdrew"
echo "records under tag remedyui:7 after withdrawing: $(notifications | grep -c 'tag=remedyui:7')"

step "start a call: the microphone prompt, then foreground work (microphone and playback)"
"$adb" logcat -c
tap "Start call"; pause 3
shot n5-microphone-prompt
tap "While using the app" || tap "Only this time" || tap "Allow"; pause 3
applog | grep -E "microphone permission|start_foreground"
services | tee "$out/dumpsys-services-call.txt"
notifications | tee "$out/dumpsys-notification-call.txt"

step "the app goes to the background for a while: the call keeps running"
"$adb" shell input keyevent KEYCODE_HOME; pause 12
echo "process: $("$adb" shell pidof "$pkg" | tr -d '\r')"
services | tee "$out/dumpsys-services-background.txt"
shade
shot n6-call-in-shade

step "hang up from the call's notification: the service ends"
expand "Call in progress"
shot n7-call-expanded
tap "Hang up"; pause 6
close_shade
applog | grep -E "answered: Action\(\"hang-up\"\)|call ended"
services | tee "$out/dumpsys-services-after-hang-up.txt"

step "start a call again and end it in the app"
"$adb" shell am start -n "$app" >/dev/null; pause 3
tap "Start call"; pause 3
services | grep -m1 isForeground
tap "End call"; pause 3
services
applog | grep -E "start_foreground|stop_foreground"

step "the app ends; pressing its notification starts it again and reports the press"
tap "Notify"; pause 2
"$adb" shell input keyevent KEYCODE_HOME; pause 2
"$adb" shell am kill "$pkg"; pause 2
echo "process after am kill: '$("$adb" shell pidof "$pkg" | tr -d '\r')'"
"$adb" logcat -c
shade
tap "A message for you"; pause 8
echo "process now: $("$adb" shell pidof "$pkg" | tr -d '\r')"
top
applog | grep -E "demo starting|Notification\(7, Activated\)|answered: Activated"

"$adb" logcat -d -s RemedyUI:I > "$out/notify-logcat.txt"
"$adb" logcat -d | grep -E "FATAL EXCEPTION|ForegroundServiceStartNotAllowed|did not then call|RemoteServiceException|ANR in $pkg" > "$out/notify-errors.txt"
echo "errors logged: $(wc -l < "$out/notify-errors.txt")"
