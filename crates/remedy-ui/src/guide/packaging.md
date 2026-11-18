# Packaging and updates

This page covers turning an app into something people install, and keeping it up to date:
`cargo remedy package`, `remedy.toml`, code signing, packages for each platform, and signed
updates with [`update`](crate::update) (feature `update`). Apps in a browser page are covered
in [`crate::guide::web`].

## `cargo remedy package`

`cargo remedy package` builds a release and packages it. `--target` picks the platform:
`windows` (the default), `linux`, `android` or `macos`.

### Windows

1. The icon from `remedy.toml` (a square PNG, 256 px or larger) becomes a multi-size `.ico`.
   Without one, a default icon is used and the tool says so.
2. With a resource compiler (`llvm-rc`, or `rc` from the Windows SDK), the icon and the
   version information are compiled into the executable.
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

### Linux, Android and macOS

- **Linux** (`--target linux`): `<bin>-<version>-linux-x86_64.tar.gz` holding the executable,
  an XDG desktop entry and the icon, laid out under a prefix (`bin/`, `share/`), so unpacking
  it into `~/.local` installs it for the person.
- **Android** (`--target android`): a signed APK with no Gradle: the app's library built with
  the NDK's clang, a manifest for a native activity, then `aapt2`, `zipalign` and `apksigner`.
  It needs `ANDROID_HOME` with the NDK and build-tools, and reads an `[android]` section of
  `remedy.toml` (below); without a keystore it signs with a debug key.
- **macOS** (`--target macos`): `<name>.app` (its `Info.plist`, the executable and an `.icns`
  icon) in a zip. It needs Apple's linker, so it is built on a Mac or with a cross linker set
  up.

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

[windows]
# Sign with this certificate from your certificate store (its SHA-1 thumbprint).
# Without it, packages are unsigned.
sign_thumbprint = "0123456789ABCDEF0123456789ABCDEF01234567"
timestamp_url = "http://timestamp.digicert.com"

[android]
# The Rust target, the lowest and the target API level, and the release key.
target = "aarch64-linux-android"
min_api = 26
target_api = 34
keystore = "release.keystore"
key_alias = "release"
```

## Android

Apps reach Android through the Android bridge, in a native activity with no Java of the
app's own. The app's entry point is `remedy_ui::android::main!`, which compiles to nothing on
other platforms. `cargo remedy new my-app --template android` makes one app for Android and
the desktop, ready for `cargo remedy package --target android`.

## Updates

The [`update`](crate::update) pack checks for a newer release, downloads it, verifies it and
applies it. It is the one networked pack, it is opt-in, and it contacts only the feed the
app names, sending nothing beyond what an HTTPS request carries.

- **Feeds** ([`update::Feed`](crate::update::Feed)): a GitHub repository's latest release
  (`GitHub`), its newest release including prereleases (`GitHubAll`), or a small JSON
  document (`Json`) with `version`, `url`, `signature` and optionally `sha256` and `notes`.
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
