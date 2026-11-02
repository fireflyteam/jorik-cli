#!/usr/bin/env python3
"""Verified Linux bootstrap. Run from a trusted checkout; never downloads code to execute."""
import argparse
import functools
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile

REPO = "fireflyteam/jorik-cli"
ASSET = "jorik-x86_64-unknown-linux-gnu.tar.gz"
MAX_DOWNLOAD = 128 * 1024 * 1024
MAX_BINARY = 256 * 1024 * 1024
TAG = re.compile(r"v?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z.-]+))?(?:\+([0-9A-Za-z.-]+))?\Z", re.ASCII)


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def version(tag):
    require(isinstance(tag, str) and len(tag) <= 128, "Invalid release tag")
    match = TAG.fullmatch(tag)
    require(match is not None, "Invalid release tag")
    major, minor, patch, pre, build = match.groups()
    require(build is None or all(build.split(".")), "Invalid build metadata")
    identifiers = pre.split(".") if pre is not None else []
    require(all(item and not (item.isdigit() and len(item) > 1 and item.startswith("0")) for item in identifiers), "Invalid prerelease tag")
    return (int(major), int(minor), int(patch)), identifiers


def compare(a, b):
    av, ap = version(a)
    bv, bp = version(b)
    if av != bv:
        return (av > bv) - (av < bv)
    if not ap or not bp:
        return (not ap) - (not bp)
    for left, right in zip(ap, bp):
        if left == right:
            continue
        if left.isdigit() and right.isdigit():
            return (int(left) > int(right)) - (int(left) < int(right))
        if left.isdigit() != right.isdigit():
            return -1 if left.isdigit() else 1
        return (left > right) - (left < right)
    return (len(ap) > len(bp)) - (len(ap) < len(bp))


def gh(args, timeout=90):
    env = dict(os.environ, GH_HOST="github.com", GH_PROMPT_DISABLED="1")
    for name in ("JORIK_TOKEN", "GH_DEBUG"):
        env.pop(name, None)
    try:
        result = subprocess.run(["gh", *args], env=env, stdin=subprocess.DEVNULL,
                                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=timeout, check=False)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise RuntimeError("A current, authenticated GitHub CLI (gh) is required; verification cannot be bypassed") from error
    require(result.returncode == 0, "GitHub download or signed provenance verification failed; no unverified fallback is allowed")
    require(len(result.stdout) <= 4 * 1024 * 1024, "GitHub response exceeds the size limit")
    return result.stdout


def select_release(metadata, requested=None, prerelease=False):
    if requested is not None:
        version(requested)
        require(isinstance(metadata, dict) and metadata.get("tag_name") == requested and not metadata.get("draft"), "Release does not match the requested tag")
        return metadata
    candidates = metadata if isinstance(metadata, list) else [metadata]
    candidates = [release for release in candidates if isinstance(release, dict) and not release.get("draft")]
    valid = []
    for release in candidates:
        try:
            _v, pre = version(release.get("tag_name"))
        except RuntimeError:
            continue
        if prerelease or (not pre and not release.get("prerelease")):
            valid.append(release)
    require(valid, "No compatible release found")
    return max(valid, key=functools.cmp_to_key(lambda a, b: compare(a["tag_name"], b["tag_name"])))


def select_asset(release):
    tag = release["tag_name"]
    version(tag)
    matches = [asset for asset in release.get("assets", []) if asset.get("name") == ASSET]
    require(len(matches) == 1, "Release must contain exactly one matching Linux asset")
    asset = matches[0]
    expected = f"https://github.com/{REPO}/releases/download/{tag}/{ASSET}"
    require(asset.get("browser_download_url") == expected, "Asset URL is not pinned to the trusted repository and tag")
    require(type(asset.get("size")) is int and 0 < asset["size"] <= MAX_DOWNLOAD, "Invalid release asset size")
    require(isinstance(asset.get("digest"), str) and re.fullmatch(r"sha256:[0-9a-fA-F]{64}", asset["digest"]), "A GitHub SHA-256 digest is required")
    return asset


def attestation_args(artifact, tag):
    version(tag)
    return ["attestation", "verify", str(artifact), "--hostname", "github.com", "--repo", REPO,
            "--cert-identity", f"https://github.com/{REPO}/.github/workflows/build.yml@refs/tags/{tag}",
            "--source-ref", f"refs/tags/{tag}", "--cert-oidc-issuer", "https://token.actions.githubusercontent.com",
            "--predicate-type", "https://slsa.dev/provenance/v1", "--deny-self-hosted-runners", "--digest-alg", "sha256", "--limit", "5"]


