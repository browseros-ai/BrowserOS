#!/usr/bin/env python3
"""Tests for the Lima R2 uploader CLI."""

import hashlib
import io
import json
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path
from typing import Any, Dict, List, Tuple
from unittest import mock

from build.cli import storage


def _build_lima_tarball(
    version: str,
    limactl_payload: bytes,
    guest_agents: Dict[str, bytes] | None = None,
) -> bytes:
    """Return a gzipped Lima release tarball with selected runtime files."""
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w:gz") as tar:
        _add_tar_file(
            tar,
            f"lima-{version}/bin/limactl",
            limactl_payload,
            mode=0o755,
        )
        for guest_arch, payload in (guest_agents or {}).items():
            _add_tar_file(
                tar,
                f"lima-{version}/share/lima/lima-guestagent.Linux-{guest_arch}.gz",
                payload,
                mode=0o644,
            )
    return buffer.getvalue()


def _add_tar_file(
    tar: tarfile.TarFile,
    name: str,
    payload: bytes,
    *,
    mode: int = 0o644,
) -> None:
    info = tarfile.TarInfo(name=name)
    info.size = len(payload)
    info.mode = mode
    tar.addfile(info, io.BytesIO(payload))


def _build_bun_zip(
    payload: bytes,
    *,
    root_dir: str = "bun-darwin-aarch64",
    binary_name: str = "bun",
) -> bytes:
    """Return a Bun release zip with the upstream directory layout."""
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr(f"{root_dir}/{binary_name}", payload)
    return buffer.getvalue()


def _build_codex_package(
    entrypoint: str,
    payload: bytes,
    actual_entrypoint: str | None = None,
) -> bytes:
    """Return a Codex package archive with package metadata and entrypoint."""
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w:gz") as tar:
        _add_tar_file(
            tar,
            "codex-package.json",
            json.dumps({"entrypoint": entrypoint}).encode("utf-8"),
        )
        _add_tar_file(tar, actual_entrypoint or entrypoint, payload, mode=0o755)
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


class NormalizeBunVersionTagTest(unittest.TestCase):
    def test_keeps_existing_bun_prefix(self) -> None:
        self.assertEqual(
            storage._normalize_bun_version_tag("bun-v1.2.3"),
            "bun-v1.2.3",
        )

    def test_accepts_plain_semver(self) -> None:
        self.assertEqual(storage._normalize_bun_version_tag("1.2.3"), "bun-v1.2.3")

    def test_accepts_v_prefixed_semver(self) -> None:
        self.assertEqual(storage._normalize_bun_version_tag("v1.2.3"), "bun-v1.2.3")

    def test_accepts_bun_prefixed_semver_without_v(self) -> None:
        self.assertEqual(
            storage._normalize_bun_version_tag("bun-1.2.3"),
            "bun-v1.2.3",
        )


class ExtractLimaFileTest(unittest.TestCase):
    def test_extracts_limactl_binary(self) -> None:
        payload = b"limactl-bytes-" + b"x" * 100
        tarball = _build_lima_tarball("1.2.3", payload)

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            tarball_path = tmp_path / "lima.tar.gz"
            tarball_path.write_bytes(tarball)
            dest = tmp_path / "limactl"

            storage._extract_lima_file(tarball_path, "bin/limactl", dest)

            self.assertEqual(dest.read_bytes(), payload)
            self.assertTrue(dest.stat().st_mode & 0o100, "should be executable")

    def test_extracts_native_guest_agent(self) -> None:
        payload = b"guest-agent-bytes-" + b"g" * 100
        tarball = _build_lima_tarball(
            "1.2.3",
            b"limactl",
            guest_agents={"aarch64": payload},
        )

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            tarball_path = tmp_path / "lima.tar.gz"
            tarball_path.write_bytes(tarball)
            dest = tmp_path / "lima-guestagent.Linux-aarch64.gz"

            storage._extract_lima_file(
                tarball_path,
                "share/lima/lima-guestagent.Linux-aarch64.gz",
                dest,
            )

            self.assertEqual(dest.read_bytes(), payload)
            self.assertFalse(dest.stat().st_mode & 0o100, "should not be executable")

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
                storage._extract_lima_file(
                    tarball_path, "bin/limactl", tmp_path / "out"
                )

    def test_raises_when_guest_agent_missing(self) -> None:
        tarball = _build_lima_tarball("1.2.3", b"limactl")

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            tarball_path = tmp_path / "lima.tar.gz"
            tarball_path.write_bytes(tarball)

            with self.assertRaisesRegex(
                RuntimeError,
                "share/lima/lima-guestagent.Linux-aarch64.gz not found",
            ):
                storage._extract_lima_file(
                    tarball_path,
                    "share/lima/lima-guestagent.Linux-aarch64.gz",
                    tmp_path / "guest-agent",
                )


