//! Checks GitHub Releases for a newer TRserial and installs it, following
//! NetQuota's updater: published releases only, a `SHA256SUMS` entry is
//! required, every download stays on GitHub hosts, and the files come from the
//! release workflow (`docs/releasing.md`).

use anyhow::{Context, Result, bail, ensure};
use reqwest::{StatusCode, Url, header::ACCEPT};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

const RELEASES: &str = "https://api.github.com/repos/KageRyo/TRserial/releases";
const CHECKSUMS: &str = "SHA256SUMS";
const USER_AGENT: &str = "TRserial-Update-Checker";
const MAX_RESPONSE: usize = 1 << 20;
const MAX_DOWNLOAD: u64 = 512 << 20;
const DOWNLOAD_PREFIX: &str = "trserial-update-";
const STALE_DOWNLOAD: Duration = Duration::from_secs(24 * 60 * 60);

/// How this copy can apply an update.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Method {
    /// Run the Windows installer, which replaces the installed copy.
    Installer,
    /// Replace the Linux executables in place and start the new version.
    ReplaceFiles,
    /// Point the user to the release page (portable Windows copy, other platforms).
    #[default]
    Manual,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Release {
    pub(crate) version: Version,
    pub(crate) page: String,
    pub(crate) asset: Option<Asset>,
    pub(crate) checksums: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Asset {
    pub(crate) name: String,
    pub(crate) url: String,
    pub(crate) size: u64,
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

pub(crate) fn current() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("Cargo package version is SemVer")
}

/// The release file this platform installs from; names match the release workflow.
fn platform_asset() -> Option<&'static str> {
    if cfg!(all(windows, target_arch = "x86_64")) {
        Some("trserial-windows-amd64-setup.exe")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("trserial-linux-amd64.tar.gz")
    } else {
        None
    }
}

/// An installed Windows copy has the Inno Setup uninstaller beside it; the
/// portable executable does not, and running the installer would put a second
/// copy elsewhere, so it is sent to the release page instead.
pub(crate) fn method(executable: &Path) -> Method {
    if cfg!(windows) {
        let installed = executable
            .parent()
            .and_then(|dir| std::fs::read_dir(dir).ok())
            .is_some_and(|entries| {
                entries.flatten().any(|entry| {
                    let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                    name.starts_with("unins") && name.ends_with(".exe")
                })
            });
        if installed {
            Method::Installer
        } else {
            Method::Manual
        }
    } else if cfg!(target_os = "linux") {
        Method::ReplaceFiles
    } else {
        Method::Manual
    }
}

pub(crate) fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                attempt.error("too many redirects")
            } else if validate_url(attempt.url().as_str()).is_ok() {
                attempt.follow()
            } else {
                attempt.error("redirect left GitHub")
            }
        }))
        .build()
        .context("無法建立更新連線")
}

