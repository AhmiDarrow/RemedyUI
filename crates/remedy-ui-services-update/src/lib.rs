//! App updates: check, download, verify, apply.
//!
//! The one networked pack, opt-in and contacting only the feed the app configures. It sends
//! nothing beyond what an HTTPS request carries (no identifiers). A release is accepted only
//! if its download verifies against the app's minisign public key (the format Tauri's updater
//! and the `minisign` tool produce) and, when the feed gives one, its SHA-256. Feeds are a
//! GitHub repository's latest release or a JSON document (a flat `{version,url,signature}`
//! feed, or Tauri's multi-platform `latest.json`). Plain `http` is refused except
//! on the loopback interface (local testing). See `docs/design/08-security-and-privacy.md`.
//!
//! A feed entry that carries a SHA-256 and no signature is refused, on purpose: the hash
//! arrives over the same channel as the address it vouches for, so whoever can change the
//! feed changes both, and it proves the download is whole, not that it is the publisher's.
//! The one checksum channel that is allowed is [`Trust::Checksums`]: the checksum file
//! published beside the asset in the app's own GitHub release, and on Windows the
//! installer's Authenticode signer pinned by thumbprint. An app that wants hashes alone on
//! its own server signs its feed's downloads with minisign instead.
//!
//! The calls block; run them off the UI thread (`App::background`). A download is held in
//! memory until it verifies, so one that is cancelled ([`download_with`]) or fails leaves
//! nothing on disk; a file that fails its publisher check is deleted.

/// The update strip's own words in every language the engine ships, from `i18n/*.ftl`. Apps
/// reword them, or add a language, with [`remedy_ui_view::strings::add_messages`] under the
/// name `services-update`; [`BannerText`] replaces them outright.
pub static STRINGS: remedy_ui_view::strings::Strings = remedy_ui_view::engine_strings!("services-update");

use std::path::{Path, PathBuf};

use base64::Engine;
use sha2::Digest;

#[allow(unsafe_code)]
mod os;

pub use os::SignatureStatus;

/// A file's Authenticode signature status and signer thumbprint (Windows; elsewhere nothing
/// is signed this way).
pub fn authenticode_of(file: &Path) -> (SignatureStatus, Option<String>) {
    os::authenticode(file)
}

/// Where releases are published.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Feed {
    /// A GitHub repository's latest release. `asset` names the file to download (exactly, or
    /// as its ending, such as `_x64-setup.exe`); its signature is the asset with `.sig`
    /// appended.
    GitHub {
        /// Owner.
        owner: String,
        /// Repository.
        repo: String,
        /// The asset name or its ending.
        asset: String,
    },
    /// A JSON document: a flat `{"version", "url", "signature", "sha256"?, "notes"?, "page"?}`, or
    /// Tauri's `latest.json` (`version`, optional `notes` / `pub_date`, and
    /// `platforms.<key>.{url, signature, sha256?}`). `signature` is the minisign signature
    /// (text, or base64 of it). A Tauri document uses the entry for this OS and architecture
    /// (`windows-x86_64`, `linux-x86_64`, `darwin-aarch64`, and the usual aliases).
    Json(String),
    /// A GitHub repository's newest release including prereleases (drafts skipped); `asset`
    /// as for [`Feed::GitHub`].
    GitHubAll {
        /// Owner.
        owner: String,
        /// Repository.
        repo: String,
        /// The asset name or its ending.
        asset: String,
    },
}

/// Why a download is trusted.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Trust {
    /// A minisign signature (`<asset>.sig`, or the feed's `signature`) under this public key
    /// (the base64 line of a `.pub` file).
    Minisign(String),
    /// The SHA-256 listed for the asset in a checksum file published beside it (GNU style:
    /// `<hex>  <name>`), and on Windows, when `publisher` is given, an Authenticode signature
    /// by the certificate with that SHA-1 thumbprint. A self-signed publisher is fine; a file
    /// changed after signing never is.
    Checksums {
        /// The checksum file's asset name (`SHA256SUMS.txt`).
        file: String,
        /// The signing certificate's SHA-1 thumbprint (hex).
        publisher: Option<String>,
    },
}

/// What an app's updater needs to know.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct UpdateConfig {
    /// The running version (semantic versioning; a leading `v` is fine).
    pub current: String,
    /// Where to look.
    pub feed: Feed,
    /// Why a download is trusted.
    pub trust: Trust,
    /// GitHub's API base (tests point it at a local server).
    pub github_api: String,
}

