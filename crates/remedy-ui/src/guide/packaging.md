# Packaging and updates

This page covers turning an app into something people install, and keeping it up to date:
`cargo remedy package`, `remedy.toml`, code signing, packages for each platform, and signed
updates with [`update`](crate::update) (feature `update`). Apps in a browser page are covered
in [`crate::guide::web`].

## `cargo remedy package`

`cargo remedy package` builds a release and packages it. `--target` picks the platform:
`windows` (the default), `linux`, `android` or `macos`.

### Windows

1. The icon from `remedy.toml` (a square PNG, 256 px or larger, plus any sizes drawn by hand)
   becomes a multi-size `.ico`. Without one, a default icon is used and the tool says so.
2. The icon and the version information are written as a resource file and linked into the
   executable (no resource compiler is needed).
3. With a certificate thumbprint in `remedy.toml` and `signtool`, the executable and the
   installer are signed.
4. Inno Setup builds a per-user installer, `<bin>_<version>_x64-setup.exe`: a Start menu
   entry, an optional desktop shortcut, an uninstaller, and a launch at the end. No
   administrator rights are needed.
5. A portable zip holds the executable with the README and license files.
6. With `REMEDY_UPDATE_KEY` set to a minisign secret key file (and
   `REMEDY_UPDATE_KEY_PASSWORD` if the key has one), the installer gets a `.sig` beside it for
   the updater.

Windows packages are built on Windows, since its resource compiler, `signtool` and Inno
Setup are Windows tools.

### Linux, Android, macOS and the web

- **Linux** (`--target linux`): `<bin>-<version>-linux-x86_64.tar.gz` holding the executable,
  an XDG desktop entry and the icon, laid out under a prefix (`bin/`, `share/`), so unpacking
  it into `~/.local` installs it for the person; and the executable alone as
  `<bin>_<version>_linux-x86_64`, for in-app updates that replace it. With `[linux] glibc =
  "2.28"` it is built with `cargo zigbuild` against that glibc, so it runs on every
  distribution since; otherwise it needs this machine's glibc or newer.
- **Android** (`--target android`): a signed APK with no Gradle: the app's library built with
  the NDK's clang, the Android bridge's Java (its compiled `classes.dex`), the app's icon as
  the launcher icon (at every screen density, and as an adaptive icon: below), a manifest, then `aapt2`, `zipalign` and `apksigner`. The bridge's Java
  includes its `SharedFiles` provider, which the manifest declares (not exported, per-file
  grants, authority `<id>.remedy.share`): it serves the files
  [`App::share`](crate::App::share) hands to the app the person chose, since other apps
  cannot read an app's files by path, and Android makes providers only from the APK's own code.
  It needs `ANDROID_HOME` with the NDK and build-tools, and reads an `[android]` section of
  `remedy.toml` (below); without a keystore it signs with a debug key.
- **macOS** (`--target macos`): `<name>.app` (its `Info.plist`, the executable and an `.icns`
  icon) in a zip. It needs Apple's linker, so it is built on a Mac or with a cross linker set
  up.
- **Web** (`--target web`): an installable page (a PWA) in `target/remedy/web/<name>` and a
  zip of it: the app built for `wasm32-unknown-unknown` with remedy-ui's `web` feature,
  `wasm-bindgen`'s glue (the CLI of the version in `Cargo.lock`), the page, a manifest, icons
  and an offline service worker, named by a `[web]` section of `remedy.toml`
  ([`crate::guide::web`]).

The results go to `target/remedy/dist`.

The release profile `cargo remedy new` writes (`lto = "fat"`, `opt-level = "s"`,
`codegen-units = 1`, `strip = true`, `panic = "abort"`) keeps the executable small.

## `remedy.toml`

