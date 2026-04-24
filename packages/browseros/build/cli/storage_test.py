#!/usr/bin/env python3
"""Tests for the Lima R2 uploader CLI."""

import hashlib
import io
import tarfile
import tempfile
import unittest
from pathlib import Path
from typing import Any, List, Tuple
from unittest import mock

from build.cli import storage


def _build_lima_tarball(version: str, payload: bytes) -> bytes:
    """Return a gzipped tar containing `lima-<v>/bin/limactl` with `payload`."""
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w:gz") as tar:
        info = tarfile.TarInfo(name=f"lima-{version}/bin/limactl")
        info.size = len(payload)
        info.mode = 0o755
        tar.addfile(info, io.BytesIO(payload))
    return buffer.getvalue()


class ParseChecksumsTest(unittest.TestCase):
    def test_parses_two_column_lines(self) -> None:
        contents = (
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  lima-1.2.3-Darwin-arm64.tar.gz\n"
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb *lima-1.2.3-Darwin-x86_64.tar.gz\n"
        )
        entries = storage._parse_checksums(contents)
        self.assertEqual(
            entries["lima-1.2.3-Darwin-arm64.tar.gz"],
            "a" * 64,
        )
        self.assertEqual(
            entries["lima-1.2.3-Darwin-x86_64.tar.gz"],
            "b" * 64,
        )

    def test_ignores_blank_lines(self) -> None:
        contents = "\n\n" + "c" * 64 + "  lima-1.0.0-Darwin-arm64.tar.gz\n\n"
        entries = storage._parse_checksums(contents)
        self.assertEqual(list(entries), ["lima-1.0.0-Darwin-arm64.tar.gz"])

    def test_rejects_malformed_lines(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "Malformed"):
            storage._parse_checksums("just-one-token\n")

    def test_rejects_non_sha256(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "Invalid sha256"):
            storage._parse_checksums("xyz foo.tar.gz\n")


class NormalizeVersionTagTest(unittest.TestCase):
    def test_keeps_existing_v_prefix(self) -> None:
        self.assertEqual(storage._normalize_version_tag("v1.2.3"), "v1.2.3")

    def test_adds_v_prefix_when_missing(self) -> None:
        self.assertEqual(storage._normalize_version_tag("1.2.3"), "v1.2.3")


class ExtractLimactlTest(unittest.TestCase):
    def test_extracts_limactl_binary(self) -> None:
        payload = b"limactl-bytes-" + b"x" * 100
        tarball = _build_lima_tarball("1.2.3", payload)

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            tarball_path = tmp_path / "lima.tar.gz"
            tarball_path.write_bytes(tarball)
            dest = tmp_path / "limactl"

            storage._extract_limactl(tarball_path, dest)

            self.assertEqual(dest.read_bytes(), payload)
            self.assertTrue(dest.stat().st_mode & 0o100, "should be executable")

    def test_raises_when_limactl_missing(self) -> None:
        buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode="w:gz") as tar:
            info = tarfile.TarInfo(name="lima-1.2.3/README")
            info.size = 5
            tar.addfile(info, io.BytesIO(b"hello"))

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            tarball_path = tmp_path / "lima.tar.gz"
            tarball_path.write_bytes(buffer.getvalue())

            with self.assertRaisesRegex(RuntimeError, "bin/limactl not found"):
                storage._extract_limactl(tarball_path, tmp_path / "out")


class RollbackTest(unittest.TestCase):
    def test_rollback_deletes_all_keys(self) -> None:
        deleted: List[Tuple[str, str]] = []

        class FakeClient:
            def delete_object(self, **kwargs: str) -> None:
                deleted.append((kwargs["Bucket"], kwargs["Key"]))

        storage._rollback(FakeClient(), "browseros", ["a", "b", "c"])
        self.assertEqual(deleted, [("browseros", "a"), ("browseros", "b"), ("browseros", "c")])

    def test_rollback_tolerates_delete_failures(self) -> None:
        class FakeClient:
            def delete_object(self, **kwargs: str) -> None:
                raise RuntimeError("boom")

        # Should not raise — it logs a warning and moves on.
        storage._rollback(FakeClient(), "browseros", ["a"])


class BuildManifestTest(unittest.TestCase):
    def test_manifest_shape(self) -> None:
        manifest = storage._build_manifest(
            "v1.2.3",
            {"arm64": "a" * 64, "x64": "b" * 64},
            {"arm64": "c" * 64, "x64": "d" * 64},
        )
        self.assertEqual(manifest["lima_version"], "v1.2.3")
        self.assertEqual(manifest["tarball_shas_upstream"]["arm64"], "a" * 64)
        self.assertEqual(manifest["r2_object_shas"]["x64"], "d" * 64)
        self.assertIn("uploaded_at", manifest)
        self.assertIn("uploaded_by", manifest)