impl UpdateConfig {
    /// An updater for version `current` on `feed`, trusting minisign signatures by
    /// `public_key`.
    pub fn new(current: impl Into<String>, feed: Feed, public_key: impl Into<String>) -> Self {
        Self::with_trust(current, feed, Trust::Minisign(public_key.into()))
    }

    /// An updater for version `current` on `feed`, trusting downloads as `trust` says.
    pub fn with_trust(current: impl Into<String>, feed: Feed, trust: Trust) -> Self {
        UpdateConfig { current: current.into(), feed, trust, github_api: "https://api.github.com".into() }
    }
}

/// A newer release.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Release {
    /// Its version.
    pub version: String,
    /// Release notes.
    pub notes: String,
    /// The download.
    pub url: String,
    /// Its minisign signature (text; empty when trusted by checksums).
    pub signature: String,
    /// Its SHA-256, hex, when the feed gives one.
    pub sha256: Option<String>,
    /// The release's own page (its notes on GitHub, the feed's `page`), for a "what's new"
    /// link, when the feed gives one.
    pub page: Option<String>,
}

impl Release {
    /// Release `version`, downloaded from `url`, with no notes, signature or checksum yet.
    pub fn new(version: impl Into<String>, url: impl Into<String>) -> Self {
        Release {
            version: version.into(),
            notes: String::new(),
            url: url.into(),
            signature: String::new(),
            sha256: None,
            page: None,
        }
    }
}

/// Stops a [`download_with`] from another thread (a Cancel button).
#[derive(Clone, Debug, Default)]
pub struct CancelDownload(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl CancelDownload {
    /// A handle nothing has cancelled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks the download to stop: it ends with [`UpdateError::Cancelled`] within one block
    /// of the transfer.
    pub fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Whether [`CancelDownload::cancel`] was called.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Why an update step failed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum UpdateError {
    /// The network or the server failed.
    Network(String),
    /// The feed did not say what it should.
    Feed(String),
    /// The download did not verify: never installed.
    Verification(String),
    /// Writing or starting the update failed.
    Io(String),
    /// The download was cancelled ([`CancelDownload`]); nothing was written.
    Cancelled,
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdateError::Network(e) => write!(f, "could not reach the update server: {e}"),
            UpdateError::Feed(e) => write!(f, "the update feed is not readable: {e}"),
            UpdateError::Verification(e) => write!(f, "the update did not verify and was not installed: {e}"),
            UpdateError::Io(e) => write!(f, "the update could not be applied: {e}"),
            UpdateError::Cancelled => write!(f, "the download was cancelled"),
        }
    }
}

impl std::error::Error for UpdateError {}

/// Only HTTPS, except the loopback interface.
fn allowed(url: &str) -> Result<(), UpdateError> {
    let local = ["http://127.0.0.1", "http://localhost", "http://[::1]"].iter().any(|p| url.starts_with(p));
    if url.starts_with("https://") || local {
        Ok(())
    } else {
        Err(UpdateError::Feed(format!("not an https address: {url}")))
    }
}

fn get_text(url: &str) -> Result<String, UpdateError> {
    allowed(url)?;
    http::get_text(url)
}

/// HTTPS for the updater. A browser page has none of its own: a page is updated by its
/// service worker, which installs the new build when the page next loads.
#[cfg(not(target_arch = "wasm32"))]
mod http {
    use std::io::Read;
    use std::time::Duration;

    use super::UpdateError;

    fn net(e: impl std::fmt::Display) -> UpdateError {
        UpdateError::Network(e.to_string())
    }

    fn agent() -> ureq::Agent {
        // ring is the TLS provider; installing it twice is harmless.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
        ureq::Agent::config_builder()
            .tls_config(tls)
            .timeout_global(Some(Duration::from_secs(60)))
            .https_only(false)
            .build()
            .into()
    }

    /// Feeds, signatures and checksum files: small text, through the shared HTTP client.
    pub(super) fn get_text(url: &str) -> Result<String, UpdateError> {
        let req = remedy_ui_services_net::Request::get(url)
            .header("User-Agent", "RemedyUI-updater")
            .header("Accept", "application/vnd.github+json, application/json, */*");
        let res = remedy_ui_services_net::Http::new("")
            .timeout(Duration::from_secs(60))
            .send_blocking(req, &remedy_ui_reactive::Cancel::new())
            .map_err(net)?;
        String::from_utf8(res.body).map_err(net)
    }