```toml
[app]
name = "Sticky Notes"
# A reverse-DNS id, unique to this app.
id = "app.remedy.sticky-notes"
publisher = "Sticky Notes authors"
description = "Sticky Notes, built with RemedyUI"
# A square PNG, 256 px or larger.
icon = "assets/icon.png"
# Optional: the icon as drawn at other sizes (pixel art, or small sizes simplified so they
# stay legible). A size that was drawn is used as it is; others come from the next larger
# drawing.
icon_sizes = ["assets/icon-16.png", "assets/icon-32.png", "assets/icon-48.png"]
# "smooth" (the default) blends pixels when scaling; "pixel" repeats them, for pixel art.
icon_scaling = "smooth"

[windows]
# Sign with this certificate from your certificate store (its SHA-1 thumbprint).
# Without it, packages are unsigned.
sign_thumbprint = "0123456789ABCDEF0123456789ABCDEF01234567"
timestamp_url = "http://timestamp.digicert.com"
# Optional: the installer offers "Start <name> when I sign in" (unchecked), which writes the
# app's value under HKCU\Software\Microsoft\Windows\CurrentVersion\Run with these arguments
# and removes it on uninstall. `autostart::set` adopts that value rather than adding a
# Startup-folder shortcut, so the app never starts twice.
autostart_args = "--background"

# Optional: take over an earlier per-user install of the same app made by another installer
# (an NSIS build, say). Setup waits for its program to close, then removes only the files
# named here from its folder (the program first), its Start menu and desktop shortcuts (a
# desktop shortcut comes back for the new install), its uninstall entry and these HKCU keys.
# Files are plain names and keys sit under Software\; the app's data is never touched.
[windows.replaces]
uninstall_key = "Notes"
files = ["notes.exe", "uninstall.exe"]
registry = ['Software\Maker\Notes']

[android]
# The Rust target, the lowest and the target API level, and the release key.
target = "aarch64-linux-android"
min_api = 26
target_api = 34
keystore = "release.keystore"
key_alias = "release"
# What the manifest declares: microphone, internet, notifications, camera, or full names
# such as "android.permission.VIBRATE". Ask for run-time ones with `App::request_permission`.
permissions = ["internet", "microphone"]
# Ongoing work that keeps running out of sight (`App::start_foreground`): microphone,
# media-playback, camera, data-sync. Declares the foreground service and its permissions.
foreground = ["microphone", "media-playback"]
# The adaptive icon: the picture in front (the icon without its backdrop; default: the icon),
# what fills behind it (a color, or two for a gradient from top to bottom; default: the icon's
# corner color when it is opaque, else white), and its one-color shape (a PNG whose alpha is
# the shape) for themed icons and the status bar's notification icon (target_api 33 or
# later). The three pictures are placed alike, so they line up.
icon_foreground = "assets/icon-foreground.png"
icon_background = ["#151b35", "#095249"]
icon_monochrome = "assets/icon-monochrome.png"
```

### Icons

Every package takes its icons from `[app] icon` and `[app] icon_sizes`: the Windows `.ico`
holds 16 to 256 px (and every drawn size), the Linux archive the `hicolor` theme's 256 px icon
and every drawn size of the theme's, the macOS `.icns` 256, 512 and 1024 px, the web page 192
and 512 px. Android gets the 48 dp launcher icon at every density (for Android 7 and older)
and an adaptive icon (8 and later): its foreground is `[android] icon_foreground` (else the icon)
filling the 72 dp middle that launchers show of the 108 dp layer (a round mask trims its
corners), its background is `[android] icon_background` (a color or a gradient), and `[android] icon_monochrome` is the themed layer Android 13
tints, which the status bar also shows for the app's notifications (without it the status
bar shows the icon's own shape).

The running app shows its icon in windows, the tray and notifications too:
[`WindowSpec::icon`](crate::WindowSpec::icon) with a
[`WindowIcon`](crate::host::WindowIcon) (`with_size` for drawn sizes, `pixel_art` for
nearest-pixel scaling), [`TrayIcon::with_artwork`](crate::host::TrayIcon::with_artwork), and
[`Notification::icon`](crate::host::Notification). An app built without
`cargo remedy package` puts its icon and version information into its Windows executable with
`remedy-ui-build` in its build script:

```toml
[build-dependencies]
remedy-ui-build = "1"
```

```text
// build.rs
fn main() {
    remedy_ui_build::WindowsResources::new()
        .icon("assets/app.ico")
        .product_name("Notes")
        .compile()
        .unwrap_or_else(|e| println!("cargo::warning={e}"));
}
```

It writes the resource file itself, so no resource compiler is needed; while
`cargo remedy package` builds, it stands aside for the package's own resources.

## Android

Apps reach Android through the Android bridge, with no Java of the app's own. The app's
entry point is `remedy_ui::android::main!`, which compiles to nothing on other platforms.
`cargo remedy new my-app --template android` makes one app for Android and the desktop,
ready for `cargo remedy package --target android`.