def verify_bytes(path, asset):
    require(path.is_file() and not path.is_symlink() and path.stat().st_size == asset["size"], "Downloaded asset size/type mismatch")
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(65536), b""):
            digest.update(block)
    require("sha256:" + digest.hexdigest() == asset["digest"].lower(), "SHA-256 mismatch")


class LimitedReader(io.RawIOBase):
    def __init__(self, source, maximum):
        self.source, self.remaining = source, maximum

    def read(self, size=-1):
        if size < 0 or size > self.remaining + 1:
            size = self.remaining + 1
        data = self.source.read(size)
        self.remaining -= len(data)
        require(self.remaining >= 0, "Uncompressed archive exceeds the size limit")
        return data


def extract(archive_path, output):
    found = False
    with gzip.open(archive_path, "rb") as gzip_stream:
        limited = LimitedReader(gzip_stream, MAX_BINARY + 1024 * 1024)
        with tarfile.open(fileobj=limited, mode="r|") as archive:
            for member in archive:
                require(not found and member.name == "jorik" and member.type in (tarfile.REGTYPE, tarfile.AREGTYPE), "Archive must contain one regular file named jorik")
                require(64 <= member.size <= MAX_BINARY, "Invalid executable size")
                source = archive.extractfile(member)
                require(source is not None, "Missing executable bytes")
                copied = 0
                while block := source.read(65536):
                    copied += len(block)
                    require(copied <= member.size, "Executable exceeds its declared size")
                    output.write(block)
                require(copied == member.size, "Truncated executable")
                found = True
        while limited.read(65536):  # Consume and validate the gzip trailer too.
            pass
    require(found, "Archive has no executable")
    output.seek(0)
    header = output.read(20)
    require(header[:6] == b"\x7fELF\x02\x01" and int.from_bytes(header[16:18], "little") in (2, 3)
            and int.from_bytes(header[18:20], "little") == 62, "Not a Linux x86_64 ELF executable")


def install(archive_path, destination):
    require(destination.is_absolute(), "Installation destination must be absolute")
    require(not destination.is_symlink() and (not destination.exists() or destination.is_file()), "Destination must be a regular file, not a symlink")
    destination.parent.mkdir(mode=0o755, parents=True, exist_ok=True)
    staged = None
    try:
        with tempfile.NamedTemporaryFile(prefix=".jorik-install-", dir=destination.parent, mode="w+b", delete=False) as output:
            staged = Path(output.name)
            extract(archive_path, output)
            output.flush()
            os.fchmod(output.fileno(), 0o755)
            os.fsync(output.fileno())
        os.replace(staged, destination)
        staged = None
        try:
            fd = os.open(destination.parent, os.O_RDONLY | os.O_DIRECTORY)
            try: os.fsync(fd)
            finally: os.close(fd)
        except OSError:
            print("Warning: installed, but directory sync failed", file=sys.stderr)
    finally:
        if staged is not None:
            staged.unlink(missing_ok=True)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pre", action="store_true", help="explicitly include prereleases")
    parser.add_argument("--tag", default=os.environ.get("TAG") or None)
    parser.add_argument("--dest", default=os.environ.get("DEST") or str(Path.home() / ".local/bin/jorik"))
    args = parser.parse_args(argv)
    require(platform.system() == "Linux" and platform.machine().lower() in ("x86_64", "amd64") and platform.libc_ver()[0] == "glibc", "Only Linux x86_64 GNU is supported by this bootstrap")
    for key, expected in [("HOST", "https://api.github.com"), ("OWNER", "fireflyteam"), ("REPO", "jorik-cli"), ("TARGET", "x86_64-unknown-linux-gnu")]:
        require(os.environ.get(key, expected) == expected, f"{key} cannot override the trusted publisher or platform")
    require(shutil.which("gh") is not None, "Install a current GitHub CLI (gh) before running this installer")
    if args.tag:
        version(args.tag)
    endpoint = f"repos/{REPO}/releases/tags/{args.tag}" if args.tag else f"repos/{REPO}/releases?per_page=100"
    release = select_release(json.loads(gh(["api", "--hostname", "github.com", endpoint], timeout=30)), args.tag, args.pre)
    asset = select_asset(release)
    tag = release["tag_name"]
    with tempfile.TemporaryDirectory(prefix="jorik-verified-install-") as tmp:
        archive = Path(tmp) / ASSET
        gh(["release", "download", tag, "--repo", REPO, "--pattern", ASSET, "--dir", tmp], timeout=180)
        verify_bytes(archive, asset)
        gh(attestation_args(archive, tag))
        verify_bytes(archive, asset)
        destination = Path(args.dest).expanduser().absolute()
        install(archive, destination)
        print(f"Installed verified {tag} to {str(destination)!r}. No sudo or downloaded script was executed.")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"Installation aborted: {error}", file=sys.stderr)
        sys.exit(1)