class ProcessArchTest(unittest.TestCase):
    """Covers download + sha verify + extract + upload in one pass."""

    def setUp(self) -> None:
        self.payload = b"limactl-binary-" + b"z" * 200
        self.tarball_bytes = _build_lima_tarball("1.2.3", self.payload)
        self.expected_tarball_sha = hashlib.sha256(self.tarball_bytes).hexdigest()
        self.expected_object_sha = hashlib.sha256(self.payload).hexdigest()

    def _fake_download(self, _url: str, dest: Path, **_kwargs: Any) -> None:
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(self.tarball_bytes)

    def test_happy_path_uploads_and_returns_shas(self) -> None:
        uploads: List[Tuple[str, str]] = []

        def fake_upload(_client: Any, _local_path: Path, r2_key: str, bucket: str) -> bool:
            uploads.append((r2_key, bucket))
            return True

        env = mock.Mock(r2_bucket="browseros")

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            with mock.patch.object(storage, "_download", side_effect=self._fake_download), \
                 mock.patch.object(storage, "upload_file_to_r2", side_effect=fake_upload):
                tarball_sha, object_sha, r2_key = storage._process_arch(
                    tag="v1.2.3",
                    arch=storage.LimaArch(internal="arm64", upstream="Darwin-arm64"),
                    tmp_dir=tmp_path,
                    checksums={
                        "lima-1.2.3-Darwin-arm64.tar.gz": self.expected_tarball_sha
                    },
                    client=mock.Mock(),
                    env=env,
                    dry_run=False,
                )

        self.assertEqual(tarball_sha, self.expected_tarball_sha)
        self.assertEqual(object_sha, self.expected_object_sha)
        self.assertEqual(
            r2_key, "artifacts/vendor/third_party/lima/limactl-darwin-arm64"
        )
        self.assertEqual(
            uploads,
            [
                (
                    "artifacts/vendor/third_party/lima/limactl-darwin-arm64",
                    "browseros",
                )
            ],
        )

    def test_sha_mismatch_aborts_before_upload(self) -> None:
        uploads: List[Tuple[str, str]] = []

        def fake_upload(_client: Any, _local_path: Path, r2_key: str, bucket: str) -> bool:
            uploads.append((r2_key, bucket))
            return True

        env = mock.Mock(r2_bucket="browseros")

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            with mock.patch.object(storage, "_download", side_effect=self._fake_download), \
                 mock.patch.object(storage, "upload_file_to_r2", side_effect=fake_upload):
                with self.assertRaisesRegex(RuntimeError, "sha256 mismatch"):
                    storage._process_arch(
                        tag="v1.2.3",
                        arch=storage.LimaArch(internal="arm64", upstream="Darwin-arm64"),
                        tmp_dir=tmp_path,
                        checksums={"lima-1.2.3-Darwin-arm64.tar.gz": "0" * 64},
                        client=mock.Mock(),
                        env=env,
                        dry_run=False,
                    )

        self.assertEqual(uploads, [])

    def test_missing_checksum_entry_aborts(self) -> None:
        env = mock.Mock(r2_bucket="browseros")

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            with self.assertRaisesRegex(RuntimeError, "missing from SHA256SUMS"):
                storage._process_arch(
                    tag="v1.2.3",
                    arch=storage.LimaArch(internal="arm64", upstream="Darwin-arm64"),
                    tmp_dir=tmp_path,
                    checksums={},
                    client=mock.Mock(),
                    env=env,
                    dry_run=False,
                )

    def test_dry_run_skips_upload(self) -> None:
        uploads: List[Tuple[str, str]] = []

        def fake_upload(*args: Any, **kwargs: Any) -> bool:
            uploads.append(("called", ""))
            return True

        env = mock.Mock(r2_bucket="browseros")

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            with mock.patch.object(storage, "_download", side_effect=self._fake_download), \
                 mock.patch.object(storage, "upload_file_to_r2", side_effect=fake_upload):
                _, _, r2_key = storage._process_arch(
                    tag="v1.2.3",
                    arch=storage.LimaArch(internal="arm64", upstream="Darwin-arm64"),
                    tmp_dir=tmp_path,
                    checksums={
                        "lima-1.2.3-Darwin-arm64.tar.gz": self.expected_tarball_sha
                    },
                    client=None,
                    env=env,
                    dry_run=True,
                )

        self.assertEqual(uploads, [])
        self.assertEqual(
            r2_key, "artifacts/vendor/third_party/lima/limactl-darwin-arm64"
        )


if __name__ == "__main__":
    unittest.main()
