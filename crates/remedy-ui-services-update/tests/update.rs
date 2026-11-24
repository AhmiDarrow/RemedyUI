//! Updates against a local server: discovery, signature and hash checks, refusals.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use base64::Engine;
use remedy_ui_services_update::{Feed, Release, UpdateConfig, UpdateError, check, download, verify};

/// A tiny HTTP server answering GETs from a path → body map, on its own thread.
fn serve(routes: Vec<(String, Vec<u8>)>) -> String {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(e) => unreachable!("bind: {e}"),
    };
    let addr = listener.local_addr().map(|a| a.to_string()).unwrap_or_default();
    let routes = Arc::new(Mutex::new(routes));
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let routes = routes.clone();
            std::thread::spawn(move || {
                let mut s = stream;
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(buf.get(..n).unwrap_or_default()).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let body = routes
                    .lock()
                    .ok()
                    .and_then(|r| r.iter().find(|(p, _)| *p == path).map(|(_, b)| b.clone()));
                let (status, body) = match body {
                    Some(b) => ("200 OK", b),
                    None => ("404 Not Found", Vec::new()),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(&body);
            });
        }
    });
    format!("http://{addr}")
}

struct Keys {
    public: String,
    secret: minisign::SecretKey,
    pk: minisign::PublicKey,
}

fn keys() -> Keys {
    match minisign::KeyPair::generate_unencrypted_keypair() {
        Ok(k) => Keys { public: k.pk.to_base64(), secret: k.sk, pk: k.pk },
        Err(e) => unreachable!("keys: {e}"),
    }
}

fn sign(k: &Keys, data: &[u8]) -> String {
    match minisign::sign(Some(&k.pk), &k.secret, std::io::Cursor::new(data), None, None) {
        Ok(s) => s.to_string(),
        Err(e) => unreachable!("sign: {e}"),
    }
}

#[test]
fn a_newer_github_release_is_found_downloaded_and_verified() {
    let k = keys();
    let installer = b"pretend installer bytes".to_vec();
    // Tauri wraps the minisign signature in base64; both forms are accepted.
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sign(&k, &installer));
    let routes = vec![
        ("/dl/SecretSticky_0.2.0_x64-setup.exe".to_owned(), installer.clone()),
        ("/dl/SecretSticky_0.2.0_x64-setup.exe.sig".to_owned(), sig_b64.into_bytes()),
    ];
    let base = serve(routes.clone());
    let release_json = format!(
        r#"{{"tag_name":"v0.2.0","body":"Faster unlock","assets":[
            {{"name":"SecretSticky_0.2.0_x64-setup.exe","browser_download_url":"{base}/dl/SecretSticky_0.2.0_x64-setup.exe"}},
            {{"name":"SecretSticky_0.2.0_x64-setup.exe.sig","browser_download_url":"{base}/dl/SecretSticky_0.2.0_x64-setup.exe.sig"}}]}}"#
    );
    let mut all = routes;
    all.push(("/repos/AhmiDarrow/SecretSticky/releases/latest".to_owned(), release_json.into_bytes()));
    let api = serve(all);
    let mut cfg = UpdateConfig::new(
        "0.1.8",
        Feed::GitHub {
            owner: "AhmiDarrow".into(),
            repo: "SecretSticky".into(),
            asset: "_x64-setup.exe".into(),
        },
        k.public.clone(),
    );
    cfg.github_api = api;
    let release = check(&cfg);
    let Ok(Some(release)) = release else { unreachable!("a newer release: {release:?}") };
    assert_eq!((release.version.as_str(), release.notes.as_str()), ("v0.2.0", "Faster unlock"));
    let dir = std::env::temp_dir().join(format!("remedy-ui-update-{}", std::process::id()));
    let mut seen = 0;
    let file = download(&cfg, &release, &dir, |got, _| seen = got);
    let Ok(file) = file else { unreachable!("download: {file:?}") };
    assert_eq!(std::fs::read(&file).ok(), Some(installer));
    assert!(seen > 0, "progress was reported");
    let _ = std::fs::remove_dir_all(&dir);

    cfg.current = "0.2.0".into();
    assert_eq!(check(&cfg), Ok(None), "up to date");
}

