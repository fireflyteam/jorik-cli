//! Release integrity requires both SHA-256 metadata and Sigstore build provenance.
//! The trusted GitHub CLI verifies the signing identity, workflow, and exact tag.
//! No Jorik credentials or configurable server URL are passed to the verifier.

use anyhow::{Context, Result, bail, ensure};
use reqwest::{Client, Response, Url, redirect};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
#[cfg(target_os = "linux")]
use std::fs;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    time::Duration,
};
use tempfile::{NamedTempFile, TempDir};

const RELEASES_API: &str = "https://api.github.com/repos/fireflyteam/jorik-cli/releases";
const DOWNLOAD_BASE: &str = "https://github.com/fireflyteam/jorik-cli/releases/download/";
const MAX_METADATA: usize = 4 * 1024 * 1024;
const MAX_DOWNLOAD: u64 = 128 * 1024 * 1024;
#[cfg(any(target_os = "linux", test))]
const MAX_BINARY: u64 = 256 * 1024 * 1024;
#[cfg(any(target_os = "linux", test))]
const MAX_UNPACKED: u64 = MAX_BINARY + 1024 * 1024;

#[derive(Deserialize, Clone, Debug)]
pub struct ReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
    pub size: u64,
    pub digest: Option<String>,
}

#[derive(Deserialize, Debug)]
struct Release {
    tag_name: String,
    assets: Vec<ReleaseAsset>,
    draft: bool,
    prerelease: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    LinuxX64,
    WindowsX64,
}

impl Target {
    fn current() -> Result<Self> {
        if cfg!(all(
            target_os = "linux",
            target_arch = "x86_64",
            target_env = "gnu"
        )) {
            Ok(Self::LinuxX64)
        } else if cfg!(all(
            target_os = "windows",
            target_arch = "x86_64",
            target_env = "msvc"
        )) {
            Ok(Self::WindowsX64)
        } else {
            bail!("automatic updates support Linux x86_64 GNU and Windows x86_64 MSVC only")
        }
    }

    fn asset_name(self, version: &Version) -> String {
        match self {
            Self::LinuxX64 => "jorik-x86_64-unknown-linux-gnu.tar.gz".to_string(),
            Self::WindowsX64 => format!("jorik-cli-{version}-x86_64-pc-windows-msvc-setup.exe"),
        }
    }
}

struct VerifiedAsset {
    url: Url,
    size: u64,
    digest: [u8; 32],
}

fn trusted_transport(url: &Url) -> bool {
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(
            url.host_str(),
            Some(
                "api.github.com"
                    | "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
            )
        )
}

fn client() -> Result<Client> {
    Client::builder()
        .user_agent(concat!("jorik-cli/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(180))
        .redirect(redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                attempt.error("too many release download redirects")
            } else if trusted_transport(attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("untrusted release download redirect")
            }
        }))
        .build()
        .context("building isolated updater client")
}

fn tag_version(tag: &str) -> Result<Version> {
    let version = tag.strip_prefix('v').unwrap_or(tag);
    let parsed = Version::parse(version).context("invalid release version")?;
    ensure!(
        parsed.to_string() == version,
        "non-canonical release version"
    );
    Ok(parsed)
}

fn newer_on_channel(release: &Release, current: &Version) -> bool {
    if release.draft {
        return false;
    }
    let Ok(version) = tag_version(&release.tag_name) else {
        return false;
    };
    // Stable installations do not silently opt into prereleases. Prerelease
    // installations may upgrade to a newer prerelease or stable release.
    !(current.pre.is_empty() && (release.prerelease || !version.pre.is_empty()))
        && version.cmp_precedence(current).is_gt()
}

fn latest_release(releases: Vec<Release>, current: &Version) -> Option<Release> {
    releases
        .into_iter()
        .filter(|release| newer_on_channel(release, current))
        .max_by(|a, b| {
            tag_version(&a.tag_name)
                .unwrap()
                .cmp_precedence(&tag_version(&b.tag_name).unwrap())
        })
}

async fn metadata<T: serde::de::DeserializeOwned>(
    client: &Client,
    url: Url,
    timeout: Duration,
) -> Result<T> {
    ensure!(
        url.host_str() == Some("api.github.com") && trusted_transport(&url),
        "untrusted release API"
    );
    let response = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2026-03-10")
        .timeout(timeout)
        .send()
        .await
        .context("fetching release metadata")?;
    let bytes = bounded_body(response, MAX_METADATA).await?;
    serde_json::from_slice(&bytes).context("invalid release metadata")
}

