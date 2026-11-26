//! Update check: newer releases on GitHub, their changelog, and verified downloads.
//!
//! The release list comes from the GitHub API (or `TRICERATOPS_UPDATE_URL`, for tests and
//! for trying a release by hand). Nothing about the user is sent: the only header is
//! `User-Agent: Triceratops/<version>`.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Where the release list lives.
pub fn base_url() -> String {
    std::env::var("TRICERATOPS_UPDATE_URL").unwrap_or_else(|_| "https://api.github.com".into())
}

const RELEASES: &str = "/repos/libano23/triceratops-rs/releases?per_page=20";

/// A release version, compared numerically.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u64, pub u64, pub u64);

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// `v0.4.0` / `0.4.0` → 0.4.0; anything else (`nightly`, `1.0`, `1.0.0-rc1`) → `None`.
pub fn parse_version(tag: &str) -> Option<Version> {
    let t = tag.trim().strip_prefix('v').unwrap_or(tag.trim());
    let parts: Vec<&str> = t.split('.').collect();
    if parts.len() != 3 || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    Some(Version(parts[0].parse().ok()?, parts[1].parse().ok()?, parts[2].parse().ok()?))
}

/// The running program's version.
pub fn current() -> Version {
    parse_version(env!("CARGO_PKG_VERSION")).unwrap_or(Version(0, 0, 0))
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub browser_download_url: String,
    #[serde(default)]
    pub digest: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    pub html_url: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

/// How this copy of Triceratops can be updated.
#[derive(Clone, Debug, PartialEq)]
pub enum Install {
    /// Replace this AppImage file.
    AppImage(PathBuf),
    /// Download the zip for the user to unzip.
    WindowsZip,
    /// A build from source or a package: point at the release page.
    ReleasePage,
}

impl Install {
    /// The kind of this running copy.
    pub fn detect() -> Install {
        if let Some(p) = std::env::var_os("APPIMAGE") {
            Install::AppImage(PathBuf::from(p))
        } else if cfg!(windows) {
            Install::WindowsZip
        } else {
            Install::ReleasePage
        }
    }

    fn asset_suffix(&self) -> Option<&'static str> {
        match self {
            Install::AppImage(_) => Some("x86_64.AppImage"),
            Install::WindowsZip => Some("windows-x64.zip"),
            Install::ReleasePage => None,
        }
    }
}

/// A newer version to offer.
#[derive(Clone, Debug)]
pub struct Offer {
    pub version: Version,
    /// Every newer release's notes, newest first, each under a `## <tag>` heading.
    pub changelog: String,
    pub html_url: String,
    /// The file to download for this kind of install (`None`: offer the release page).
    pub asset: Option<Asset>,
}

/// The newest published release newer than `current`, with the notes of every newer one.
pub fn offer(releases: &[Release], current: &Version, install: &Install) -> Option<Offer> {
    let mut newer: Vec<(Version, &Release)> = releases
        .iter()
        .filter(|r| !r.draft && !r.prerelease)
        .filter_map(|r| parse_version(&r.tag_name).map(|v| (v, r)))
        .filter(|(v, _)| v > current)
        .collect();
    newer.sort_by_key(|n| std::cmp::Reverse(n.0));
    let (version, latest) = *newer.first()?;
    let changelog = newer.iter().map(|(_, r)| format!("## {}\n\n{}\n\n", r.tag_name, r.body.as_deref().unwrap_or("").trim())).collect::<String>();
    let asset = install.asset_suffix().and_then(|suffix| latest.assets.iter().find(|a| a.name.ends_with(suffix)).cloned());
    Some(Offer { version, changelog: changelog.trim_end().to_string(), html_url: latest.html_url.clone(), asset })
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(10))).timeout_recv_body(Some(Duration::from_secs(60))).build().new_agent()
}

fn user_agent() -> String {
    format!("Triceratops/{}", env!("CARGO_PKG_VERSION"))
}

/// The release list (newest first, as GitHub returns it).
pub fn fetch_releases(base: &str) -> Result<Vec<Release>, String> {
    let url = format!("{}{RELEASES}", base.trim_end_matches('/'));
    let mut resp = agent().get(&url).header("User-Agent", user_agent()).header("Accept", "application/vnd.github+json").call().map_err(|e| format!("cannot reach {}: {e}", base))?;
    let text = resp.body_mut().with_config().limit(5 * 1024 * 1024).read_to_string().map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| format!("unexpected answer: {e}"))
}

/// Download `asset` to `dest`, reporting bytes received; verifies size and SHA-256.
/// On any failure or a cancel, `dest` is removed.
pub fn download(asset: &Asset, dest: &Path, progress: &dyn Fn(u64), cancel: &AtomicBool) -> Result<(), String> {
    let r = download_inner(asset, dest, progress, cancel);
    if r.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    r
}