    /// The body of `url`, reporting bytes so far and the total when the server says it;
    /// `progress` answering `false` stops it with [`UpdateError::Cancelled`].
    pub(super) fn fetch(
        url: &str,
        mut progress: impl FnMut(u64, Option<u64>) -> bool,
    ) -> Result<Vec<u8>, UpdateError> {
        let res = agent().get(url).header("User-Agent", "RemedyUI-updater").call().map_err(net)?;
        let total =
            res.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok());
        let mut reader = res.into_body().into_reader();
        let mut bytes = Vec::with_capacity(total.and_then(|t: u64| usize::try_from(t).ok()).unwrap_or(0));
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = reader.read(&mut buf).map_err(net)?;
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(buf.get(..n).unwrap_or_default());
            if !progress(bytes.len() as u64, total) {
                return Err(UpdateError::Cancelled);
            }
        }
        Ok(bytes)
    }
}

#[cfg(target_arch = "wasm32")]
mod http {
    use super::UpdateError;

    fn none() -> UpdateError {
        UpdateError::Network(
            "a browser page has no updater of its own: its service worker installs a new build when the page next loads"
                .into(),
        )
    }

    pub(super) fn get_text(_url: &str) -> Result<String, UpdateError> {
        Err(none())
    }

    pub(super) fn fetch(
        _url: &str,
        _progress: impl FnMut(u64, Option<u64>) -> bool,
    ) -> Result<Vec<u8>, UpdateError> {
        Err(none())
    }
}

fn version(v: &str) -> Result<semver::Version, UpdateError> {
    semver::Version::parse(v.trim().trim_start_matches('v'))
        .map_err(|e| UpdateError::Feed(format!("version {v}: {e}")))
}

/// Keys Tauri writes under `platforms` in `latest.json`, preferred first, including aliases.
fn tauri_platform_keys_for(os: &str, arch: &str) -> &'static [&'static str] {
    match (os, arch) {
        ("windows", "x86_64") => &["windows-x86_64", "windows-x86-64"],
        ("windows", "aarch64") => &["windows-aarch64", "windows-arm64"],
        ("linux", "x86_64") => &["linux-x86_64", "linux-x86-64"],
        ("linux", "aarch64") => &["linux-aarch64", "linux-arm64"],
        ("macos", "aarch64") => &["darwin-aarch64", "darwin-arm64"],
        ("macos", "x86_64") => &["darwin-x86_64", "darwin-x86-64"],
        _ => &[],
    }
}

fn json_str(v: &serde_json::Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(str::to_owned)
}

/// A release from a flat JSON feed or a Tauri `latest.json` document, for this host.
fn json_release(v: &serde_json::Value) -> Result<Release, UpdateError> {
    json_release_on(v, std::env::consts::OS, std::env::consts::ARCH)
}

fn json_release_on(v: &serde_json::Value, os: &str, arch: &str) -> Result<Release, UpdateError> {
    let version = json_str(v, "version").ok_or_else(|| UpdateError::Feed("no version".into()))?;
    let notes = json_str(v, "notes").unwrap_or_default();
    if let Some(platforms) = v.get("platforms").and_then(|p| p.as_object()) {
        let keys = tauri_platform_keys_for(os, arch);
        if keys.is_empty() {
            return Err(UpdateError::Feed(format!("no Tauri platform key for {os}-{arch}")));
        }
        for key in keys {
            let Some(entry) = platforms.get(*key) else { continue };
            let url = entry.get("url").and_then(|x| x.as_str()).unwrap_or("").trim();
            let signature = entry.get("signature").and_then(|x| x.as_str()).unwrap_or("").trim();
            if url.is_empty() {
                return Err(UpdateError::Feed(format!("no url for {key}")));
            }
            if signature.is_empty() {
                return Err(UpdateError::Feed(format!("no signature for {key}")));
            }
            let sha256 = entry.get("sha256").and_then(|x| x.as_str()).map(str::to_owned);
            return Ok(Release {
                version,
                notes,
                url: url.to_owned(),
                signature: signature.to_owned(),
                sha256,
                page: entry.get("page").and_then(|x| x.as_str()).map(str::to_owned).or_else(|| json_str(v, "page")),
            });
        }
        return Err(UpdateError::Feed(format!("no platform in the feed for {os}-{arch}")));
    }
    Ok(Release {
        version,
        notes,
        url: json_str(v, "url").ok_or_else(|| UpdateError::Feed("no url".into()))?,
        signature: json_str(v, "signature").ok_or_else(|| UpdateError::Feed("no signature".into()))?,
        sha256: json_str(v, "sha256"),
        page: json_str(v, "page"),
    })
}

