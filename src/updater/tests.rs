use super::*;
use std::io::Cursor;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn asset(tag: &str, target: Target) -> ReleaseAsset {
    let name = target.asset_name(&tag_version(tag).unwrap());
    ReleaseAsset {
        browser_download_url: release_url(tag, Some(&name)).unwrap().to_string(),
        name,
        size: 4,
        digest: Some(format!("sha256:{:x}", Sha256::digest(b"test"))),
    }
}

fn release(tag: &str) -> Release {
    Release {
        tag_name: tag.to_string(),
        assets: vec![asset(tag, Target::LinuxX64), asset(tag, Target::WindowsX64)],
        draft: false,
        prerelease: !tag_version(tag).unwrap().pre.is_empty(),
    }
}

#[test]
fn stable_channel_skips_drafts_prereleases_and_same_version_builds() {
    let mut draft = release("v99.0.0");
    draft.draft = true;
    let mut mislabeled = release("v98.0.0");
    mislabeled.prerelease = true;
    let current = Version::parse("0.4.1").unwrap();
    let chosen = latest_release(
        vec![
            draft,
            mislabeled,
            release("v2.0.0-beta.1"),
            release("v0.4.1+build.2"),
            release("v0.3.0"),
            release("v0.5.0"),
        ],
        &current,
    )
    .unwrap();
    assert_eq!(chosen.tag_name, "v0.5.0");
    assert!(latest_release(vec![release("v0.4.1+other")], &current).is_none());
}

#[test]
fn prerelease_installations_can_advance_to_new_prereleases_or_stable() {
    let current = Version::parse("0.5.0-alpha.1").unwrap();
    assert_eq!(
        latest_release(vec![release("v0.5.0-beta.1")], &current)
            .unwrap()
            .tag_name,
        "v0.5.0-beta.1"
    );
    assert_eq!(
        latest_release(vec![release("v0.5.0-beta.1"), release("v0.5.0")], &current)
            .unwrap()
            .tag_name,
        "v0.5.0"
    );
}

#[test]
fn invalid_and_path_like_tags_are_rejected() {
    for tag in [
        "../latest",
        "latest",
        "v01.2.3",
        "vv1.2.3",
        "v1.2.3/asset",
        "v1.2.3?token=x",
        "v1.2.3\n",
    ] {
        assert!(tag_version(tag).is_err(), "{tag:?}");
        assert!(release_url(tag, None).is_err());
    }
}

#[test]
fn target_selection_requires_exact_filenames_and_tag() {
    let release = release("v0.5.0");
    for target in [Target::LinuxX64, Target::WindowsX64] {
        let selected = select_asset(&release, "v0.5.0", target).unwrap();
        assert_eq!(
            selected.url,
            release_url(
                "v0.5.0",
                Some(&target.asset_name(&Version::parse("0.5.0").unwrap()))
            )
            .unwrap()
        );
    }
    assert!(select_asset(&release, "v0.6.0", Target::LinuxX64).is_err());
    let mut wrong_target = release;
    wrong_target.assets[1].name = "jorik-cli-0.5.0-aarch64-pc-windows-msvc-setup.exe".to_string();
    assert!(select_asset(&wrong_target, "v0.5.0", Target::WindowsX64).is_err());
}

#[test]
fn ambiguous_or_missing_compatible_assets_are_rejected() {
    let mut release = release("v0.5.0");
    release.assets.push(release.assets[0].clone());
    assert!(select_asset(&release, "v0.5.0", Target::LinuxX64).is_err());
    release.assets.clear();
    assert!(select_asset(&release, "v0.5.0", Target::LinuxX64).is_err());
}

#[test]
fn unverified_empty_oversized_or_draft_assets_are_rejected() {
    for digest in [
        None,
        Some(""),
        Some("sha512:abcd"),
        Some("sha256:bad"),
        Some("sha256:gggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg"),
    ] {
        let mut release = release("v0.5.0");
        release.assets[0].digest = digest.map(str::to_string);
        assert!(select_asset(&release, "v0.5.0", Target::LinuxX64).is_err());
    }
    for size in [0, MAX_DOWNLOAD + 1] {
        let mut release = release("v0.5.0");
        release.assets[0].size = size;
        assert!(select_asset(&release, "v0.5.0", Target::LinuxX64).is_err());
    }
    let mut release = release("v0.5.0");
    release.draft = true;
    assert!(select_asset(&release, "v0.5.0", Target::LinuxX64).is_err());
}