class ExtractBunFileTest(unittest.TestCase):
    def test_extracts_bun_binary(self) -> None:
        payload = b"bun-binary-" + b"b" * 100

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            zip_path = tmp_path / "bun.zip"
            zip_path.write_bytes(_build_bun_zip(payload))
            dest = tmp_path / "bun"

            storage._extract_bun_file(zip_path, dest)

            self.assertEqual(dest.read_bytes(), payload)
            self.assertTrue(dest.stat().st_mode & 0o100, "should be executable")

    def test_raises_when_bun_missing(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            zip_path = tmp_path / "bun.zip"
            with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_DEFLATED) as archive:
                archive.writestr("bun-darwin-aarch64/README.md", b"missing")

            with self.assertRaisesRegex(RuntimeError, "bun not found in Bun zip"):
                storage._extract_bun_file(zip_path, tmp_path / "bun")

    def test_raises_with_requested_binary_name_when_missing(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            zip_path = tmp_path / "bun.zip"
            with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_DEFLATED) as archive:
                archive.writestr("bun-windows-x64-baseline/README.md", b"missing")

            with self.assertRaisesRegex(RuntimeError, r"bun\.exe not found"):
                storage._extract_bun_file(
                    zip_path,
                    tmp_path / "bun.exe",
                    binary_name="bun.exe",
                )

    def test_windows_exe_is_not_marked_executable(self) -> None:
        payload = b"bun-windows-exe-" + b"b" * 100

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            zip_path = tmp_path / "bun.zip"
            zip_path.write_bytes(
                _build_bun_zip(
                    payload,
                    root_dir="bun-windows-x64-baseline",
                    binary_name="bun.exe",
                )
            )
            dest = tmp_path / "bun.exe"

            storage._extract_bun_file(zip_path, dest, binary_name="bun.exe")

            self.assertEqual(dest.read_bytes(), payload)
            self.assertFalse(dest.stat().st_mode & 0o100, "should not be executable")


class BunTargetTest(unittest.TestCase):
    def test_bun_targets_have_expected_fields(self) -> None:
        self.assertEqual(
            [
                (target.internal, target.upstream, target.r2_name, target.binary_name)
                for target in storage.BUN_TARGETS
            ],
            [
                ("darwin-arm64", "darwin-aarch64", "bun-darwin-arm64", "bun"),
                ("darwin-x64", "darwin-x64", "bun-darwin-x64", "bun"),
                ("linux-arm64", "linux-aarch64", "bun-linux-arm64", "bun"),
                (
                    "linux-x64",
                    "linux-x64-baseline",
                    "bun-linux-x64-baseline",
                    "bun",
                ),
                (
                    "windows-x64",
                    "windows-x64-baseline",
                    "bun-windows-x64-baseline.exe",
                    "bun.exe",
                ),
            ],
        )


class RollbackTest(unittest.TestCase):
    def test_rollback_deletes_all_keys(self) -> None:
        deleted: List[Tuple[str, str]] = []

        class FakeClient:
            def delete_object(self, **kwargs: str) -> None:
                deleted.append((kwargs["Bucket"], kwargs["Key"]))

        storage._rollback(FakeClient(), "browseros", ["a", "b", "c"])
        self.assertEqual(
            deleted, [("browseros", "a"), ("browseros", "b"), ("browseros", "c")]
        )

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
            {
                "arm64": {"limactl": "c" * 64, "guest_agent": "d" * 64},
                "x64": {"limactl": "e" * 64, "guest_agent": "f" * 64},
            },
        )
        self.assertEqual(manifest["lima_version"], "v1.2.3")
        self.assertEqual(manifest["tarball_shas_upstream"]["arm64"], "a" * 64)
        self.assertEqual(manifest["r2_object_shas"]["x64"]["limactl"], "e" * 64)
        self.assertEqual(manifest["r2_object_shas"]["x64"]["guest_agent"], "f" * 64)
        self.assertIn("uploaded_at", manifest)
        self.assertIn("uploaded_by", manifest)

    def test_bun_manifest_shape(self) -> None:
        manifest = storage._build_bun_manifest(
            "bun-v1.2.3",
            {"darwin-arm64": "a" * 64, "linux-x64": "b" * 64},
            {"darwin-arm64": "c" * 64, "linux-x64": "d" * 64},
        )
        self.assertEqual(manifest["bun_version"], "bun-v1.2.3")
        self.assertEqual(
            manifest["zip_shas_upstream"]["darwin-arm64"],
            "a" * 64,
        )
        self.assertEqual(manifest["r2_object_shas"]["linux-x64"], "d" * 64)
        self.assertIn("uploaded_at", manifest)
        self.assertIn("uploaded_by", manifest)

    def test_codex_manifest_shape(self) -> None:
        manifest = storage._build_codex_manifest(
            "rust-v0.136.0",
            {"darwin-arm64": "a" * 64, "windows-x64": "b" * 64},
            {"darwin-arm64": "c" * 64, "windows-x64": "d" * 64},
        )
        self.assertEqual(manifest["codex_release_tag"], "rust-v0.136.0")
        self.assertEqual(manifest["codex_cli_version"], "0.136.0")
        self.assertEqual(manifest["package_shas_upstream"]["darwin-arm64"], "a" * 64)
        self.assertEqual(manifest["r2_object_shas"]["windows-x64"], "d" * 64)
        self.assertIn("uploaded_at", manifest)
        self.assertIn("uploaded_by", manifest)

    def test_claude_code_manifest_shape(self) -> None:
        manifest = storage._build_claude_code_manifest(
            "2.1.159",
            {"darwin-arm64": "5" * 64, "windows-x64": "6" * 64},
            {
                "darwin-arm64": {"platform": "darwin-arm64", "binary": "claude"},
                "windows-x64": {"platform": "win32-x64", "binary": "claude.exe"},
            },
        )
        self.assertEqual(manifest["claude_code_version"], "2.1.159")
        self.assertEqual(manifest["binary_shas_upstream"]["darwin-arm64"], "5" * 64)
        self.assertEqual(
            manifest["platforms"]["windows-x64"],
            {"platform": "win32-x64", "binary": "claude.exe"},
        )
        self.assertIsNot(
            manifest["binary_shas_upstream"],
            manifest["r2_object_shas"],
        )
        self.assertIn("uploaded_at", manifest)
        self.assertIn("uploaded_by", manifest)