/// Looks for a release newer than the running version. `Ok(None)`: up to date.
pub fn check(cfg: &UpdateConfig) -> Result<Option<Release>, UpdateError> {
    let current = version(&cfg.current)?;
    let release = match &cfg.feed {
        Feed::Json(url) => {
            let v: serde_json::Value =
                serde_json::from_str(&get_text(url)?).map_err(|e| UpdateError::Feed(e.to_string()))?;
            json_release(&v)?
        }
        Feed::GitHub { owner, repo, asset } => {
            let url =
                format!("{}/repos/{owner}/{repo}/releases/latest", cfg.github_api.trim_end_matches('/'));
            let v: serde_json::Value =
                serde_json::from_str(&get_text(&url)?).map_err(|e| UpdateError::Feed(e.to_string()))?;
            github_release(cfg, &v, asset)?
        }
        Feed::GitHubAll { owner, repo, asset } => {
            let url = format!("{}/repos/{owner}/{repo}/releases", cfg.github_api.trim_end_matches('/'));
            let v: serde_json::Value =
                serde_json::from_str(&get_text(&url)?).map_err(|e| UpdateError::Feed(e.to_string()))?;
            // The newest non-draft release that has the asset.
            let mut best: Option<(semver::Version, &serde_json::Value)> = None;
            for r in v.as_array().into_iter().flatten() {
                if r.get("draft").and_then(serde_json::Value::as_bool).unwrap_or(false) {
                    continue;
                }
                let Some(tag) = r.get("tag_name").and_then(|t| t.as_str()) else { continue };
                let Ok(ver) = version(tag) else { continue };
                let has = r.get("assets").and_then(|a| a.as_array()).into_iter().flatten().any(|a| {
                    a.get("name")
                        .and_then(|n| n.as_str())
                        .is_some_and(|n| n == asset || (n.ends_with(asset.as_str()) && !n.ends_with(".sig")))
                });
                if has && best.as_ref().is_none_or(|(b, _)| ver > *b) {
                    best = Some((ver, r));
                }
            }
            match best {
                Some((_, r)) => github_release(cfg, r, asset)?,
                None => return Ok(None),
            }
        }
    };
    Ok((version(&release.version)? > current).then_some(release))
}

/// A release from a GitHub API release object, with what its trust needs: the minisign
/// signature, or the asset's SHA-256 from the published checksum file.
fn github_release(cfg: &UpdateConfig, v: &serde_json::Value, asset: &str) -> Result<Release, UpdateError> {
    let tag = v.get("tag_name").and_then(|t| t.as_str()).ok_or_else(|| UpdateError::Feed("no tag".into()))?;
    let assets = v.get("assets").and_then(|a| a.as_array()).cloned().unwrap_or_default();
    let find = |pred: &dyn Fn(&str) -> bool| {
        assets.iter().find_map(|a| {
            let name = a.get("name")?.as_str()?;
            pred(name).then(|| a.get("browser_download_url")?.as_str().map(str::to_owned)).flatten()
        })
    };
    let matches = |n: &str| n == asset || (n.ends_with(asset) && !n.ends_with(".sig"));
    let url = find(&matches).ok_or_else(|| UpdateError::Feed(format!("no asset matching {asset}")))?;
    let file_name = url.rsplit('/').next().unwrap_or_default().to_owned();
    let (signature, sha256) = match &cfg.trust {
        Trust::Minisign(_) => {
            let sig_name = format!("{file_name}.sig");
            let sig_url = find(&|n: &str| n == sig_name)
                .ok_or_else(|| UpdateError::Feed(format!("no signature {sig_name}")))?;
            (get_text(&sig_url)?, None)
        }
        Trust::Checksums { file, .. } => {
            let sums_url = find(&|n: &str| n == file).ok_or_else(|| {
                UpdateError::Feed(format!("the release is missing {file}; refusing to install"))
            })?;
            let sums = get_text(&sums_url)?;
            let hash = checksum_for(&sums, &file_name)
                .ok_or_else(|| UpdateError::Feed(format!("{file} does not list {file_name}")))?;
            (String::new(), Some(hash))
        }
    };
    Ok(Release {
        version: tag.to_owned(),
        notes: v.get("body").and_then(|b| b.as_str()).unwrap_or_default().to_owned(),
        url,
        signature,
        sha256: Some(sha256).flatten(),
        page: v.get("html_url").and_then(|u| u.as_str()).map(str::to_owned),
    })
}

/// The SHA-256 a GNU-style checksum file (`<64 hex>  [*]name` per line) lists for `name`.
pub fn checksum_for(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let mut it = line.split_whitespace();
        let hash = it.next()?;
        let file = it.next()?.trim_start_matches('*');
        (file == name && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
    })
}