#[test]
fn tampered_downloads_wrong_keys_and_bad_hashes_are_refused() {
    let k = keys();
    let other = keys();
    let data = b"the real update".to_vec();
    let signature = sign(&k, &data);
    let release = {
        let mut v_ = Release::new("1.0.0", "");
        v_.signature = signature.clone();
        v_
    };
    let cfg = UpdateConfig::new("0.9.0", Feed::Json(String::new()), k.public.clone());
    assert_eq!(verify(&cfg, &release, &data), Ok(()));
    assert!(
        matches!(verify(&cfg, &release, b"the real updatE"), Err(UpdateError::Verification(_))),
        "one byte changed"
    );
    let stranger = UpdateConfig::new("0.9.0", Feed::Json(String::new()), other.public.clone());
    assert!(
        matches!(verify(&stranger, &release, &data), Err(UpdateError::Verification(_))),
        "someone else's key"
    );
    let wrong_hash = {
        let mut v_ = release.clone();
        v_.sha256 = Some("00".repeat(32));
        v_
    };
    assert!(matches!(verify(&cfg, &wrong_hash, &data), Err(UpdateError::Verification(_))), "hash mismatch");
    use sha2::Digest;
    let right: String = sha2::Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        verify(
            &cfg,
            &{
                let mut v_ = release;
                v_.sha256 = Some(right.to_uppercase());
                v_
            },
            &data
        ),
        Ok(())
    );
}

#[test]
fn json_feeds_work_and_only_https_or_loopback_is_contacted() {
    let k = keys();
    let data = b"portable binary".to_vec();
    let base = serve(vec![("/app.bin".to_owned(), data.clone())]);
    let feed = format!(
        r#"{{"version":"2.0.0","url":"{base}/app.bin","signature":{}}}"#,
        serde_json::to_string(&sign(&k, &data)).unwrap_or_default()
    );
    let feed_base = serve(vec![("/feed.json".to_owned(), feed.into_bytes())]);
    let cfg = UpdateConfig::new("1.4.2", Feed::Json(format!("{feed_base}/feed.json")), k.public.clone());
    let release = check(&cfg);
    assert!(matches!(&release, Ok(Some(r)) if r.version == "2.0.0"), "{release:?}");
    let remote = UpdateConfig::new("1.0.0", Feed::Json("http://example.com/feed.json".into()), k.public);
    assert!(
        matches!(check(&remote), Err(UpdateError::Feed(e)) if e.contains("https")),
        "plain http to the internet is refused"
    );
}

#[test]
fn a_tauri_latest_json_feed_picks_this_platform_and_verifies() {
    let k = keys();
    let data = b"Remedy.Desktop 1.0 installer".to_vec();
    // Tauri wraps the minisign signature in base64, as latest.json does.
    let sig = base64::engine::general_purpose::STANDARD.encode(sign(&k, &data));
    let win_name = "Remedy.Desktop_1.0.0_x64-setup.exe";
    let linux_name = "Remedy.Desktop_1.0.0_amd64.AppImage";
    let mac_name = "Remedy.Desktop_1.0.0_aarch64.app.tar.gz";
    let base = serve(vec![
        (format!("/{win_name}"), data.clone()),
        (format!("/{linux_name}"), data.clone()),
        (format!("/{mac_name}"), data.clone()),
    ]);
    let win = format!("{base}/{win_name}");
    let linux = format!("{base}/{linux_name}");
    let mac = format!("{base}/{mac_name}");
    let feed = serde_json::json!({
        "version": "v1.0.0",
        "notes": "first RemedyUI build",
        "pub_date": "2026-11-22T21:00:00Z",
        "platforms": {
            "windows-x86_64": { "signature": sig, "url": win },
            "linux-x86_64": { "signature": sig, "url": linux },
            "darwin-aarch64": { "signature": sig, "url": mac }
        }
    });
    let feed_base = serve(vec![("/latest.json".to_owned(), feed.to_string().into_bytes())]);
    let cfg = UpdateConfig::new("0.70.5", Feed::Json(format!("{feed_base}/latest.json")), k.public.clone());
    let release = check(&cfg);
    let Ok(Some(release)) = release else { unreachable!("tauri feed: {release:?}") };
    assert_eq!(release.version, "v1.0.0");
    assert_eq!(release.notes, "first RemedyUI build");
    let expected = if cfg!(target_os = "windows") {
        win_name
    } else if cfg!(target_os = "macos") {
        mac_name
    } else {
        linux_name
    };
    assert!(release.url.ends_with(expected), "{}", release.url);
    let dir = std::env::temp_dir().join(format!("remedy-ui-tauri-feed-{}", std::process::id()));
    let file = download(&cfg, &release, &dir, |_, _| {});
    let Ok(file) = file else { unreachable!("download: {file:?}") };
    assert_eq!(std::fs::read(&file).ok(), Some(data));
    let _ = std::fs::remove_dir_all(&dir);
    let mut up_to_date = cfg;
    up_to_date.current = "1.0.0".into();
    assert_eq!(check(&up_to_date), Ok(None), "up to date with a v-prefixed feed version");
}