async fn bounded_body(mut response: Response, maximum: usize) -> Result<Vec<u8>> {
    ensure!(
        response.status().is_success(),
        "release server returned {}",
        response.status()
    );
    if let Some(size) = response.content_length() {
        ensure!(size <= maximum as u64, "release metadata is too large");
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.context("reading release metadata")? {
        ensure!(
            chunk.len() <= maximum.saturating_sub(body.len()),
            "release metadata is too large"
        );
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub async fn check_for_updates() -> Option<(String, Vec<ReleaseAsset>)> {
    let client = client().ok()?;
    let releases = metadata::<Vec<Release>>(
        &client,
        Url::parse(&format!("{RELEASES_API}?per_page=100")).ok()?,
        Duration::from_secs(2),
    )
    .await
    .ok()?;
    let current = Version::parse(env!("CARGO_PKG_VERSION")).ok()?;
    let release = latest_release(releases, &current)?;
    Some((release.tag_name, release.assets))
}

fn release_url(tag: &str, download_name: Option<&str>) -> Result<Url> {
    tag_version(tag)?;
    let mut url = if download_name.is_some() {
        Url::parse(DOWNLOAD_BASE)?
    } else {
        Url::parse("https://github.com/fireflyteam/jorik-cli/releases/tag/")?
    };
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("invalid release URL"))?
        .pop_if_empty()
        .push(tag);
    if let Some(name) = download_name {
        url.path_segments_mut().unwrap().push(name);
    }
    Ok(url)
}

fn digest_bytes(digest: Option<&str>) -> Result<[u8; 32]> {
    let hex = digest
        .and_then(|value| value.strip_prefix("sha256:"))
        .context("release asset has no GitHub SHA-256 digest; refusing unverified installation")?;
    ensure!(
        hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "invalid release SHA-256 digest"
    );
    let mut digest = [0; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)?;
    }
    Ok(digest)
}

fn select_asset(release: &Release, requested_tag: &str, target: Target) -> Result<VerifiedAsset> {
    ensure!(
        release.tag_name == requested_tag && !release.draft,
        "release metadata does not match the confirmed tag"
    );
    let name = target.asset_name(&tag_version(requested_tag)?);
    let mut matches = release.assets.iter().filter(|asset| asset.name == name);
    let asset = matches
        .next()
        .with_context(|| format!("release has no compatible asset named {name}"))?;
    ensure!(
        matches.next().is_none(),
        "release has duplicate compatible assets"
    );
    let url = Url::parse(&asset.browser_download_url).context("invalid asset URL")?;
    ensure!(
        url == release_url(requested_tag, Some(&name))?,
        "asset URL is not pinned to the confirmed repository, tag and filename"
    );
    ensure!(
        asset.size > 0 && asset.size <= MAX_DOWNLOAD,
        "release asset size is empty or exceeds the download limit"
    );
    Ok(VerifiedAsset {
        url,
        size: asset.size,
        digest: digest_bytes(asset.digest.as_deref())?,
    })
}

async fn verified_response(
    mut response: Response,
    asset: &VerifiedAsset,
    directory: &Path,
    suffix: &str,
) -> Result<NamedTempFile> {
    ensure!(
        response.status().is_success(),
        "asset download returned {}",
        response.status()
    );
    if let Some(length) = response.content_length() {
        ensure!(
            length == asset.size,
            "asset Content-Length does not match release metadata"
        );
    }
    ensure!(
        response
            .headers()
            .get(reqwest::header::CONTENT_ENCODING)
            .is_none_or(|encoding| encoding == "identity"),
        "encoded release downloads are not supported"
    );
    let mut file = tempfile::Builder::new()
        .prefix("jorik-verified-")
        .suffix(suffix)
        .tempfile_in(directory)
        .context("creating private download file")?;
    let mut hash = Sha256::new();
    let mut count = 0u64;
    while let Some(chunk) = response.chunk().await.context("reading release download")? {
        ensure!(
            chunk.len() as u64 <= asset.size.saturating_sub(count),
            "download exceeds the declared asset size"
        );
        count += chunk.len() as u64;
        hash.update(&chunk);
        file.write_all(&chunk).context("staging release download")?;
    }
    ensure!(
        count == asset.size,
        "download size does not match release metadata"
    );
    let actual: [u8; 32] = hash.finalize().into();
    ensure!(
        actual == asset.digest,
        "SHA-256 mismatch; refusing to install downloaded bytes"
    );
    file.flush().context("flushing verified download")?;
    file.seek(SeekFrom::Start(0))?;
    Ok(file)
}

#[cfg(any(target_os = "linux", test))]
fn extract_linux_binary(source: &mut File, destination: &mut File) -> Result<()> {
    source.seek(SeekFrom::Start(0))?;
    let gzip = flate2::read::GzDecoder::new(source).take(MAX_UNPACKED + 1);
    let mut archive = tar::Archive::new(gzip);
    let mut found = false;
    for entry in archive.entries().context("reading release archive")? {
        let mut entry = entry.context("reading archive entry")?;
        // Never unpack archive paths. Only one regular file with this exact
        // package name may be copied into the caller's private staging file.
        ensure!(
            !found
                && entry.path_bytes().as_ref() == b"jorik"
                && entry.header().entry_type().is_file(),
            "archive must contain exactly one regular file named jorik"
        );
        ensure!(
            entry.size() >= 64 && entry.size() <= MAX_BINARY,
            "invalid executable size in archive"
        );
        let size = entry.size();
        let copied = std::io::copy(&mut entry, destination).context("staging executable")?;
        ensure!(copied == size, "truncated executable in archive");
        found = true;
    }
    ensure!(found, "release archive contains no executable");
    // Consume the gzip trailer too, validating CRC/length and bounding padding
    // and decompression bombs even after the tar end-of-archive marker.
    let mut gzip = archive.into_inner();
    std::io::copy(&mut gzip, &mut std::io::sink()).context("validating compressed archive")?;
    ensure!(
        gzip.limit() > 0,
        "uncompressed release archive exceeds the size limit"
    );
    destination.seek(SeekFrom::Start(0))?;
    let mut header = [0; 20];
    destination
        .read_exact(&mut header)
        .context("reading executable header")?;
    ensure!(
        &header[..4] == b"\x7fELF"
            && header[4] == 2
            && header[5] == 1
            && matches!(u16::from_le_bytes([header[16], header[17]]), 2 | 3)
            && u16::from_le_bytes([header[18], header[19]]) == 62,
        "release does not contain a Linux x86_64 ELF executable"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn install_linux(source: &mut File, destination: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::symlink_metadata(destination).context("inspecting installed executable")?;
    ensure!(
        metadata.file_type().is_file(),
        "installed executable is not a regular file"
    );
    let parent = destination
        .parent()
        .context("installed executable has no parent directory")?;
    let mut staged = tempfile::Builder::new().prefix(".jorik-update-").tempfile_in(parent)
        .context("cannot stage update beside installed binary; install manually with appropriate directory permissions")?;
    extract_linux_binary(source, staged.as_file_mut())?;
    staged
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o755))?;
    staged
        .as_file()
        .sync_all()
        .context("syncing staged executable")?;
    drop(
        staged
            .persist(destination)
            .map_err(|error| error.error)
            .context("atomically replacing installed executable")?,
    );
    if let Err(error) = File::open(parent).and_then(|directory| directory.sync_all()) {
        eprintln!("Warning: update installed, but directory sync failed: {error}");
    }
    Ok(())
}

fn validate_windows_installer(file: &mut File) -> Result<()> {
    file.seek(SeekFrom::Start(0))?;
    let mut header = [0; 64];
    file.read_exact(&mut header)
        .context("reading installer header")?;
    ensure!(
        &header[..2] == b"MZ",
        "release asset is not a Windows installer"
    );
    let offset = u32::from_le_bytes(header[60..64].try_into().unwrap());
    file.seek(SeekFrom::Start(u64::from(offset)))?;
    let mut signature = [0; 6];
    file.read_exact(&mut signature)
        .context("reading installer PE header")?;
    // Inno Setup can use an x86 bootstrap even for its x64-only installer.
    ensure!(
        &signature[..4] == b"PE\0\0"
            && matches!(
                u16::from_le_bytes([signature[4], signature[5]]),
                0x14c | 0x8664
            ),
        "invalid Windows installer PE header"
    );
    Ok(())
}

fn windows_command(installer: &Path, destination: &Path) -> Result<std::process::Command> {
    ensure!(
        destination
            .file_name()
            .is_some_and(|name| name == "jorik.exe"),
        "automatic Windows updates require the installed jorik.exe; use the release installer for development builds"
    );
    let parent = destination
        .parent()
        .context("installed executable has no parent directory")?;
    let mut directory = std::ffi::OsString::from("/DIR=");
    directory.push(parent);
    let mut command = std::process::Command::new(installer);
    command
        .args(["/SILENT", "/NORESTART", "/SP-"])
        .arg(directory);
    if let Some(parent) = installer.parent() {
        command.current_dir(parent);
    }
    Ok(command)
}

fn attestation_command(artifact: &Path, tag: &str) -> Result<tokio::process::Command> {
    tag_version(tag)?;
    let mut command = tokio::process::Command::new("gh");
    command
        .arg("attestation")
        .arg("verify")
        .arg(artifact)
        .args([
            "--hostname",
            "github.com",
            "--repo",
            "fireflyteam/jorik-cli",
            "--cert-oidc-issuer",
            "https://token.actions.githubusercontent.com",
            "--predicate-type",
            "https://slsa.dev/provenance/v1",
            "--cert-identity",
        ])
        .arg(format!(
            "https://github.com/fireflyteam/jorik-cli/.github/workflows/build.yml@refs/tags/{tag}"
        ))
        .arg("--source-ref")
        .arg(format!("refs/tags/{tag}"))
        .args([
            "--deny-self-hosted-runners",
            "--digest-alg",
            "sha256",
            "--limit",
            "5",
        ])
        .env("GH_HOST", "github.com")
        .env("GH_PROMPT_DISABLED", "1")
        .env_remove("JORIK_TOKEN")
        .env_remove("GH_DEBUG")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    Ok(command)
}

async fn verify_attestation(artifact: &Path, tag: &str) -> Result<()> {
    let status = tokio::time::timeout(
        Duration::from_secs(90),
        attestation_command(artifact, tag)?.status(),
    )
    .await
    .context("signed build verification timed out")?
    .context(
        "cannot run provenance verifier; install a current GitHub CLI (gh) and authenticate it",
    )?;
    ensure!(
        status.success(),
        "signed build provenance verification failed; check gh authentication/version and the release's attestations. No hash-only fallback is allowed"
    );
    Ok(())
}

fn recheck_download(file: &mut File, asset: &VerifiedAsset) -> Result<()> {
    file.seek(SeekFrom::Start(0))?;
    let mut hash = Sha256::new();
    let mut length = 0u64;
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        length += count as u64;
        ensure!(length <= asset.size, "verified download changed");
        hash.update(&buffer[..count]);
    }
    let digest: [u8; 32] = hash.finalize().into();
    ensure!(
        length == asset.size && digest == asset.digest,
        "verified download changed"
    );
    file.seek(SeekFrom::Start(0))?;
    Ok(())
}