#[test]
fn asset_urls_cannot_switch_repository_version_scheme_or_origin() {
    for url in [
        "http://github.com/fireflyteam/jorik-cli/releases/download/v0.5.0/jorik-x86_64-unknown-linux-gnu.tar.gz",
        "https://github.com/attacker/jorik-cli/releases/download/v0.5.0/jorik-x86_64-unknown-linux-gnu.tar.gz",
        "https://github.com/fireflyteam/jorik-cli/releases/download/v0.6.0/jorik-x86_64-unknown-linux-gnu.tar.gz",
        "https://github.com/fireflyteam/jorik-cli/releases/latest/download/jorik-x86_64-unknown-linux-gnu.tar.gz",
        "https://github.com.evil.example/fireflyteam/jorik-cli/releases/download/v0.5.0/jorik-x86_64-unknown-linux-gnu.tar.gz",
        "https://user:secret@github.com/fireflyteam/jorik-cli/releases/download/v0.5.0/jorik-x86_64-unknown-linux-gnu.tar.gz",
        "https://github.com/fireflyteam/jorik-cli/releases/download/v0.5.0/jorik-x86_64-unknown-linux-gnu.tar.gz?redirect=evil",
    ] {
        let mut release = release("v0.5.0");
        release.assets[0].browser_download_url = url.to_string();
        assert!(
            select_asset(&release, "v0.5.0", Target::LinuxX64).is_err(),
            "{url}"
        );
    }
}

#[test]
fn redirects_are_https_only_and_use_exact_github_hosts() {
    for url in [
        "https://api.github.com/",
        "https://github.com/",
        "https://release-assets.githubusercontent.com/asset?signature=x",
        "https://objects.githubusercontent.com/asset",
    ] {
        assert!(trusted_transport(&Url::parse(url).unwrap()));
    }
    for url in [
        "http://github.com/",
        "https://github.com.evil.example/",
        "https://evil.example/",
        "https://github.com:444/",
        "https://user:password@github.com/",
        "https://raw.githubusercontent.com/arbitrary/script",
    ] {
        assert!(!trusted_transport(&Url::parse(url).unwrap()), "{url}");
    }
}

async fn response(body: &[u8], status: &str, chunked: bool, declared: Option<usize>) -> Response {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let headers = if chunked {
        "Transfer-Encoding: chunked\r\n".to_string()
    } else {
        format!("Content-Length: {}\r\n", declared.unwrap_or(body.len()))
    };
    let mut wire = format!("HTTP/1.1 {status}\r\n{headers}Connection: close\r\n\r\n").into_bytes();
    if chunked {
        wire.extend_from_slice(format!("{:x}\r\n", body.len()).as_bytes());
        wire.extend_from_slice(body);
        wire.extend_from_slice(b"\r\n0\r\n\r\n");
    } else {
        wire.extend_from_slice(body);
    }
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 8192];
        assert!(stream.read(&mut request).await.unwrap() > 0);
        stream.write_all(&wire).await.unwrap();
    });
    Client::new().get(url).send().await.unwrap()
}