class PackagedAgentCliContractTest(unittest.TestCase):
    def test_server_resource_keys_match_storage_upload_keys(self) -> None:
        manifest_path = (
            Path(__file__).resolve().parents[3]
            / "browseros-agent"
            / "scripts"
            / "build"
            / "config"
            / "server-prod-resources.json"
        )
        manifest = json.loads(manifest_path.read_text())
        actual_keys = {
            f"artifacts/vendor/{resource['source']['key']}"
            for resource in manifest["resources"]
            if resource["source"]["type"] == "r2"
            and resource["source"]["key"].startswith("third_party/codex/")
        }
        expected_keys = {
            f"{storage.CODEX_R2_PREFIX}/{storage._platform_binary_object_name('codex', platform.target)}"
            for platform in storage.CODEX_PLATFORMS
        }

        self.assertEqual(actual_keys, expected_keys)


class ProcessArchTest(unittest.TestCase):
    """Covers download + sha verify + extract + upload in one pass."""

    def setUp(self) -> None:
        self.limactl_payload = b"limactl-binary-" + b"z" * 200
        self.guest_agent_payload = b"guest-agent-" + b"y" * 200
        self.tarball_bytes = _build_lima_tarball(
            "1.2.3",
            self.limactl_payload,
            guest_agents={"aarch64": self.guest_agent_payload},
        )
        self.expected_tarball_sha = hashlib.sha256(self.tarball_bytes).hexdigest()
        self.expected_limactl_sha = hashlib.sha256(self.limactl_payload).hexdigest()
        self.expected_guest_agent_sha = hashlib.sha256(
            self.guest_agent_payload
        ).hexdigest()

    def _fake_download(self, _url: str, dest: Path, **_kwargs: Any) -> None:
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(self.tarball_bytes)

    def test_happy_path_uploads_and_returns_shas(self) -> None:
        uploads: List[Tuple[str, str, bytes]] = []

        def fake_upload(
            _client: Any, local_path: Path, r2_key: str, bucket: str
        ) -> bool:
            uploads.append((r2_key, bucket, local_path.read_bytes()))
            return True

        env = mock.Mock(r2_bucket="browseros")

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            with (
                mock.patch.object(
                    storage, "_download", side_effect=self._fake_download
                ),
                mock.patch.object(
                    storage, "upload_file_to_r2", side_effect=fake_upload
                ),
            ):
                tarball_sha, object_shas, r2_keys = storage._process_arch(
                    tag="v1.2.3",
                    arch=storage.LimaArch(
                        internal="arm64",
                        upstream="Darwin-arm64",
                        linux_guest_arch="aarch64",
                    ),
                    tmp_dir=tmp_path,
                    checksums={
                        "lima-1.2.3-Darwin-arm64.tar.gz": self.expected_tarball_sha
                    },
                    client=mock.Mock(),
                    env=env,
                    dry_run=False,
                )

        self.assertEqual(tarball_sha, self.expected_tarball_sha)
        self.assertEqual(
            object_shas,
            {
                "limactl": self.expected_limactl_sha,
                "guest_agent": self.expected_guest_agent_sha,
            },
        )
        self.assertEqual(
            r2_keys,
            [
                "artifacts/vendor/third_party/lima/limactl-darwin-arm64",
                "artifacts/vendor/third_party/lima/lima-guestagent.Linux-aarch64.gz",
            ],
        )
        self.assertEqual(
            uploads,
            [
                (
                    "artifacts/vendor/third_party/lima/limactl-darwin-arm64",
                    "browseros",
                    self.limactl_payload,
                ),
                (
                    "artifacts/vendor/third_party/lima/lima-guestagent.Linux-aarch64.gz",
                    "browseros",
                    self.guest_agent_payload,
                ),
            ],
        )

    def test_sha_mismatch_aborts_before_upload(self) -> None:
        uploads: List[Tuple[str, str]] = []

        def fake_upload(
            _client: Any, _local_path: Path, r2_key: str, bucket: str
        ) -> bool:
            uploads.append((r2_key, bucket))
            return True

        env = mock.Mock(r2_bucket="browseros")

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            with (
                mock.patch.object(
                    storage, "_download", side_effect=self._fake_download
                ),
                mock.patch.object(
                    storage, "upload_file_to_r2", side_effect=fake_upload
                ),
            ):
                with self.assertRaisesRegex(RuntimeError, "sha256 mismatch"):
                    storage._process_arch(
                        tag="v1.2.3",
                        arch=storage.LimaArch(
                            internal="arm64",
                            upstream="Darwin-arm64",
                            linux_guest_arch="aarch64",
                        ),
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
                    arch=storage.LimaArch(
                        internal="arm64",
                        upstream="Darwin-arm64",
                        linux_guest_arch="aarch64",
                    ),
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
            with (
                mock.patch.object(
                    storage, "_download", side_effect=self._fake_download
                ),
                mock.patch.object(
                    storage, "upload_file_to_r2", side_effect=fake_upload
                ),
            ):
                _, _, r2_keys = storage._process_arch(
                    tag="v1.2.3",
                    arch=storage.LimaArch(
                        internal="arm64",
                        upstream="Darwin-arm64",
                        linux_guest_arch="aarch64",
                    ),
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
            r2_keys,
            [
                "artifacts/vendor/third_party/lima/limactl-darwin-arm64",
                "artifacts/vendor/third_party/lima/lima-guestagent.Linux-aarch64.gz",
            ],
        )

    def test_missing_guest_agent_aborts_before_upload(self) -> None:
        tarball_bytes = _build_lima_tarball("1.2.3", self.limactl_payload)
        expected_sha = hashlib.sha256(tarball_bytes).hexdigest()
        uploads: List[Tuple[str, str]] = []

        def fake_download(_url: str, dest: Path, **_kwargs: Any) -> None:
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes(tarball_bytes)

        def fake_upload(
            _client: Any, _local_path: Path, r2_key: str, bucket: str
        ) -> bool:
            uploads.append((r2_key, bucket))
            return True

        env = mock.Mock(r2_bucket="browseros")

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            with (
                mock.patch.object(storage, "_download", side_effect=fake_download),
                mock.patch.object(
                    storage, "upload_file_to_r2", side_effect=fake_upload
                ),
            ):
                with self.assertRaisesRegex(
                    RuntimeError,
                    "share/lima/lima-guestagent.Linux-aarch64.gz not found",
                ):
                    storage._process_arch(
                        tag="v1.2.3",
                        arch=storage.LimaArch(
                            internal="arm64",
                            upstream="Darwin-arm64",
                            linux_guest_arch="aarch64",
                        ),
                        tmp_dir=tmp_path,
                        checksums={"lima-1.2.3-Darwin-arm64.tar.gz": expected_sha},
                        client=mock.Mock(),
                        env=env,
                        dry_run=False,
                    )

        self.assertEqual(uploads, [])


class ProcessBunTargetTest(unittest.TestCase):
    def setUp(self) -> None:
        self.bun_payload = b"bun-binary-" + b"z" * 200
        self.zip_bytes = _build_bun_zip(self.bun_payload)
        self.expected_zip_sha = hashlib.sha256(self.zip_bytes).hexdigest()
        self.expected_bun_sha = hashlib.sha256(self.bun_payload).hexdigest()

    def _fake_download(self, _url: str, dest: Path, **_kwargs: Any) -> None:
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(self.zip_bytes)

    def test_happy_path_uploads_and_returns_shas(self) -> None:
        uploads: List[Tuple[str, str, bytes]] = []

        def fake_upload(
            _client: Any, local_path: Path, r2_key: str, bucket: str
        ) -> bool:
            uploads.append((r2_key, bucket, local_path.read_bytes()))
            return True

        env = mock.Mock(r2_bucket="browseros")

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            with (
                mock.patch.object(
                    storage, "_download", side_effect=self._fake_download
                ),
                mock.patch.object(
                    storage, "upload_file_to_r2", side_effect=fake_upload
                ),
            ):
                zip_sha, binary_sha, r2_key = storage._process_bun_target(
                    tag="bun-v1.2.3",
                    target=storage.BunTarget(
                        internal="darwin-arm64",
                        upstream="darwin-aarch64",
                        r2_name="bun-darwin-arm64",
                    ),
                    tmp_dir=tmp_path,
                    checksums={"bun-darwin-aarch64.zip": self.expected_zip_sha},
                    client=mock.Mock(),
                    env=env,
                    dry_run=False,
                )

        self.assertEqual(zip_sha, self.expected_zip_sha)
        self.assertEqual(binary_sha, self.expected_bun_sha)
        self.assertEqual(r2_key, "artifacts/vendor/third_party/bun/bun-darwin-arm64")
        self.assertEqual(
            uploads,
            [
                (
                    "artifacts/vendor/third_party/bun/bun-darwin-arm64",
                    "browseros",
                    self.bun_payload,
                )
            ],
        )

    def test_uses_bun_download_timeout(self) -> None:
        download_kwargs: List[Dict[str, Any]] = []

        def fake_download(_url: str, dest: Path, **kwargs: Any) -> None:
            download_kwargs.append(kwargs)
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes(self.zip_bytes)

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            with mock.patch.object(storage, "_download", side_effect=fake_download):
                storage._process_bun_target(
                    tag="bun-v1.2.3",
                    target=storage.BunTarget(
                        internal="darwin-arm64",
                        upstream="darwin-aarch64",
                        r2_name="bun-darwin-arm64",
                    ),
                    tmp_dir=tmp_path,
                    checksums={"bun-darwin-aarch64.zip": self.expected_zip_sha},
                    client=None,
                    env=mock.Mock(r2_bucket="browseros"),
                    dry_run=True,
                )

        self.assertEqual(download_kwargs, [{"timeout": storage.BUN_HTTP_TIMEOUT_S}])

    def test_sha_mismatch_aborts_before_upload(self) -> None:
        uploads: List[Tuple[str, str]] = []

        def fake_upload(
            _client: Any, _local_path: Path, r2_key: str, bucket: str
        ) -> bool:
            uploads.append((r2_key, bucket))
            return True

        env = mock.Mock(r2_bucket="browseros")

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            with (
                mock.patch.object(
                    storage, "_download", side_effect=self._fake_download
                ),
                mock.patch.object(
                    storage, "upload_file_to_r2", side_effect=fake_upload
                ),
            ):
                with self.assertRaisesRegex(RuntimeError, "sha256 mismatch"):
                    storage._process_bun_target(
                        tag="bun-v1.2.3",
                        target=storage.BunTarget(
                            internal="darwin-arm64",
                            upstream="darwin-aarch64",
                            r2_name="bun-darwin-arm64",
                        ),
                        tmp_dir=tmp_path,
                        checksums={"bun-darwin-aarch64.zip": "0" * 64},
                        client=mock.Mock(),
                        env=env,
                        dry_run=False,
                    )

        self.assertEqual(uploads, [])

    def test_dry_run_skips_upload(self) -> None:
        uploads: List[Tuple[str, str]] = []

        def fake_upload(*args: Any, **kwargs: Any) -> bool:
            uploads.append(("called", ""))
            return True

        env = mock.Mock(r2_bucket="browseros")

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            with (
                mock.patch.object(
                    storage, "_download", side_effect=self._fake_download
                ),
                mock.patch.object(
                    storage, "upload_file_to_r2", side_effect=fake_upload
                ),
            ):
                _, _, r2_key = storage._process_bun_target(
                    tag="bun-v1.2.3",
                    target=storage.BunTarget(
                        internal="darwin-arm64",
                        upstream="darwin-aarch64",
                        r2_name="bun-darwin-arm64",
                    ),
                    tmp_dir=tmp_path,
                    checksums={"bun-darwin-aarch64.zip": self.expected_zip_sha},
                    client=None,
                    env=env,
                    dry_run=True,
                )

        self.assertEqual(uploads, [])
        self.assertEqual(r2_key, "artifacts/vendor/third_party/bun/bun-darwin-arm64")

    def test_windows_target_extracts_exe_and_uploads_exe_key(self) -> None:
        payload = b"bun-windows-exe-" + b"w" * 200
        zip_bytes = _build_bun_zip(
            payload,
            root_dir="bun-windows-x64-baseline",
            binary_name="bun.exe",
        )
        expected_zip_sha = hashlib.sha256(zip_bytes).hexdigest()
        expected_bun_sha = hashlib.sha256(payload).hexdigest()
        uploads: List[Tuple[str, str, bytes]] = []

        def fake_download(_url: str, dest: Path, **_kwargs: Any) -> None:
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes(zip_bytes)

        def fake_upload(
            _client: Any, local_path: Path, r2_key: str, bucket: str
        ) -> bool:
            uploads.append((r2_key, bucket, local_path.read_bytes()))
            return True

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            with (
                mock.patch.object(storage, "_download", side_effect=fake_download),
                mock.patch.object(
                    storage,
                    "upload_file_to_r2",
                    side_effect=fake_upload,
                ),
            ):
                zip_sha, binary_sha, r2_key = storage._process_bun_target(
                    tag="bun-v1.2.3",
                    target=storage.BunTarget(
                        internal="windows-x64",
                        upstream="windows-x64-baseline",
                        r2_name="bun-windows-x64-baseline.exe",
                        binary_name="bun.exe",
                    ),
                    tmp_dir=tmp_path,
                    checksums={"bun-windows-x64-baseline.zip": expected_zip_sha},
                    client=mock.Mock(),
                    env=mock.Mock(r2_bucket="browseros"),
                    dry_run=False,
                )

        self.assertEqual(zip_sha, expected_zip_sha)
        self.assertEqual(binary_sha, expected_bun_sha)
        self.assertEqual(
            r2_key,
            "artifacts/vendor/third_party/bun/bun-windows-x64-baseline.exe",
        )
        self.assertEqual(
            uploads,
            [
                (
                    "artifacts/vendor/third_party/bun/bun-windows-x64-baseline.exe",
                    "browseros",
                    payload,
                )
            ],
        )


class ExtractCodexFileTest(unittest.TestCase):
    def test_extracts_declared_entrypoint(self) -> None:
        payload = b"codex-binary-" + b"c" * 100

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            package_path = tmp_path / "codex.tar.gz"
            package_path.write_bytes(_build_codex_package("bin/codex", payload))
            dest = tmp_path / "codex"

            storage._extract_codex_file(package_path, dest)

            self.assertEqual(dest.read_bytes(), payload)
            self.assertTrue(dest.stat().st_mode & 0o100, "should be executable")

    def test_raises_when_declared_entrypoint_missing(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            package_path = tmp_path / "codex.tar.gz"
            package_path.write_bytes(
                _build_codex_package("bin/missing", b"payload", "bin/codex")
            )

            with self.assertRaisesRegex(RuntimeError, "Codex entrypoint .* not found"):
                storage._extract_codex_file(package_path, tmp_path / "codex")

    def test_windows_entrypoint_is_not_marked_executable(self) -> None:
        payload = b"codex-windows-exe-" + b"c" * 100

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            package_path = tmp_path / "codex.tar.gz"
            package_path.write_bytes(_build_codex_package("bin/codex.exe", payload))
            dest = tmp_path / "codex.exe"

            storage._extract_codex_file(package_path, dest)

            self.assertEqual(dest.read_bytes(), payload)
            self.assertFalse(dest.stat().st_mode & 0o100, "should not be executable")


class ProcessCodexPlatformTest(unittest.TestCase):
    def setUp(self) -> None:
        self.codex_payload = b"codex-binary-" + b"x" * 200
        self.package_bytes = _build_codex_package("bin/codex", self.codex_payload)
        self.expected_package_sha = hashlib.sha256(self.package_bytes).hexdigest()
        self.expected_codex_sha = hashlib.sha256(self.codex_payload).hexdigest()

    def _fake_download(self, _url: str, dest: Path, **_kwargs: Any) -> None:
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(self.package_bytes)

    def test_happy_path_uploads_and_returns_shas(self) -> None:
        uploads: List[Tuple[str, str, bytes]] = []

        def fake_upload(
            _client: Any, local_path: Path, r2_key: str, bucket: str
        ) -> bool:
            uploads.append((r2_key, bucket, local_path.read_bytes()))
            return True

        env = mock.Mock(r2_bucket="browseros")

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            with (
                mock.patch.object(
                    storage, "_download", side_effect=self._fake_download
                ),
                mock.patch.object(
                    storage, "upload_file_to_r2", side_effect=fake_upload
                ),
            ):
                package_sha, binary_sha, r2_key = storage._process_codex_platform(
                    tag="rust-v0.136.0",
                    platform=storage.CodexPlatform(
                        target="darwin-arm64",
                        upstream="aarch64-apple-darwin",
                    ),
                    tmp_dir=tmp_path,
                    checksums={
                        "codex-package-aarch64-apple-darwin.tar.gz": self.expected_package_sha
                    },
                    client=mock.Mock(),
                    env=env,
                    dry_run=False,
                )

        self.assertEqual(package_sha, self.expected_package_sha)
        self.assertEqual(binary_sha, self.expected_codex_sha)
        self.assertEqual(
            r2_key, "artifacts/vendor/third_party/codex/codex-darwin-arm64"
        )
        self.assertEqual(
            uploads,
            [
                (
                    "artifacts/vendor/third_party/codex/codex-darwin-arm64",
                    "browseros",
                    self.codex_payload,
                )
            ],
        )

    def test_sha_mismatch_aborts_before_upload(self) -> None:
        uploads: List[Tuple[str, str]] = []

        def fake_upload(
            _client: Any, _local_path: Path, r2_key: str, bucket: str
        ) -> bool:
            uploads.append((r2_key, bucket))
            return True

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            with (
                mock.patch.object(
                    storage, "_download", side_effect=self._fake_download
                ),
                mock.patch.object(
                    storage, "upload_file_to_r2", side_effect=fake_upload
                ),
            ):
                with self.assertRaisesRegex(RuntimeError, "sha256 mismatch"):
                    storage._process_codex_platform(
                        tag="rust-v0.136.0",
                        platform=storage.CodexPlatform(
                            target="darwin-arm64",
                            upstream="aarch64-apple-darwin",
                        ),
                        tmp_dir=tmp_path,
                        checksums={
                            "codex-package-aarch64-apple-darwin.tar.gz": "0" * 64
                        },
                        client=mock.Mock(),
                        env=mock.Mock(r2_bucket="browseros"),
                        dry_run=False,
                    )

        self.assertEqual(uploads, [])

    def test_dry_run_skips_upload(self) -> None:
        uploads: List[Tuple[str, str]] = []

        def fake_upload(*args: Any, **kwargs: Any) -> bool:
            uploads.append(("called", ""))
            return True

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            with (
                mock.patch.object(
                    storage, "_download", side_effect=self._fake_download
                ),
                mock.patch.object(
                    storage, "upload_file_to_r2", side_effect=fake_upload
                ),
            ):
                _, _, r2_key = storage._process_codex_platform(
                    tag="rust-v0.136.0",
                    platform=storage.CodexPlatform(
                        target="windows-x64",
                        upstream="x86_64-pc-windows-msvc",
                    ),
                    tmp_dir=tmp_path,
                    checksums={
                        "codex-package-x86_64-pc-windows-msvc.tar.gz": self.expected_package_sha
                    },
                    client=None,
                    env=mock.Mock(r2_bucket="browseros"),
                    dry_run=True,
                )

        self.assertEqual(uploads, [])
        self.assertEqual(
            r2_key, "artifacts/vendor/third_party/codex/codex-windows-x64.exe"
        )


class ProcessClaudeCodePlatformTest(unittest.TestCase):
    def setUp(self) -> None:
        self.claude_payload = b"claude-binary-" + b"y" * 200
        self.expected_binary_sha = hashlib.sha256(self.claude_payload).hexdigest()
        self.manifest = {
            "platforms": {
                "darwin-arm64": {
                    "binary": "claude",
                    "checksum": self.expected_binary_sha,
                    "size": len(self.claude_payload),
                },
                "win32-x64": {
                    "binary": "claude.exe",
                    "checksum": self.expected_binary_sha,
                    "size": len(self.claude_payload),
                },
            }
        }

    def _fake_download(self, _url: str, dest: Path, **_kwargs: Any) -> None:
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(self.claude_payload)

    def test_happy_path_uploads_and_returns_shas(self) -> None:
        uploads: List[Tuple[str, str, bytes]] = []

        def fake_upload(
            _client: Any, local_path: Path, r2_key: str, bucket: str
        ) -> bool:
            uploads.append((r2_key, bucket, local_path.read_bytes()))
            return True

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            with (
                mock.patch.object(
                    storage, "_download", side_effect=self._fake_download
                ),
                mock.patch.object(
                    storage, "upload_file_to_r2", side_effect=fake_upload
                ),
            ):
                binary_sha, r2_key = storage._process_claude_code_platform(
                    version="2.1.159",
                    platform=storage.ClaudeCodePlatform(
                        target="darwin-arm64",
                        upstream="darwin-arm64",
                    ),
                    manifest=self.manifest,
                    tmp_dir=tmp_path,
                    client=mock.Mock(),
                    env=mock.Mock(r2_bucket="browseros"),
                    dry_run=False,
                )

        self.assertEqual(binary_sha, self.expected_binary_sha)
        self.assertEqual(
            r2_key,
            "artifacts/vendor/third_party/claude-code/claude-darwin-arm64",
        )
        self.assertEqual(
            uploads,
            [
                (
                    "artifacts/vendor/third_party/claude-code/claude-darwin-arm64",
                    "browseros",
                    self.claude_payload,
                )
            ],
        )

    def test_size_mismatch_aborts_before_upload(self) -> None:
        uploads: List[Tuple[str, str]] = []
        bad_manifest = {
            "platforms": {
                "darwin-arm64": {
                    "binary": "claude",
                    "checksum": self.expected_binary_sha,
                    "size": len(self.claude_payload) + 1,
                }
            }
        }

        def fake_upload(
            _client: Any, _local_path: Path, r2_key: str, bucket: str
        ) -> bool:
            uploads.append((r2_key, bucket))
            return True

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            with (
                mock.patch.object(
                    storage, "_download", side_effect=self._fake_download
                ),
                mock.patch.object(
                    storage, "upload_file_to_r2", side_effect=fake_upload
                ),
            ):
                with self.assertRaisesRegex(RuntimeError, "size mismatch"):
                    storage._process_claude_code_platform(
                        version="2.1.159",
                        platform=storage.ClaudeCodePlatform(
                            target="darwin-arm64",
                            upstream="darwin-arm64",
                        ),
                        manifest=bad_manifest,
                        tmp_dir=tmp_path,
                        client=mock.Mock(),
                        env=mock.Mock(r2_bucket="browseros"),
                        dry_run=False,
                    )

        self.assertEqual(uploads, [])

    def test_missing_manifest_platform_aborts(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(
                RuntimeError, "missing from Claude Code manifest"
            ):
                storage._process_claude_code_platform(
                    version="2.1.159",
                    platform=storage.ClaudeCodePlatform(
                        target="linux-x64",
                        upstream="linux-x64",
                    ),
                    manifest=self.manifest,
                    tmp_dir=Path(tmp),
                    client=mock.Mock(),
                    env=mock.Mock(r2_bucket="browseros"),
                    dry_run=False,
                )

    def test_dry_run_skips_upload_and_uses_manifest_binary_name(self) -> None:
        uploads: List[Tuple[str, str]] = []

        def fake_upload(*args: Any, **kwargs: Any) -> bool:
            uploads.append(("called", ""))
            return True

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            with (
                mock.patch.object(
                    storage, "_download", side_effect=self._fake_download
                ),
                mock.patch.object(
                    storage, "upload_file_to_r2", side_effect=fake_upload
                ),
            ):
                _, r2_key = storage._process_claude_code_platform(
                    version="2.1.159",
                    platform=storage.ClaudeCodePlatform(
                        target="windows-x64",
                        upstream="win32-x64",
                    ),
                    manifest=self.manifest,
                    tmp_dir=tmp_path,
                    client=None,
                    env=mock.Mock(r2_bucket="browseros"),
                    dry_run=True,
                )

        self.assertEqual(uploads, [])
        self.assertEqual(
            r2_key,
            "artifacts/vendor/third_party/claude-code/claude-windows-x64.exe",
        )


class UploadVendorRollbackTest(unittest.TestCase):
    def test_codex_upload_rolls_back_objects_uploaded_before_failure(self) -> None:
        client = mock.Mock()
        env = mock.Mock(
            r2_bucket="browseros", has_r2_config=mock.Mock(return_value=True)
        )
        calls = 0

        def fake_process(
            tag: str,
            platform: Any,
            tmp_dir: Path,
            checksums: Dict[str, str],
            client_arg: Any,
            env_arg: Any,
            dry_run: bool,
            uploaded_keys: List[str],
        ) -> Tuple[str, str, str]:
            nonlocal calls
            calls += 1
            if calls == 1:
                key = "artifacts/vendor/third_party/codex/codex-darwin-arm64"
                uploaded_keys.append(key)
                return "a" * 64, "b" * 64, key
            raise RuntimeError("boom")

        with (
            mock.patch.object(storage, "BOTO3_AVAILABLE", True),
            mock.patch.object(storage, "EnvConfig", return_value=env),
            mock.patch.object(storage, "get_r2_client", return_value=client),
            mock.patch.object(
                storage, "_fetch_codex_package_checksums", return_value={}
            ),
            mock.patch.object(
                storage, "_process_codex_platform", side_effect=fake_process
            ),
        ):
            with self.assertRaises(Exception) as exc:
                storage.upload_codex(version="rust-v0.136.0", dry_run=False)

        self.assertEqual(exc.exception.exit_code, 1)
        client.delete_object.assert_called_once_with(
            Bucket="browseros",
            Key="artifacts/vendor/third_party/codex/codex-darwin-arm64",
        )


if __name__ == "__main__":
    unittest.main()