The APK carries the bridge's own Java (`classes.dex` from the `remedy-ui-bridge-android`
release the app builds with, found with `cargo metadata`), and its manifest declares:

- `dev.remedyui.android.RemedyActivity`, Android's `NativeActivity` loading the app's
  library (`android.app.lib_name`), the launcher entry, the configuration changes the
  engine follows in place, `adjustResize` for the keyboard and a theme without an action
  bar. It also hears a notification being pressed while the app runs (`onNewIntent`, which
  `NativeActivity` drops), so [`App::on_notification`](crate::App::on_notification) gets
  `Activated` and the app comes forward;
- `dev.remedyui.android.NotificationOpen`, a not-exported alias of the activity that the
  app's notifications open, so no other app can claim one was pressed;
- with `[android] foreground`, the `dev.remedyui.android.Foreground` service with those
  foreground service types, plus `FOREGROUND_SERVICE`, the `FOREGROUND_SERVICE_<TYPE>`
  permission of each type (Android 14 requires them), the microphone or camera permission
  for those types and `POST_NOTIFICATIONS` (the work's notification). The app still asks
  for the run-time ones with [`App::request_permission`](crate::App::request_permission).

[`App::notify`](crate::App::notify) posts to the app's two channels, "Notifications"
(alerting) and "Quiet notifications" (for [`Notification::silent`](crate::host::Notification::silent())),
which the person can tune in the system settings; the status bar shows the app's icon in one
color (Android 13's themed icon layer when the icon has one), and the notification's
picture is its large icon. Android shows the first three buttons. On Android 13 and later
ask with `Permission::Notifications` first; before 13 that answers at once whether the
app's notifications are on. Buttons and swiping a notification away reach the app while it
runs; pressing a notification works whether it runs or not (it starts the app, which hears
the press after its setup).

[`App::start_foreground`](crate::App::start_foreground) keeps the app running while it is out
of sight for a [`ForegroundTask`](crate::host::ForegroundTask): a voice call is
`ForegroundKind::Microphone` and `ForegroundKind::MediaPlayback`, shown by a notification
with a "Hang up" button. Ask for the microphone first and start the work while the app is
on screen: Android refuses microphone and camera work started from the background, and the
call answers [`Unsupported`](crate::host::Unsupported) with what is missing. If Android
ends the work later (the daily limit on data sync, Android 15),
[`App::on_foreground_stopped`](crate::App::on_foreground_stopped) says why. On the desktop and
the web apps keep running anyway, so the call answers `Ok` there and shows nothing.

An app built into an APK by other means can keep a plain `android.app.NativeActivity` with
`hasCode="false"`: the bridge then loads its Java from memory, notifications show and their
buttons and dismissals are reported, but pressing a notification only opens the app:
`NativeActivity` drops the intent that says which one, so `Activated` never arrives unless
the press starts the app afresh (no task of the app left). Foreground work answers `Unsupported`.

An Android app's data folder ([`App::data_dir`](crate::App::data_dir)) is the private
internal storage Android gives it (`/data/user/<user>/<package>/files`, the bridge's
`android::data_dir`): it is the app's own already, so no app name is added. Only the app
reads it, it needs no permission, it is kept across updates, and it goes when the app is
uninstalled or its data is cleared. Persisted signals, window geometry and the app's own
documents ([`App::persist_store`](crate::App::persist_store)) live there, in `state/` and
`windows.txt`, and are written when the app goes to the background, since Android may end a
background app without warning.

An APK built another way shares files only if its manifest declares the provider the same
way and its `classes.dex` holds it (the bridge's `dex/classes.dex`, rebuilt by its
`tools/build-dex.sh`, does); a plain `NativeActivity` APK with no code of its own has no
provider, so [`App::share`](crate::App::share) there shares the title and text alone, or
answers unsupported when the app requires the files.

## Updates

The [`update`](crate::update) pack checks for a newer release, downloads it, verifies it and
applies it. It is the one networked pack, it is opt-in, and it contacts only the feed the
app names, sending nothing beyond what an HTTPS request carries.

- **Feeds** ([`update::Feed`](crate::update::Feed)): a GitHub repository's latest release
  (`GitHub`), its newest release including prereleases (`GitHubAll`), or a JSON document
  (`Json`): a flat `{version, url, signature}` feed, or Tauri's multi-platform
  `latest.json` (`platforms.windows-x86_64` and the rest). Optional `sha256` and `notes`
  are accepted in both forms. `Apply::RunInstaller` takes the installer arguments (a
  silent Windows NSIS update passes `/S` and `/UPDATE`).
- **Trust** ([`update::Trust`](crate::update::Trust)): a minisign signature under the app's
  public key (the format `minisign` and Tauri's updater produce; `cargo remedy package`
  writes it), or the SHA-256 from a checksum file published beside the release, optionally
  with the Windows Authenticode publisher. A download that does not verify is deleted and
  never run.
- **Plain `http`** is refused, except on the loopback interface for local testing.

The calls block, so run them off the UI thread with [`App::background`](crate::App::background):

```no_run
use remedy_ui::reactive::Signal;
use remedy_ui::update::{self, Feed, UpdateConfig, UpdateState};
use remedy_ui::{App, AppConfig, WindowSpec};

/// The app's minisign public key (the second line of its `.pub` file).
const PUBLIC_KEY: &str = "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";

fn feed() -> UpdateConfig {
    let feed = Feed::GitHub { owner: "example".into(), repo: "notes".into(), asset: "_x64-setup.exe".into() };
    UpdateConfig::new(env!("CARGO_PKG_VERSION"), feed, PUBLIC_KEY)
}

remedy_ui::run(AppConfig::new("Notes"), |app: &App| {
    let state = Signal::new(UpdateState::Checking);
    app.background(
        || update::check(&feed()),
        move |found| {
            state.set(match found {
                Ok(Some(release)) => UpdateState::Available(release),
                Ok(None) => UpdateState::UpToDate,
                Err(e) => UpdateState::Failed(e.to_string()),
            })
        },
    );
    app.open(WindowSpec::new("Notes"), move |ui, _| {
        let banner = update::update_banner(ui, state, || { /* download, then apply */ }, || {});
        ui.root().child(banner);
    });
})
.ok();
```

After [`update::check`](crate::update::check) finds a release,
[`update::download`](crate::update::download) fetches and verifies it (reporting progress),
and [`update::apply`](crate::update::apply) runs the installer or puts the new executable in
place of the running one ([`update::Apply`](crate::update::Apply)); the app then quits so it
can be replaced.

Verification needs no network, so it is easy to test. A release whose signature does not
match is refused:

```
use remedy_ui::update::{Feed, Release, UpdateConfig, UpdateError, verify};

let cfg = UpdateConfig::new(
    "1.0.0",
    Feed::Json("https://example.com/notes/latest.json".into()),
    "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3",
);
let mut release = Release::new("1.1.0", "https://example.com/notes/notes_1.1.0_x64-setup.exe");
release.signature = "not a signature".into();
let refused = verify(&cfg, &release, b"the downloaded installer");
assert!(matches!(refused, Err(UpdateError::Verification(_))));
```

## The update banner

[`update::update_banner`](crate::update::update_banner) is a strip that shows where an update
stands, from an [`UpdateState`](crate::update::UpdateState) signal the app sets: checking, up
to date, a new version with an Install button, a progress bar while downloading, ready with a
Restart button, or what failed. It is empty while idle.
[`update::update_banner_with`](crate::update::update_banner_with) takes the strings in the
app's language ([`update::BannerText`](crate::update::BannerText)).

```
# use remedy_ui::AppConfig;
# use remedy_ui::testing::TestApp;
use remedy_ui::reactive::Signal;
use remedy_ui::update::{Release, UpdateState, update_banner};
use remedy_ui::{App, WindowSpec};

let state = Signal::new(UpdateState::Idle);
let app = move |app: &App| {
    app.open(WindowSpec::new("Notes"), move |ui, _| {
        ui.root().child(update_banner(ui, state, || {}, || {}));
    });
};
# let mut test = TestApp::new(AppConfig::new("Notes"), app);
# let Some(ui) = test.ui() else { unreachable!("the window is open") };
# let names = |ui: &remedy_ui::Ui| ui.reader_view().into_iter().filter_map(|n| n.name).collect::<Vec<_>>();
let release = Release::new("1.1.0", "https://example.com/notes_1.1.0_x64-setup.exe");
state.set(UpdateState::Available(release));
# test.frames(1);
let shown = names(&ui);
assert!(shown.iter().any(|n| n == "Version 1.1.0 is available."), "{shown:?}");
assert!(shown.iter().any(|n| n == "Install"));
```