#[tokio::test]
async fn verified_download_returns_only_matching_bytes() {
    let release = release("v0.5.0");
    let asset = select_asset(&release, "v0.5.0", Target::LinuxX64).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut file = verified_response(
        response(b"test", "200 OK", false, None).await,
        &asset,
        dir.path(),
        ".tar.gz",
    )
    .await
    .unwrap();
    let mut actual = Vec::new();
    file.read_to_end(&mut actual).unwrap();
    assert_eq!(actual, b"test");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            file.as_file().metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[tokio::test]
async fn hash_size_truncation_and_http_failures_leave_no_staged_file() {
    let release = release("v0.5.0");
    let asset = select_asset(&release, "v0.5.0", Target::LinuxX64).unwrap();
    for (body, status, chunked, declared) in [
        (&b"evil"[..], "200 OK", false, None),
        (&b"toolong"[..], "200 OK", false, None),
        (&b"too short"[..], "200 OK", false, Some(100)),
        (&b"to"[..], "200 OK", false, Some(4)),
        (&b"toolong"[..], "200 OK", true, None),
        (&b"to"[..], "200 OK", true, None),
        (&b"test"[..], "404 Not Found", false, None),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let result = verified_response(
            response(body, status, chunked, declared).await,
            &asset,
            dir.path(),
            ".exe",
        )
        .await;
        assert!(result.is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}

#[tokio::test]
async fn metadata_is_bounded_even_without_content_length() {
    for chunked in [false, true] {
        assert!(
            bounded_body(
                response(b"too much metadata", "200 OK", chunked, None).await,
                8
            )
            .await
            .is_err()
        );
    }
    assert_eq!(
        bounded_body(response(b"{}", "200 OK", false, None).await, 8)
            .await
            .unwrap(),
        b"{}"
    );
}

fn elf() -> Vec<u8> {
    let mut bytes = vec![0; 64];
    bytes[..4].copy_from_slice(b"\x7fELF");
    bytes[4] = 2;
    bytes[5] = 1;
    bytes[16] = 3;
    bytes[18] = 62;
    bytes
}

fn archive(entries: &[(&str, tar::EntryType, Vec<u8>)]) -> Vec<u8> {
    let gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut archive = tar::Builder::new(gzip);
    for (path, kind, bytes) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(*kind);
        header.set_size(bytes.len() as u64);
        header.set_mode(0o755);
        // Deliberately permit malicious names in the test fixture.
        header.as_mut_bytes()[..100].fill(0);
        header.as_mut_bytes()[..path.len()].copy_from_slice(path.as_bytes());
        header.set_cksum();
        archive.append(&header, Cursor::new(bytes)).unwrap();
    }
    archive.into_inner().unwrap().finish().unwrap()
}

fn staged_archive(bytes: &[u8]) -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(bytes).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file
}

#[test]
fn linux_archive_copies_only_the_expected_regular_executable() {
    let bytes = elf();
    let mut source = staged_archive(&archive(&[(
        "jorik",
        tar::EntryType::Regular,
        bytes.clone(),
    )]));
    let mut destination = NamedTempFile::new().unwrap();
    extract_linux_binary(source.as_file_mut(), destination.as_file_mut()).unwrap();
    destination.seek(SeekFrom::Start(0)).unwrap();
    let mut actual = Vec::new();
    destination.read_to_end(&mut actual).unwrap();
    assert_eq!(actual, bytes);
}

#[test]
fn linux_archive_rejects_traversal_links_duplicates_and_wrong_binary_types() {
    for entries in [
        vec![("../outside", tar::EntryType::Regular, elf())],
        vec![("/absolute", tar::EntryType::Regular, elf())],
        vec![("jorik", tar::EntryType::Symlink, vec![])],
        vec![("jorik", tar::EntryType::Link, vec![])],
        vec![
            ("jorik", tar::EntryType::Regular, elf()),
            ("jorik", tar::EntryType::Regular, elf()),
        ],
        vec![
            ("jorik", tar::EntryType::Regular, elf()),
            ("other", tar::EntryType::Regular, elf()),
        ],
        vec![("jorik", tar::EntryType::Regular, vec![0; 64])],
        vec![],
    ] {
        let mut source = staged_archive(&archive(&entries));
        let mut destination = NamedTempFile::new().unwrap();
        assert!(extract_linux_binary(source.as_file_mut(), destination.as_file_mut()).is_err());
    }
    let mut wrong_arch = elf();
    wrong_arch[18] = 183; // AArch64, not x86_64.
    let mut source = staged_archive(&archive(&[("jorik", tar::EntryType::Regular, wrong_arch)]));
    let mut destination = NamedTempFile::new().unwrap();
    assert!(extract_linux_binary(source.as_file_mut(), destination.as_file_mut()).is_err());
}

#[test]
fn truncated_or_oversized_compressed_archives_are_rejected() {
    let mut bytes = archive(&[("jorik", tar::EntryType::Regular, elf())]);
    bytes.truncate(bytes.len() - 8); // Entire tar is present; gzip trailer is not.
    let mut source = staged_archive(&bytes);
    let mut destination = NamedTempFile::new().unwrap();
    assert!(extract_linux_binary(source.as_file_mut(), destination.as_file_mut()).is_err());

    let mut header = tar::Header::new_gnu();
    header.set_path("jorik").unwrap();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_size(MAX_BINARY + 1);
    header.set_cksum();
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(header.as_bytes()).unwrap();
    let mut source = staged_archive(&gzip.finish().unwrap());
    assert!(extract_linux_binary(source.as_file_mut(), destination.as_file_mut()).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn linux_replacement_is_atomic_and_failed_validation_preserves_original() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let installed = dir.path().join("jorik");
    std::fs::write(&installed, b"original executable").unwrap();
    let mut bad = staged_archive(&archive(&[("jorik", tar::EntryType::Regular, vec![0; 64])]));
    assert!(install_linux(bad.as_file_mut(), &installed).is_err());
    assert_eq!(std::fs::read(&installed).unwrap(), b"original executable");
    let bytes = elf();
    let mut good = staged_archive(&archive(&[(
        "jorik",
        tar::EntryType::Regular,
        bytes.clone(),
    )]));
    install_linux(good.as_file_mut(), &installed).unwrap();
    assert_eq!(std::fs::read(&installed).unwrap(), bytes);
    assert_eq!(
        std::fs::metadata(&installed).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_installation_does_not_follow_a_destination_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("original");
    std::fs::write(&original, b"unchanged").unwrap();
    let installed = dir.path().join("jorik");
    std::os::unix::fs::symlink(&original, &installed).unwrap();
    let mut source = staged_archive(&archive(&[("jorik", tar::EntryType::Regular, elf())]));
    assert!(install_linux(source.as_file_mut(), &installed).is_err());
    assert_eq!(std::fs::read(original).unwrap(), b"unchanged");
}

#[test]
fn windows_installer_requires_a_valid_pe_header() {
    for machine in [0x14cu16, 0x8664] {
        let mut pe = vec![0; 64];
        pe[..2].copy_from_slice(b"MZ");
        pe[60] = 64;
        pe.extend_from_slice(b"PE\0\0");
        pe.extend_from_slice(&machine.to_le_bytes());
        let mut file = staged_archive(&pe);
        validate_windows_installer(file.as_file_mut()).unwrap();
    }
    for bytes in [b"shell script".to_vec(), vec![0; 100]] {
        let mut file = staged_archive(&bytes);
        assert!(validate_windows_installer(file.as_file_mut()).is_err());
    }
}

#[test]
fn missing_and_null_digests_deserialize_but_never_pass_verification() {
    for json in [
        r#"{"name":"asset","browser_download_url":"https://github.com/asset","size":10}"#,
        r#"{"name":"asset","browser_download_url":"https://github.com/asset","size":10,"digest":null}"#,
    ] {
        let asset: ReleaseAsset = serde_json::from_str(json).unwrap();
        assert!(digest_bytes(asset.digest.as_deref()).is_err());
    }
}

#[tokio::test]
async fn production_client_refuses_plain_http_before_connecting() {
    let error = client()
        .unwrap()
        .get("http://127.0.0.1:9/")
        .send()
        .await
        .unwrap_err();
    assert!(error.is_builder());
}

#[tokio::test]
#[ignore = "network: verifies a published release download, without running or installing it"]
async fn published_release_verifies_without_installing() {
    let client = client().unwrap();
    let tag = "v0.4.1";
    let release: Release = metadata(
        &client,
        Url::parse(&format!("{RELEASES_API}/tags/{tag}")).unwrap(),
        Duration::from_secs(15),
    )
    .await
    .unwrap();
    let asset = select_asset(&release, tag, Target::LinuxX64).unwrap();
    let response = client
        .get(asset.url.clone())
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .send()
        .await
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let mut download = verified_response(response, &asset, directory.path(), ".tar.gz")
        .await
        .unwrap();
    let mut binary = NamedTempFile::new_in(directory.path()).unwrap();
    extract_linux_binary(download.as_file_mut(), binary.as_file_mut()).unwrap();
    assert!(binary.as_file().metadata().unwrap().len() > 64);
}

#[test]
fn windows_launch_uses_direct_arguments_and_the_existing_install_directory() {
    let installer = Path::new("private-stage/verified.exe");
    let destination = Path::new("Program Files/Jorik/jorik.exe");
    let command = windows_command(installer, destination).unwrap();
    assert_eq!(command.get_program(), installer.as_os_str());
    let args = command.get_args().collect::<Vec<_>>();
    assert_eq!(args.len(), 4);
    assert_eq!(args[0], "/SILENT");
    assert_eq!(args[1], "/NORESTART");
    assert_eq!(args[2], "/SP-");
    assert_eq!(args[3], "/DIR=Program Files/Jorik");
    assert_eq!(command.get_current_dir(), installer.parent());
    assert!(windows_command(installer, Path::new("target/debug/jorik-cli.exe")).is_err());
}