fn download_inner(asset: &Asset, dest: &Path, progress: &dyn Fn(u64), cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    let resp = agent().get(&asset.browser_download_url).header("User-Agent", user_agent()).call().map_err(|e| format!("download failed: {e}"))?;
    let mut reader = resp.into_body().into_reader();
    let mut file = std::fs::File::create(dest).map_err(|e| format!("cannot write {}: {e}", dest.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let n = reader.read(&mut buf).map_err(|e| format!("download failed: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| format!("cannot write {}: {e}", dest.display()))?;
        total += n as u64;
        progress(total);
    }
    file.sync_all().map_err(|e| e.to_string())?;
    if total != asset.size {
        return Err(format!("incomplete download: {total} of {} bytes", asset.size));
    }
    if let Some(want) = asset.digest.as_deref().and_then(|d| d.strip_prefix("sha256:")) {
        let got: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if !got.eq_ignore_ascii_case(want) {
            return Err("the download does not match its checksum".into());
        }
    }
    Ok(())
}

/// Put a verified AppImage in place of `target`: executable, then an atomic rename (the
/// running program keeps its mounted copy).
pub fn replace_appimage(downloaded: &Path, target: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(downloaded, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    std::fs::rename(downloaded, target).map_err(|e| format!("cannot replace {}: {e}", target.display()))
}

/// Where the download goes: next to the AppImage (so the rename stays on one filesystem),
/// otherwise the Downloads folder (or the temp folder).
pub fn download_target(install: &Install, asset: &Asset) -> PathBuf {
    match install {
        Install::AppImage(p) => {
            let mut s = p.clone().into_os_string();
            s.push(".download");
            PathBuf::from(s)
        }
        _ => dirs::download_dir().unwrap_or_else(std::env::temp_dir).join(&asset.name),
    }
}

/// What a background job reports.
pub enum Event {
    /// The release list was read: the offer, if any.
    Checked(Result<Option<Offer>, String>),
    /// A download finished (the verified file) or failed.
    Downloaded(Result<PathBuf, String>),
}

/// Background release checks and downloads, reporting over a channel.
pub struct Checker {
    rx: Receiver<Event>,
    pub received: Arc<AtomicU64>,
    pub cancel: Arc<AtomicBool>,
}

impl Checker {
    /// Read the release list on a thread.
    pub fn check(install: Install) -> Checker {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let r = fetch_releases(&base_url()).map(|list| offer(&list, &current(), &install));
            let _ = tx.send(Event::Checked(r));
        });
        Checker { rx, received: Arc::new(AtomicU64::new(0)), cancel: Arc::new(AtomicBool::new(false)) }
    }

    /// Download `asset` to `dest` on a thread.
    pub fn download(asset: Asset, dest: PathBuf) -> Checker {
        let (tx, rx) = channel();
        let received = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (r2, c2) = (received.clone(), cancel.clone());
        std::thread::spawn(move || {
            let r = download(&asset, &dest, &|n| r2.store(n, Ordering::Relaxed), &c2).map(|_| dest);
            let _ = tx.send(Event::Downloaded(r));
        });
        Checker { rx, received, cancel }
    }

    /// The job's result, once it is done.
    pub fn poll(&self) -> Option<Event> {
        self.rx.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn releases() -> Vec<Release> {
        serde_json::from_str(
            r#"[
            {"tag_name": "v0.6.0", "body": "draft", "draft": true, "prerelease": false, "html_url": "u6", "assets": []},
            {"tag_name": "v0.5.0", "body": "* Five", "draft": false, "prerelease": false, "html_url": "u5",
             "assets": [{"name": "Triceratops-0.5.0-x86_64.AppImage", "size": 3, "browser_download_url": "a", "digest": "sha256:00"},
                        {"name": "Triceratops-0.5.0-windows-x64.zip", "size": 4, "browser_download_url": "z", "digest": null}]},
            {"tag_name": "v0.4.0-rc1", "body": "rc", "draft": false, "prerelease": true, "html_url": "u4rc", "assets": []},
            {"tag_name": "v0.4.0", "body": null, "draft": false, "prerelease": false, "html_url": "u4", "assets": []},
            {"tag_name": "nightly", "body": "n", "draft": false, "prerelease": false, "html_url": "un", "assets": []},
            {"tag_name": "v0.3.0", "body": "* Three", "draft": false, "prerelease": false, "html_url": "u3", "assets": []}
        ]"#,
        )
        .unwrap()
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(parse_version("0.10.0").unwrap() > parse_version("0.9.1").unwrap());
        assert_eq!(parse_version("v0.3.0"), Some(Version(0, 3, 0)));
        for bad in ["nightly", "1.0", "v1.0.0-rc1", "1.0.0.0", "v", ""] {
            assert_eq!(parse_version(bad), None, "{bad}");
        }
        assert_eq!(current(), parse_version(env!("CARGO_PKG_VERSION")).unwrap());
    }

    #[test]
    fn offer_lists_newer_releases_only() {
        let o = offer(&releases(), &Version(0, 3, 0), &Install::ReleasePage).unwrap();
        assert_eq!(o.version, Version(0, 5, 0));
        assert_eq!(o.html_url, "u5");
        assert!(o.changelog.starts_with("## v0.5.0\n\n* Five"), "{}", o.changelog);
        let p5 = o.changelog.find("## v0.5.0").unwrap();
        let p4 = o.changelog.find("## v0.4.0").unwrap();
        assert!(p5 < p4);
        for absent in ["v0.3.0", "rc", "draft", "nightly"] {
            assert!(!o.changelog.contains(absent), "{absent} in {}", o.changelog);
        }
        assert!(offer(&releases(), &Version(0, 5, 0), &Install::ReleasePage).is_none());
    }

    #[test]
    fn assets_by_install_kind() {
        let rel = releases();
        let a = offer(&rel, &Version(0, 3, 0), &Install::AppImage("/x/T.AppImage".into())).unwrap().asset.unwrap();
        assert!(a.name.ends_with("x86_64.AppImage"));
        let z = offer(&rel, &Version(0, 3, 0), &Install::WindowsZip).unwrap().asset.unwrap();
        assert!(z.name.ends_with("windows-x64.zip"));
        assert!(offer(&rel, &Version(0, 3, 0), &Install::ReleasePage).unwrap().asset.is_none());
        // a release without the file for this install: no asset, the page is offered
        let only4: Vec<Release> = rel.into_iter().filter(|r| r.tag_name == "v0.4.0").collect();
        assert!(offer(&only4, &Version(0, 3, 0), &Install::WindowsZip).unwrap().asset.is_none());
        assert_eq!(download_target(&Install::AppImage("/x/T.AppImage".into()), &Asset { name: "n".into(), size: 0, browser_download_url: String::new(), digest: None }), PathBuf::from("/x/T.AppImage.download"));
    }

    /// Serve `body` once per connection over plain HTTP/1.1, on a free local port.
    fn serve(body: Vec<u8>, connections: usize) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(connections) {
                let Ok(mut s) = stream else { continue };
                let mut req = Vec::new();
                let mut b = [0u8; 1024];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    let Ok(n) = s.read(&mut b) else { break };
                    if n == 0 {
                        break;
                    }
                    req.extend_from_slice(&b[..n]);
                }
                let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(&body);
            }
        });
        format!("http://{addr}")
    }

    fn sha(bytes: &[u8]) -> String {
        format!("sha256:{}", Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect::<String>())
    }

    #[test]
    fn download_verifies_and_cancels() {
        let body = b"new triceratops".to_vec();
        let dir = std::env::temp_dir().join(format!("tri-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("T.AppImage.download");
        let url = serve(body.clone(), 3);
        let asset = |digest: String| Asset { name: "T.AppImage".into(), size: body.len() as u64, browser_download_url: format!("{url}/file"), digest: Some(digest) };
        // good: written, progress reaches the size
        let seen = AtomicU64::new(0);
        download(&asset(sha(&body)), &dest, &|n| seen.store(n, Ordering::Relaxed), &AtomicBool::new(false)).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert_eq!(seen.load(Ordering::Relaxed), body.len() as u64);
        // wrong checksum: refused, file removed
        let e = download(&asset(sha(b"something else")), &dest, &|_| {}, &AtomicBool::new(false)).unwrap_err();
        assert!(e.contains("checksum"), "{e}");
        assert!(!dest.exists());
        // wrong size: refused
        let mut short = asset(sha(&body));
        short.size += 1;
        assert!(download(&short, &dest, &|_| {}, &AtomicBool::new(false)).unwrap_err().contains("incomplete"));
        assert!(!dest.exists());
        // cancelled: nothing written
        assert_eq!(download(&asset(sha(&body)), &dest, &|_| {}, &AtomicBool::new(true)).unwrap_err(), "cancelled");
        assert!(!dest.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn checker_reads_the_release_list() {
        let json = serde_json::to_vec(&serde_json::json!([{"tag_name": "v99.0.0", "body": "* Far future", "draft": false, "prerelease": false, "html_url": "u", "assets": []}])).unwrap();
        let url = serve(json, 1);
        let list = fetch_releases(&url).unwrap();
        let o = offer(&list, &current(), &Install::ReleasePage).unwrap();
        assert_eq!(o.version, Version(99, 0, 0));
        assert!(fetch_releases("http://127.0.0.1:1").unwrap_err().starts_with("cannot reach"));
    }

    #[test]
    fn replace_appimage_swaps_the_file() {
        let dir = std::env::temp_dir().join(format!("tri-replace-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("Triceratops.AppImage");
        std::fs::write(&target, b"old").unwrap();
        let dl = dir.join("Triceratops.AppImage.download");
        std::fs::write(&dl, b"new").unwrap();
        replace_appimage(&dl, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert!(!dl.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o755);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
