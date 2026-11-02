import gzip
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location("bootstrap", Path(__file__).parents[1] / "bootstrap.py")
b = importlib.util.module_from_spec(spec)
spec.loader.exec_module(b)


def elf():
    data = bytearray(64)
    data[:6] = b"\x7fELF\x02\x01"
    data[16] = 3
    data[18] = 62
    return bytes(data)


def archive(entries=None):
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w:gz") as tar:
        for name, kind, data in entries or [("jorik", tarfile.REGTYPE, elf())]:
            member = tarfile.TarInfo(name)
            member.type = kind
            member.size = len(data)
            tar.addfile(member, io.BytesIO(data))
    return output.getvalue()


def release(data):
    return {"tag_name": "v0.5.0", "draft": False, "prerelease": False, "assets": [{"name": b.ASSET,
        "browser_download_url": f"https://github.com/{b.REPO}/releases/download/v0.5.0/{b.ASSET}",
        "size": len(data), "digest": "sha256:" + hashlib.sha256(data).hexdigest()}]}


class BootstrapTests(unittest.TestCase):
    def test_version_policy_and_origin_pinning(self):
        data = archive()
        metadata = release(data)
        self.assertEqual(b.select_asset(metadata)["size"], len(data))
        for tag in ["../latest", "latest", "v01.2.3", "v1.2.3-a..b", "v1.2.3?x", "vv1.2.3"]:
            with self.assertRaises(RuntimeError): b.version(tag)
        self.assertGreater(b.compare("v0.5.0", "v0.5.0-rc.1"), 0)
        self.assertGreater(b.compare("v0.5.0-rc.10", "v0.5.0-rc.2"), 0)
        for change in [{"digest": None}, {"size": b.MAX_DOWNLOAD + 1}, {"browser_download_url": "https://evil.example/asset"}]:
            bad = release(data)
            bad["assets"][0].update(change)
            with self.assertRaises(RuntimeError): b.select_asset(bad)

    def test_provenance_policy_pins_tag_workflow_and_issuer(self):
        args = b.attestation_args(Path("/private/asset"), "v0.5.0")
        for value in ["--deny-self-hosted-runners", "refs/tags/v0.5.0", "https://token.actions.githubusercontent.com", "https://slsa.dev/provenance/v1",
            "https://github.com/fireflyteam/jorik-cli/.github/workflows/build.yml@refs/tags/v0.5.0"]:
            self.assertIn(value, args)

    def test_success_installs_atomically_only_after_hash_and_provenance(self):
        for fail_signature in [False, True]:
            data = archive()
            with tempfile.TemporaryDirectory() as tmp:
                dest = Path(tmp) / "jorik"
                dest.write_bytes(b"old")
                def gh(args, timeout=90):
                    if args[0] == "api": return json.dumps([release(data)]).encode()
                    if args[0] == "release":
                        (Path(args[args.index("--dir") + 1]) / b.ASSET).write_bytes(data)
                        return b""
                    self.assertEqual(args[:2], ["attestation", "verify"])
                    if fail_signature: raise RuntimeError("invalid signature")
                    return b""
                with mock.patch.dict(os.environ, {}, clear=True), mock.patch.object(b, "gh", side_effect=gh), mock.patch.object(b.shutil, "which", return_value="/usr/bin/gh"), mock.patch.object(b.platform, "system", return_value="Linux"), mock.patch.object(b.platform, "machine", return_value="x86_64"), mock.patch.object(b.platform, "libc_ver", return_value=("glibc", "2.39")):
                    if fail_signature:
                        with self.assertRaises(RuntimeError): b.main(["--dest", str(dest)])
                        self.assertEqual(dest.read_bytes(), b"old")
                    else:
                        b.main(["--dest", str(dest)])
                        self.assertEqual(dest.read_bytes(), elf())
                        self.assertEqual(dest.stat().st_mode & 0o777, 0o755)
                self.assertEqual([p.name for p in Path(tmp).iterdir()], ["jorik"])

    def test_unsafe_archives_and_truncated_gzip_preserve_existing_binary(self):
        variants = [archive([(name, kind, data)]) for name, kind, data in [
            ("../escape", tarfile.REGTYPE, elf()), ("/absolute", tarfile.REGTYPE, elf()),
            ("jorik", tarfile.SYMTYPE, b""), ("jorik", tarfile.LNKTYPE, b""),
            ("jorik", tarfile.REGTYPE, bytes(64)),
        ]]
        variants += [archive()[:-8], archive([("jorik", tarfile.REGTYPE, elf()), ("other", tarfile.REGTYPE, elf())])]
        for data in variants:
            with tempfile.TemporaryDirectory() as tmp:
                package, dest = Path(tmp) / "archive", Path(tmp) / "jorik"
                package.write_bytes(data)
                dest.write_bytes(b"old")
                with self.assertRaises(Exception): b.install(package, dest)
                self.assertEqual(dest.read_bytes(), b"old")
                self.assertEqual(len(list(Path(tmp).iterdir())), 2)

    def test_digest_mismatch_and_stream_limits_fail_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "archive"
            path.write_bytes(b"evil")
            metadata = release(b"good")["assets"][0]
            with self.assertRaises(RuntimeError): b.verify_bytes(path, metadata)
        limited = b.LimitedReader(io.BytesIO(b"x" * 32), 8)
        with self.assertRaises(RuntimeError): limited.read(1000)


if __name__ == "__main__": unittest.main()