async fn install_inner(tag: &str) -> Result<()> {
    let current = Version::parse(env!("CARGO_PKG_VERSION"))?;
    let version = tag_version(tag)?;
    ensure!(
        version.cmp_precedence(&current).is_gt(),
        "refusing a downgrade or same-version reinstall"
    );
    let target = Target::current()?;
    let client = client()?;
    let mut url = Url::parse(&format!("{RELEASES_API}/tags/"))?;
    url.path_segments_mut().unwrap().pop_if_empty().push(tag);
    // Do not install from metadata cached when the notification appeared.
    let release: Release = metadata(&client, url, Duration::from_secs(15)).await?;
    ensure!(
        newer_on_channel(&release, &current),
        "confirmed release is outside this installation's update channel"
    );
    let asset = select_asset(&release, tag, target)?;
    let staging = TempDir::new().context("creating private update directory")?;
    println!("Downloading and verifying {tag}…");
    let response = client
        .get(asset.url.clone())
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .send()
        .await
        .context("downloading release asset")?;
    let suffix = if target == Target::WindowsX64 {
        ".exe"
    } else {
        ".tar.gz"
    };
    let mut download = verified_response(response, &asset, staging.path(), suffix).await?;
    println!("Verifying signed build provenance…");
    verify_attestation(download.path(), tag).await?;
    recheck_download(download.as_file_mut(), &asset)?;
    let destination = std::env::current_exe().context("locating installed executable")?;
    match target {
        Target::LinuxX64 => {
            #[cfg(target_os = "linux")]
            {
                install_linux(download.as_file_mut(), &destination)?;
                println!("Update to {tag} installed. Restart Jorik to use it.");
            }
            #[cfg(not(target_os = "linux"))]
            bail!("Linux update installation is not supported on this operating system");
        }
        Target::WindowsX64 => {
            validate_windows_installer(download.as_file_mut())?;
            let path = download.path().to_path_buf();
            let mut command = windows_command(&path, &destination)?;
            // Close the writable file handle before CreateProcess. Keep the
            // random, private staging directory alive for the installer.
            let (file, _path) = download.keep().map_err(|error| error.error)?;
            drop(file);
            command
                .spawn()
                .context("starting verified release installer")?;
            let directory = staging.keep();
            println!(
                "Verified installer started for {tag}; Jorik will now exit.\nInstaller staging directory: {}",
                directory.display()
            );
        }
    }
    Ok(())
}

pub async fn install(tag: &str) -> Result<()> {
    let page = release_url(tag, None)?;
    install_inner(tag).await.with_context(|| {
        format!(
            "Update did not complete. No unverified fallback was attempted.\nManual release: {page}"
        )
    })
}

#[cfg(test)]
mod tests;