/// A signature as minisign text: as given, or decoded from Tauri's base64 wrapping.
fn signature_text(sig: &str) -> String {
    let t = sig.trim();
    if t.starts_with("untrusted comment:") {
        return t.to_owned();
    }
    base64::engine::general_purpose::STANDARD
        .decode(t)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_else(|| t.to_owned())
}

/// Checks `bytes` against the release's signature and hash (the Authenticode publisher, for
/// [`Trust::Checksums`], is checked on the saved file by [`download`]).
pub fn verify(cfg: &UpdateConfig, release: &Release, bytes: &[u8]) -> Result<(), UpdateError> {
    let bad = |e: String| UpdateError::Verification(e);
    match &cfg.trust {
        Trust::Minisign(public_key) => {
            let key = minisign_verify::PublicKey::from_base64(public_key.trim())
                .map_err(|e| bad(format!("public key: {e}")))?;
            let sig = minisign_verify::Signature::decode(&signature_text(&release.signature))
                .map_err(|e| bad(format!("signature: {e}")))?;
            key.verify(bytes, &sig, false).map_err(|e| bad(format!("signature does not match: {e}")))?;
        }
        Trust::Checksums { file, .. } if release.sha256.is_none() => {
            return Err(bad(format!("no checksum from {file}")));
        }
        Trust::Checksums { .. } => {}
    }
    if let Some(want) = &release.sha256 {
        let got: String = sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
        if !got.eq_ignore_ascii_case(want.trim()) {
            return Err(bad(format!("SHA-256 {got} is not the published {want}")));
        }
    }
    Ok(())
}

/// Checks that `file` carries an Authenticode signature by the certificate with `thumbprint`
/// (SHA-1, hex). A self-signed (untrusted) publisher is accepted; a missing signature, another
/// signer, or a file changed after signing is not.
pub fn verify_publisher(file: &Path, thumbprint: &str) -> Result<(), UpdateError> {
    let bad = |e: &str| UpdateError::Verification(e.to_owned());
    let (status, signer) = os::authenticode(file);
    match status {
        SignatureStatus::HashMismatch => return Err(bad("the installer was changed after it was signed")),
        SignatureStatus::NotSigned if signer.is_none() => return Err(bad("the installer is not signed")),
        _ => {}
    }
    let want: String = thumbprint.chars().filter(char::is_ascii_hexdigit).collect::<String>().to_uppercase();
    match signer {
        Some(got) if got == want => Ok(()),
        Some(_) => Err(bad("the installer is not signed by the expected publisher")),
        None => Err(bad("the installer's signature could not be read")),
    }
}

/// Downloads the release into `dir`, reporting `(received, total)` bytes, and verifies it.
/// Returns the verified file; a file that does not verify is deleted.
pub fn download(
    cfg: &UpdateConfig,
    release: &Release,
    dir: &Path,
    progress: impl FnMut(u64, Option<u64>),
) -> Result<PathBuf, UpdateError> {
    download_with(cfg, release, dir, &CancelDownload::new(), progress)
}

/// [`download`], stopped by `cancel` from another thread: it then ends with
/// [`UpdateError::Cancelled`], having written nothing.
pub fn download_with(
    cfg: &UpdateConfig,
    release: &Release,
    dir: &Path,
    cancel: &CancelDownload,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<PathBuf, UpdateError> {
    allowed(&release.url)?;
    if cancel.is_cancelled() {
        return Err(UpdateError::Cancelled);
    }
    let bytes = http::fetch(&release.url, |got, total| {
        progress(got, total);
        !cancel.is_cancelled()
    })?;
    verify(cfg, release, &bytes)?;
    let name = release.url.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("update.bin");
    let name: String =
        name.chars().map(|c| if c.is_ascii_alphanumeric() || ".-_".contains(c) { c } else { '_' }).collect();
    std::fs::create_dir_all(dir).map_err(|e| UpdateError::Io(e.to_string()))?;
    let path = dir.join(name);
    // Written and closed before anything checks or starts it.
    std::fs::write(&path, &bytes).map_err(|e| UpdateError::Io(e.to_string()))?;
    if let Trust::Checksums { publisher: Some(thumbprint), .. } = &cfg.trust
        && let Err(e) = verify_publisher(&path, thumbprint)
    {
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    Ok(path)
}

/// How to apply a verified download.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Apply {
    /// Start the installer with these arguments; the app then quits so it can be replaced.
    /// A silent Windows NSIS update passes `/S` and `/UPDATE` here. Stop sidecars (a local
    /// runtime and the like) before calling [`apply`].
    RunInstaller(Vec<String>),
    /// Put the download in place of the running executable and start it; the app then quits.
    ReplaceExecutable,
}

/// Applies a verified update. On success the app should quit at once.
pub fn apply(file: &Path, how: &Apply) -> Result<(), UpdateError> {
    let io = |e: std::io::Error| UpdateError::Io(e.to_string());
    match how {
        Apply::RunInstaller(args) => {
            std::process::Command::new(file).args(args).spawn().map_err(io)?;
        }
        Apply::ReplaceExecutable => {
            let exe = std::env::current_exe().map_err(io)?;
            // A running executable can be renamed (not overwritten) on every platform.
            let old = exe.with_extension("old");
            let _ = std::fs::remove_file(&old);
            std::fs::rename(&exe, &old).map_err(io)?;
            if let Err(e) = std::fs::copy(file, &exe) {
                let _ = std::fs::rename(&old, &exe);
                return Err(io(e));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755));
            }
            restart_after_exit(&exe).map_err(io)?;
        }
    }
    Ok(())
}