/// Returns the newest published release above the running version, if any.
pub(crate) async fn check(client: &reqwest::Client) -> Result<Option<Release>> {
    let response = client
        .get(RELEASES)
        .header(ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .context("無法連線到 GitHub")?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    ensure!(
        response.status().is_success(),
        "GitHub 回應 HTTP {}",
        response.status()
    );
    let body = read_limited(response, MAX_RESPONSE).await?;
    let releases: Vec<ApiRelease> =
        serde_json::from_slice(&body).context("無法解析 GitHub 的版本清單")?;
    Ok(select(releases, &current(), platform_asset()))
}

/// Skips drafts, tags that are not SemVer, and pre-releases unless the running
/// version is itself a pre-release; then picks the highest version above `current`.
fn select(releases: Vec<ApiRelease>, current: &Version, asset: Option<&str>) -> Option<Release> {
    let allow_pre = !current.pre.is_empty();
    let (version, release) = releases
        .into_iter()
        .filter(|release| !release.draft)
        .filter_map(|release| {
            let tag = release
                .tag_name
                .strip_prefix('v')
                .unwrap_or(&release.tag_name);
            Some((Version::parse(tag).ok()?, release))
        })
        .filter(|(version, release)| allow_pre || (!release.prerelease && version.pre.is_empty()))
        .filter(|(version, _)| version > current)
        .max_by(|(left, _), (right, _)| left.cmp(right))?;
    let find = |name: &str| release.assets.iter().find(|asset| asset.name == name);
    Some(Release {
        asset: asset.and_then(find).map(|asset| Asset {
            name: asset.name.clone(),
            url: asset.browser_download_url.clone(),
            size: asset.size,
        }),
        checksums: find(CHECKSUMS).map(|asset| asset.browser_download_url.clone()),
        page: release.html_url,
        version,
    })
}

/// Only HTTPS URLs on GitHub release hosts, without embedded credentials.
fn validate_url(raw: &str) -> Result<()> {
    let url = Url::parse(raw).with_context(|| format!("無效的下載網址：{raw}"))?;
    let host = url.host_str().unwrap_or_default().trim_end_matches('.');
    ensure!(
        url.scheme() == "https" && url.username().is_empty() && url.password().is_none(),
        "不接受的下載網址：{raw}"
    );
    ensure!(
        host == "github.com"
            || host == "api.github.com"
            || host.ends_with(".githubusercontent.com"),
        "下載網址不在 GitHub：{raw}"
    );
    Ok(())
}

/// The lowercase SHA-256 listed for `asset` in a `sha256sum` manifest.
fn checksum_for(manifest: &str, asset: &str) -> Result<String> {
    let mut found: Option<String> = None;
    for line in manifest.lines() {
        let mut fields = line.split_whitespace();
        let (Some(hash), Some(name)) = (fields.next(), fields.next()) else {
            continue;
        };
        if name.trim_start_matches('*') != asset {
            continue;
        }
        let hash = hash.to_ascii_lowercase();
        ensure!(
            hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
            "SHA256SUMS 中 {asset} 的檢查碼格式錯誤"
        );
        ensure!(
            found.as_ref().is_none_or(|previous| *previous == hash),
            "SHA256SUMS 中 {asset} 有互相衝突的檢查碼"
        );
        found = Some(hash);
    }
    found.with_context(|| format!("SHA256SUMS 沒有 {asset}"))
}

async fn read_limited(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.context("讀取 GitHub 回應失敗")? {
        ensure!(body.len() + chunk.len() <= limit, "GitHub 回應過大");
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Hashes and counts a download so a short, long or altered file is rejected.
struct Verifier {
    hasher: Sha256,
    written: u64,
    expected_size: u64,
}
impl Verifier {
    fn new(expected_size: u64) -> Self {
        Self {
            hasher: Sha256::new(),
            written: 0,
            expected_size,
        }
    }
    fn update(&mut self, chunk: &[u8]) -> Result<()> {
        self.written += chunk.len() as u64;
        ensure!(
            self.written <= self.expected_size,
            "下載檔比版本資訊記載的大"
        );
        self.hasher.update(chunk);
        Ok(())
    }
    fn finish(self, expected: &str) -> Result<()> {
        ensure!(
            self.written == self.expected_size,
            "下載檔大小與版本資訊不符"
        );
        let actual = format!("{:x}", self.hasher.finalize());
        ensure!(actual == expected, "下載檔的 SHA-256 不符，已捨棄");
        Ok(())
    }
}

/// Downloads the platform file into a new temporary folder and verifies it
/// against `SHA256SUMS` before returning its path.
pub(crate) async fn download(
    client: &reqwest::Client,
    release: &Release,
    progress: impl Fn(u64, u64),
) -> Result<PathBuf> {
    let asset = release
        .asset
        .as_ref()
        .context("這個版本沒有適用於此平台的檔案")?;
    let checksums = release
        .checksums
        .as_deref()
        .context("這個版本沒有 SHA256SUMS，無法確認下載檔")?;
    validate_url(&asset.url)?;
    validate_url(checksums)?;
    ensure!(
        Path::new(&asset.name).file_name() == Some(asset.name.as_ref()),
        "不接受的檔名：{}",
        asset.name
    );
    ensure!(asset.size <= MAX_DOWNLOAD, "更新檔過大");
    let manifest = client
        .get(checksums)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .context("無法下載 SHA256SUMS")?;
    let manifest = read_limited(manifest, MAX_RESPONSE).await?;
    let expected = checksum_for(&String::from_utf8_lossy(&manifest), &asset.name)?;

    let dir = std::env::temp_dir().join(format!(
        "{DOWNLOAD_PREFIX}{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    std::fs::create_dir(&dir).context("無法建立更新暫存資料夾")?;
    let path = dir.join(&asset.name);
    let result = async {
        let mut response = client
            .get(&asset.url)
            .header(ACCEPT, "application/octet-stream")
            .timeout(Duration::from_secs(600))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .context("無法下載更新檔")?;
        if let Some(length) = response.content_length() {
            ensure!(length == asset.size, "下載檔大小與版本資訊不符");
        }
        let mut file = std::fs::File::create(&path)?;
        let mut verifier = Verifier::new(asset.size);
        while let Some(chunk) = response.chunk().await.context("下載更新檔中斷")? {
            verifier.update(&chunk)?;
            file.write_all(&chunk)?;
            progress(verifier.written, asset.size);
        }
        file.sync_all()?;
        verifier.finish(&expected)
    }
    .await;
    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(error);
    }
    Ok(path)
}

/// Starts the interactive installer; Inno Setup replaces the installed copy
/// after TRserial exits. The installer stays in the temporary folder until
/// [`cleanup_stale_downloads`] removes it a day later.
#[cfg(windows)]
pub(crate) fn launch_installer(installer: &Path) -> Result<()> {
    std::process::Command::new(installer)
        .spawn()
        .context("無法開啟安裝程式")?;
    Ok(())
}

/// Replaces `trserial-desktop`, and `trserial` when it sits beside it, with the
/// files from the downloaded archive. Each original is kept as a hard link until
/// its replacement is in place, so a failure restores it.
#[cfg(target_os = "linux")]
pub(crate) fn replace_files(archive: &Path, executable: &Path) -> Result<()> {
    let dir = executable.parent().context("無法取得程式所在資料夾")?;
    let staging = dir.join(format!(".trserial-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir(&staging)
        .with_context(|| format!("無法寫入 {}，請改用手動更新", dir.display()))?;
    let result = (|| {
        let found = extract(archive, &staging)?;
        ensure!(found.contains(&"trserial-desktop"), "更新檔缺少桌面版程式");
        for name in ["trserial-desktop", "trserial"] {
            let target = if name == "trserial-desktop" {
                executable.to_path_buf()
            } else {
                dir.join(name)
            };
            if name != "trserial-desktop" && !target.exists() {
                continue;
            }
            ensure!(found.contains(&name), "更新檔缺少 {name}");
            replace(&staging.join(name), &target)?;
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&staging);
    result
}

#[cfg(target_os = "linux")]
fn replace(staged: &Path, target: &Path) -> Result<()> {
    let backup = target.with_file_name(format!(
        ".{}.backup",
        target.file_name().unwrap_or_default().to_string_lossy()
    ));
    let _ = std::fs::remove_file(&backup);
    std::fs::hard_link(target, &backup)
        .with_context(|| format!("無法備份 {}", target.display()))?;
    if let Err(error) = std::fs::rename(staged, target) {
        let _ = std::fs::remove_file(&backup);
        return Err(error).with_context(|| format!("無法替換 {}", target.display()));
    }
    let _ = std::fs::remove_file(&backup);
    Ok(())
}

/// Extracts only the two executables from `<package>/` in the release archive,
/// rejecting absolute or parent paths and anything but regular files.
#[cfg(target_os = "linux")]
fn extract(archive: &Path, staging: &Path) -> Result<Vec<&'static str>> {
    use std::{io::Read, os::unix::fs::OpenOptionsExt, path::Component};
    let file = std::fs::File::open(archive).context("無法開啟更新檔")?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let mut found = Vec::new();
    for entry in tar.entries().context("無法讀取更新檔")? {
        let mut entry = entry.context("無法讀取更新檔")?;
        let path = entry.path()?.into_owned();
        let parts: Vec<_> = path.components().collect();
        ensure!(
            !parts.is_empty()
                && parts
                    .iter()
                    .all(|part| matches!(part, Component::Normal(_))),
            "更新檔含不安全的路徑：{}",
            path.display()
        );
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            continue;
        }
        ensure!(kind.is_file(), "更新檔含不支援的項目：{}", path.display());
        let Some(name) = (parts.len() == 2)
            .then(|| {
                ["trserial-desktop", "trserial"]
                    .into_iter()
                    .find(|n| parts[1].as_os_str() == *n)
            })
            .flatten()
        else {
            continue;
        };
        ensure!(!found.contains(&name), "更新檔有重複的 {name}");
        let size = entry.header().size()?;
        ensure!(size <= MAX_DOWNLOAD, "更新檔中的 {name} 過大");
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o755)
            .open(staging.join(name))?;
        let written = std::io::copy(&mut (&mut entry).take(MAX_DOWNLOAD + 1), &mut out)?;
        ensure!(written == size, "更新檔中的 {name} 不完整");
        out.sync_all()?;
        found.push(name);
    }
    Ok(found)
}

/// Removes update folders left in the temporary directory for over a day.
pub(crate) fn cleanup_stale_downloads() {
    cleanup_in(&std::env::temp_dir(), SystemTime::now());
}

fn cleanup_in(root: &Path, now: SystemTime) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let stale = entry
            .file_name()
            .to_string_lossy()
            .starts_with(DOWNLOAD_PREFIX)
            && entry.file_type().is_ok_and(|kind| kind.is_dir())
            && entry
                .metadata()
                .and_then(|meta| meta.modified())
                .is_ok_and(|modified| {
                    now.duration_since(modified).unwrap_or_default() > STALE_DOWNLOAD
                });
        if stale {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Fails when the running build has no automatic path, so callers can offer the
/// release page instead.
pub(crate) fn ensure_supported(method: Method) -> Result<()> {
    if method == Method::Manual {
        bail!("這個版本請從 GitHub 下載安裝");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, draft: bool, prerelease: bool, assets: &[&str]) -> ApiRelease {
        ApiRelease {
            tag_name: tag.into(),
            html_url: format!("https://github.com/KageRyo/TRserial/releases/tag/{tag}"),
            draft,
            prerelease,
            assets: assets
                .iter()
                .map(|name| ApiAsset {
                    name: (*name).into(),
                    browser_download_url: format!(
                        "https://github.com/KageRyo/TRserial/releases/download/{tag}/{name}"
                    ),
                    size: 7,
                })
                .collect(),
        }
    }

    #[test]
    fn picks_the_newest_published_release_and_its_platform_files() {
        let current = Version::parse("0.1.0").unwrap();
        let asset = "trserial-windows-amd64-setup.exe";
        let found = select(
            vec![
                release("v0.1.0", false, false, &[asset, CHECKSUMS]),
                release("v0.3.0", true, false, &[asset, CHECKSUMS]),
                release("v0.2.1-beta.1", false, true, &[asset, CHECKSUMS]),
                release("nightly", false, false, &[asset]),
                release("v0.2.0", false, false, &[asset, CHECKSUMS]),
                release("v0.1.5", false, false, &[asset, CHECKSUMS]),
            ],
            &current,
            Some(asset),
        )
        .unwrap();
        assert_eq!(found.version, Version::parse("0.2.0").unwrap());
        assert_eq!(found.asset.unwrap().name, asset);
        assert!(found.checksums.unwrap().ends_with("/v0.2.0/SHA256SUMS"));

        let up_to_date = vec![release("v0.1.0", false, false, &[asset])];
        assert!(select(up_to_date, &current, Some(asset)).is_none());
    }

    #[test]
    fn pre_releases_are_offered_only_to_pre_release_builds() {
        let pre = Version::parse("0.2.0-beta.1").unwrap();
        let found = select(vec![release("v0.2.0-beta.2", false, true, &[])], &pre, None).unwrap();
        assert_eq!(found.version, Version::parse("0.2.0-beta.2").unwrap());
        assert!(found.asset.is_none() && found.checksums.is_none());
    }

    #[test]
    fn downloads_must_stay_on_github_over_https() {
        for ok in [
            "https://github.com/KageRyo/TRserial/releases/download/v1.0.0/SHA256SUMS",
            "https://release-assets.githubusercontent.com/github-production-release-asset/1",
        ] {
            assert!(validate_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "http://github.com/KageRyo/TRserial/releases/download/v1.0.0/SHA256SUMS",
            "https://example.com/trserial-windows-amd64-setup.exe",
            "https://github.com.example.com/a",
            "https://user:secret@github.com/a",
        ] {
            assert!(validate_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn checksum_lookup_rejects_missing_malformed_and_conflicting_entries() {
        let good = "a".repeat(64);
        let other = "b".repeat(64);
        let manifest = format!("{good}  setup.exe\n{other} *portable.exe\n");
        assert_eq!(checksum_for(&manifest, "setup.exe").unwrap(), good);
        assert_eq!(checksum_for(&manifest, "portable.exe").unwrap(), other);
        assert!(checksum_for(&manifest, "missing.zip").is_err());
        assert!(checksum_for("xyz  setup.exe", "setup.exe").is_err());
        assert!(
            checksum_for(
                &format!("{good}  setup.exe\n{other}  setup.exe"),
                "setup.exe"
            )
            .is_err()
        );
    }

    #[test]
    fn verifier_rejects_altered_short_and_oversized_downloads() {
        let expected = format!("{:x}", Sha256::digest(b"trserial"));
        let mut ok = Verifier::new(8);
        ok.update(b"tr").unwrap();
        ok.update(b"serial").unwrap();
        ok.finish(&expected).unwrap();

        let mut altered = Verifier::new(8);
        altered.update(b"TRSERIAL").unwrap();
        assert!(altered.finish(&expected).is_err());

        let mut short = Verifier::new(8);
        short.update(b"trseria").unwrap();
        assert!(short.finish(&expected).is_err());

        assert!(Verifier::new(4).update(b"trserial").is_err());
    }

    #[test]
    fn cleanup_removes_only_old_update_folders() {
        let root = tempfile::TempDir::new().unwrap();
        let update = root.path().join(format!("{DOWNLOAD_PREFIX}1"));
        let unrelated = root.path().join("keep-me");
        std::fs::create_dir(&update).unwrap();
        std::fs::create_dir(&unrelated).unwrap();
        cleanup_in(root.path(), SystemTime::now());
        assert!(update.exists(), "a fresh download is kept");
        cleanup_in(root.path(), SystemTime::now() + STALE_DOWNLOAD * 2);
        assert!(!update.exists() && unrelated.exists());
    }

    #[cfg(target_os = "linux")]
    fn archive(entries: &[(&str, &[u8])]) -> tempfile::NamedTempFile {
        let file = tempfile::NamedTempFile::new().unwrap();
        let encoder = flate2::write::GzEncoder::new(file.reopen().unwrap(), Default::default());
        let mut builder = tar::Builder::new(encoder);
        for (path, data) in entries {
            // Write the name field directly: the tar crate refuses to create the
            // `..` entries these tests need to feed the extractor.
            let mut header = tar::Header::new_gnu();
            header.as_gnu_mut().unwrap().name[..path.len()].copy_from_slice(path.as_bytes());
            header.set_entry_type(tar::EntryType::Regular);
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append(&header, *data).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap();
        file
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_update_replaces_both_executables_from_the_archive() {
        let dir = tempfile::TempDir::new().unwrap();
        let desktop = dir.path().join("trserial-desktop");
        std::fs::write(&desktop, b"old desktop").unwrap();
        std::fs::write(dir.path().join("trserial"), b"old cli").unwrap();
        let package = archive(&[
            ("trserial-linux-amd64/README.md", b"docs"),
            ("trserial-linux-amd64/trserial-desktop", b"new desktop"),
            ("trserial-linux-amd64/trserial", b"new cli"),
        ]);
        replace_files(package.path(), &desktop).unwrap();
        assert_eq!(std::fs::read(&desktop).unwrap(), b"new desktop");
        assert_eq!(
            std::fs::read(dir.path().join("trserial")).unwrap(),
            b"new cli"
        );
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| name.starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_update_rejects_unsafe_archives_and_keeps_the_old_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let desktop = dir.path().join("trserial-desktop");
        std::fs::write(&desktop, b"old desktop").unwrap();
        let escaping = archive(&[("trserial-linux-amd64/../../trserial-desktop", b"evil")]);
        assert!(replace_files(escaping.path(), &desktop).is_err());
        let missing = archive(&[("trserial-linux-amd64/trserial", b"cli only")]);
        assert!(replace_files(missing.path(), &desktop).is_err());
        assert_eq!(std::fs::read(&desktop).unwrap(), b"old desktop");
    }
}
