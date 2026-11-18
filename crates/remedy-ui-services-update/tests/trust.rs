//! Releases trusted by a published checksum file and an Authenticode publisher (as Hearth's
//! are), including prereleases.

use std::io::{Read, Write};
use std::net::TcpListener;

use remedy_ui_services_update::{
    Feed, Trust, UpdateConfig, UpdateError, check, checksum_for, download, verify,
};

/// A tiny HTTP server: routes are `(path, body)`; `{base}` in a body becomes its own address.
fn serve(routes: Vec<(String, Vec<u8>)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| unreachable!("bind: {e}"));
    let base = format!("http://{}", listener.local_addr().map(|a| a.to_string()).unwrap_or_default());
    let routes: Vec<(String, Vec<u8>)> = routes
        .into_iter()
        .map(|(p, b)| (p, String::from_utf8_lossy(&b).replace("{base}", &base).into_bytes()))
        .collect();
    std::thread::spawn(move || {
        for mut s in listener.incoming().flatten() {
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(buf.get(..n).unwrap_or_default()).to_string();
            let path = req.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let (status, body) = match routes.iter().find(|(p, _)| *p == path) {
                Some((_, b)) => ("200 OK", b.clone()),
                None => ("404 Not Found", Vec::new()),
            };
            let _ = s.write_all(
                format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len())
                    .as_bytes(),
            );
            let _ = s.write_all(&body);
        }
    });
    base
}

fn sha256_hex(b: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
fn checksum_trust_picks_the_newest_release_including_prereleases() {
    let installer = b"hearth setup bytes".to_vec();
    let sums = format!("{}  HearthSetup.exe\n{}  other.zip\n", sha256_hex(&installer), "0".repeat(64));
    let release = |tag: &str, draft: bool| {
        format!(
            r#"{{"tag_name":"{tag}","draft":{draft},"body":"notes {tag}","assets":[
              {{"name":"HearthSetup.exe","browser_download_url":"{{base}}/dl/{tag}/HearthSetup.exe"}},
              {{"name":"SHA256SUMS.txt","browser_download_url":"{{base}}/dl/{tag}/SHA256SUMS.txt"}}]}}"#
        )
    };
    let list = format!(
        "[{},{},{}]",
        release("v0.3.14", false),
        release("v0.4.0-beta.1", false),
        release("v0.5.0", true)
    );
    let base = serve(vec![
        ("/repos/AhmiDarrow/Hearth/releases".into(), list.into_bytes()),
        ("/dl/v0.4.0-beta.1/HearthSetup.exe".into(), installer.clone()),
        ("/dl/v0.4.0-beta.1/SHA256SUMS.txt".into(), sums.clone().into_bytes()),
    ]);
    let feed = Feed::GitHubAll {
        owner: "AhmiDarrow".into(),
        repo: "Hearth".into(),
        asset: "HearthSetup.exe".into(),
    };
    let mut cfg = UpdateConfig::with_trust(
        "0.3.14",
        feed,
        Trust::Checksums { file: "SHA256SUMS.txt".into(), publisher: None },
    );
    cfg.github_api = base;
    let release = match check(&cfg) {
        Ok(Some(r)) => r,
        other => unreachable!("{other:?}"),
    };
    assert_eq!(release.version, "v0.4.0-beta.1", "the prerelease is newer; the draft is skipped");
    assert_eq!(release.sha256.as_deref(), Some(sha256_hex(&installer).as_str()));
    let dir = std::env::temp_dir().join(format!("remedy-ui-sums-{}", std::process::id()));
    let file = download(&cfg, &release, &dir, |_, _| {});
    assert_eq!(file.as_ref().ok().and_then(|f| std::fs::read(f).ok()), Some(installer.clone()));
    // Bytes that do not match the published checksum are refused.
    let mut wrong = release.clone();
    wrong.sha256 = Some("1".repeat(64));
    assert!(matches!(verify(&cfg, &wrong, &installer), Err(UpdateError::Verification(_))));
    let _ = std::fs::remove_dir_all(&dir);
    cfg.current = "0.4.0".into();
    assert_eq!(check(&cfg), Ok(None), "a final release outranks its prerelease");
}

#[test]
fn checksum_files_are_read_like_sha256sum_writes_them() {
    let a = "a".repeat(64);
    assert_eq!(
        checksum_for(&format!("{a}  HearthSetup.exe"), "HearthSetup.exe").as_deref(),
        Some(a.as_str())
    );
    assert_eq!(
        checksum_for(&format!("{a} *HearthSetup.exe"), "HearthSetup.exe").as_deref(),
        Some(a.as_str())
    );
    assert_eq!(checksum_for("abc  HearthSetup.exe", "HearthSetup.exe"), None, "only real SHA-256 lines");
    assert_eq!(checksum_for(&format!("{a}  Other.exe"), "HearthSetup.exe"), None);
}

/// Authenticode against programs on this PC: Git for Windows is signed, rustup is not.
#[cfg(windows)]
#[test]
fn installers_must_be_signed_by_the_named_publisher() {
    use remedy_ui_services_update::verify_publisher;
    let signed = std::path::Path::new("C:/Program Files/Git/git-bash.exe");
    let home = std::env::var_os("USERPROFILE").map(std::path::PathBuf::from).unwrap_or_default();
    let unsigned = home.join(".cargo/bin/rustup.exe");
    assert!(signed.exists() && unsigned.exists(), "this test needs Git for Windows and rustup installed");
    let (_, thumb) = remedy_ui_services_update::authenticode_of(signed);
    let thumb = thumb.unwrap_or_else(|| unreachable!("Git for Windows is Authenticode-signed"));
    assert_eq!(thumb.len(), 40);
    assert!(verify_publisher(signed, &thumb.to_lowercase()).is_ok(), "thumbprints compare in any case");
    assert!(
        matches!(verify_publisher(signed, &"0".repeat(40)), Err(UpdateError::Verification(_))),
        "another publisher"
    );
    assert!(matches!(verify_publisher(&unsigned, &thumb), Err(UpdateError::Verification(_))), "unsigned");
    // The same file changed after signing is refused.
    let copy = std::env::temp_dir().join(format!("remedy-ui-tamper-{}.exe", std::process::id()));
    let mut bytes = std::fs::read(signed).unwrap_or_default();
    if let Some(b) = bytes.get_mut(4096) {
        *b ^= 0xFF;
    }
    let _ = std::fs::write(&copy, &bytes);
    assert!(matches!(verify_publisher(&copy, &thumb), Err(UpdateError::Verification(_))), "tampered");
    let _ = std::fs::remove_file(&copy);
}