/// Starts `exe` with this process's arguments once this process has exited, so a
/// single-instance app's new copy does not find the old one still running and hand over to
/// it. Unix waits in a shell; Windows starts it at once (its updates run an installer, which
/// waits for the app itself).
fn restart_after_exit(exe: &Path) -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(unix)]
    {
        wait_then_exec(std::process::id(), exe, &args).spawn().map(|_| ())
    }
    #[cfg(not(unix))]
    {
        std::process::Command::new(exe).args(&args).spawn().map(|_| ())
    }
}

/// A shell that waits for process `pid` to end, then becomes `exe args`.
#[cfg(unix)]
fn wait_then_exec(pid: u32, exe: &Path, args: &[String]) -> std::process::Command {
    // `$1` is the pid to wait for, `$2` the program, the rest its arguments.
    let script = "while kill -0 \"$1\" 2>/dev/null; do sleep 0.1; done; shift; exec \"$@\"";
    let mut c = std::process::Command::new("/bin/sh");
    c.arg("-c").arg(script).arg("sh").arg(pid.to_string()).arg(exe).args(args);
    c
}

/// Where an update stands, for the banner.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum UpdateState {
    /// Nothing to show.
    #[default]
    Idle,
    /// Looking for a release.
    Checking,
    /// Up to date.
    UpToDate,
    /// A newer release is available.
    Available(Release),
    /// Downloading: the fraction received, when the size is known.
    Downloading(Option<f32>),
    /// Downloaded and verified; installing restarts the app.
    Ready,
    /// Something failed (the message says what, for the user).
    Failed(String),
}

/// What the update strip says, for apps in other languages. `available` and `install` may hold `{version}`;
/// `restart` is `None` for apps whose installer restarts them (no Restart button);
/// `failure` turns an error (English, from the updater) into what to show.
#[derive(Clone)]
#[non_exhaustive]
pub struct BannerText {
    /// While looking for a release.
    pub checking: String,
    /// No newer release.
    pub up_to_date: String,
    /// A newer release, with `{version}`.
    pub available: String,
    /// While downloading.
    pub downloading: String,
    /// Downloaded and verified.
    pub ready: String,
    /// The Install button.
    pub install: String,
    /// The Restart button, or none.
    pub restart: Option<String>,
    /// The progress bar's accessible name.
    pub progress: String,
    /// What a failure shows.
    pub failure: std::rc::Rc<dyn Fn(&str) -> String>,
}

impl Default for BannerText {
    /// The English words.
    fn default() -> Self {
        BannerText::localized("en")
    }
}

impl BannerText {
    /// The words in `locale` (a BCP 47 tag; the closest language the engine ships answers,
    /// else English), from [`STRINGS`] with any rewording an app added under
    /// `services-update`. [`update_banner`] shows these for the window's locale. A failure
    /// still shows the error's own text.
    pub fn localized(locale: &str) -> Self {
        let say = |id: &str| STRINGS.format_in(locale, id, &[]);
        BannerText {
            checking: say("banner-checking"),
            up_to_date: say("banner-up-to-date"),
            // The banner fills `{version}` itself.
            available: STRINGS.format_in(locale, "banner-available", &[("version", "{version}".into())]),
            downloading: say("banner-downloading"),
            ready: say("banner-ready"),
            install: say("banner-install"),
            restart: Some(say("banner-restart")),
            progress: say("banner-progress"),
            failure: std::rc::Rc::new(str::to_owned),
        }
    }
}

impl std::fmt::Debug for BannerText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BannerText").field("install", &self.install).field("restart", &self.restart).finish()
    }
}

/// A strip that shows an update's state, with Install (when one is available) and Restart
/// (when it is ready) buttons. It is empty while idle.
#[track_caller]
pub fn update_banner(
    ui: &remedy_ui_view::Ui,
    state: remedy_ui_reactive::Signal<UpdateState>,
    on_install: impl Fn() + 'static,
    on_restart: impl Fn() + 'static,
) -> remedy_ui_view::El {
    update_banner_with(ui, state, &BannerText::localized(&ui.locale()), on_install, on_restart)
}

/// [`update_banner`] in the app's words ([`BannerText`]).
#[track_caller]
pub fn update_banner_with(
    ui: &remedy_ui_view::Ui,
    state: remedy_ui_reactive::Signal<UpdateState>,
    texts: &BannerText,
    on_install: impl Fn() + 'static,
    on_restart: impl Fn() + 'static,
) -> remedy_ui_view::El {
    use remedy_ui_widgets_core as w;
    let gap = ui.theme().spacing.unit * 2.0;
    let text = w::text(ui, "");
    let pct = remedy_ui_reactive::Signal::new(None::<f32>);
    let bar = w::progress(ui, texts.progress.clone(), pct);
    // `install` may hold `{version}` too ("Install 1.2.0 & restart"); it shows only while a
    // release is available.
    let install_text = texts.install.clone();
    let install_label = move || match state.get() {
        UpdateState::Available(r) => install_text.replace("{version}", r.version.trim_start_matches('v')),
        _ => install_text.clone(),
    };
    let install = w::button(ui, install_label, on_install).variant("primary");
    let restart = texts.restart.clone().map(|r| w::button(ui, r, on_restart).variant("primary"));
    let mut row = w::row(ui)
        .gap(gap)
        .align(remedy_ui_view::Align::Center)
        .child(text.clone())
        .child(w::spacer(ui))
        .child(install.clone());
    if let Some(r) = &restart {
        row = row.child(r.clone());
    }
    let col = w::column(ui).gap(gap).child(row).child(bar.clone());
    let c2 = col.clone();
    let texts = texts.clone();
    col.bind(move || {
        let s = state.get();
        let (msg, show_install, show_restart, progress) = match &s {
            UpdateState::Idle => (String::new(), false, false, None),
            UpdateState::Checking => (texts.checking.clone(), false, false, None),
            UpdateState::UpToDate => (texts.up_to_date.clone(), false, false, None),
            UpdateState::Available(r) => {
                (texts.available.replace("{version}", r.version.trim_start_matches('v')), true, false, None)
            }
            UpdateState::Downloading(f) => (texts.downloading.clone(), false, false, Some(*f)),
            UpdateState::Ready => (texts.ready.clone(), false, true, None),
            UpdateState::Failed(e) => ((texts.failure)(e), false, false, None),
        };
        let _ = text.clone().text(remedy_ui_view::RichText::plain(msg));
        let _ = install.clone().hidden(!show_install);
        if let Some(r) = &restart {
            let _ = r.clone().hidden(!show_restart);
        }
        let _ = bar.clone().hidden(progress.is_none());
        pct.set(progress.flatten());
        let _ = c2.clone().hidden(s == UpdateState::Idle);
    })
}

#[cfg(test)]
mod json_feed_tests {
    use super::*;

    fn tauri_doc() -> serde_json::Value {
        serde_json::json!({
            "version": "v1.0.0",
            "notes": "first RemedyUI build",
            "pub_date": "2026-11-22T21:00:00Z",
            "platforms": {
                "windows-x86_64": {
                    "signature": "win-sig",
                    "url": "https://example.com/Remedy.Desktop_1.0.0_x64-setup.exe"
                },
                "linux-x86-64": {
                    "signature": "linux-alias-sig",
                    "url": "https://example.com/Remedy.Desktop_1.0.0_amd64.AppImage"
                },
                "darwin-aarch64": {
                    "signature": "mac-sig",
                    "url": "https://example.com/Remedy.Desktop_1.0.0_aarch64.app.tar.gz"
                },
                "darwin-x86_64": {
                    "signature": "mac-intel-sig",
                    "url": "https://example.com/Remedy.Desktop_1.0.0_x64.app.tar.gz"
                }
            }
        })
    }

    #[test]
    fn tauri_keys_match_the_usual_os_and_arch_names() {
        assert_eq!(tauri_platform_keys_for("windows", "x86_64").first(), Some(&"windows-x86_64"));
        assert_eq!(tauri_platform_keys_for("linux", "x86_64"), &["linux-x86_64", "linux-x86-64"]);
        assert_eq!(tauri_platform_keys_for("macos", "aarch64").first(), Some(&"darwin-aarch64"));
        assert!(tauri_platform_keys_for("fuchsia", "x86_64").is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_replaced_program_starts_only_after_the_old_one_exits() -> std::io::Result<()> {
        let dir = std::env::temp_dir().join(format!("remedy-update-wait-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mark = dir.join("started");
        let mut old = std::process::Command::new("sleep").arg("0.6").spawn()?;
        let started = std::time::Instant::now();
        let mut new =
            super::wait_then_exec(old.id(), std::path::Path::new("touch"), &[mark.display().to_string()])
                .spawn()?;
        std::thread::sleep(std::time::Duration::from_millis(250));
        assert!(!mark.exists(), "not while the old one runs");
        let _ = old.wait();
        let _ = new.wait();
        assert!(
            mark.exists() && started.elapsed() >= std::time::Duration::from_millis(600),
            "once it has gone"
        );
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn a_tauri_latest_json_picks_this_platform_and_its_alias() -> Result<(), UpdateError> {
        let v = tauri_doc();
        let win = json_release_on(&v, "windows", "x86_64")?;
        assert_eq!(win.version, "v1.0.0");
        assert_eq!(win.notes, "first RemedyUI build");
        assert!(win.url.ends_with("_x64-setup.exe"));
        assert_eq!(win.signature, "win-sig");

        // linux-x86_64 is absent; the linux-x86-64 alias still matches.
        let linux = json_release_on(&v, "linux", "x86_64")?;
        assert!(linux.url.ends_with(".AppImage"));
        assert_eq!(linux.signature, "linux-alias-sig");

        let mac = json_release_on(&v, "macos", "aarch64")?;
        assert!(mac.url.contains("aarch64"));
        assert_eq!(mac.signature, "mac-sig");
        Ok(())
    }

    #[test]
    fn a_flat_json_feed_is_unchanged() -> Result<(), UpdateError> {
        let v = serde_json::json!({
            "version": "2.0.0",
            "url": "https://example.com/app.bin",
            "signature": "sig",
            "sha256": "ab",
            "notes": "flat"
        });
        let r = json_release_on(&v, "linux", "x86_64")?;
        assert_eq!(
            (r.version.as_str(), r.url.as_str(), r.signature.as_str(), r.sha256.as_deref(), r.notes.as_str()),
            ("2.0.0", "https://example.com/app.bin", "sig", Some("ab"), "flat")
        );
        Ok(())
    }

    #[test]
    fn a_tauri_feed_without_this_platform_or_a_signature_is_refused() {
        let v = tauri_doc();
        let err = json_release_on(&v, "linux", "aarch64").map(|r| r.url);
        assert!(matches!(&err, Err(UpdateError::Feed(e)) if e.contains("linux-aarch64")), "{err:?}");

        let empty_sig = serde_json::json!({
            "version": "1.0.0",
            "platforms": { "windows-x86_64": { "url": "https://example.com/app.exe", "signature": "" } }
        });
        let err = json_release_on(&empty_sig, "windows", "x86_64").map(|r| r.url);
        assert!(matches!(&err, Err(UpdateError::Feed(e)) if e.contains("signature")), "{err:?}");
    }

    #[test]
    fn a_cancelled_download_writes_nothing_and_a_feed_gives_the_release_page() {
        let cancel = CancelDownload::new();
        cancel.cancel();
        assert!(cancel.is_cancelled());
        let cfg = UpdateConfig::new("1.0.0", Feed::Json("https://example.invalid/latest.json".into()), "");
        let release = Release::new("1.1.0", "https://example.invalid/app-1.1.0.exe");
        let dir = std::env::temp_dir().join(format!("remedy-update-cancel-{}", std::process::id()));
        let got = download_with(&cfg, &release, &dir, &cancel, |_, _| {});
        assert_eq!(got, Err(UpdateError::Cancelled));
        assert!(!dir.exists(), "nothing was written");
        assert_eq!(release.page, None);
        let feed = serde_json::json!({
            "version": "1.1.0", "url": "https://example.invalid/a.exe", "signature": "s",
            "page": "https://example.invalid/releases/1.1.0"
        });
        let page = json_release(&feed).ok().and_then(|r| r.page);
        assert_eq!(page.as_deref(), Some("https://example.invalid/releases/1.1.0"));
    }

}
