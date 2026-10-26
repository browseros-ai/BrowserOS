#!/usr/bin/env python3
"""Neo Docker image build contract, exercised through build_image."""

import tempfile
import unittest
from contextlib import contextmanager
from pathlib import Path
from typing import Iterator, Optional

from bos_build.release.docker_image import ImageError, ImageRequest, ImageResult, build_image

CONTEXT = Path("/repo/packages/browseros/docker/neo")
REVISION = "169e41f5a0eb9a507a5b976c65ee644733cdc900"


@contextmanager
def _servers(*arches: str, version: str = "0.0.77") -> Iterator[Path]:
    """A --server-dir holding fake binaries that embed claw-server's version marker."""
    with tempfile.TemporaryDirectory() as directory:
        for arch in arches:
            binary = Path(directory) / arch / "browseros-claw-server-rs"
            binary.parent.mkdir()
            binary.write_bytes(b"\x7fELF...browseros-claw-server-version=" + version.encode() + b";...")
        yield Path(directory)


def _release(*keys: str) -> dict:
    return {
        "version": "0.52.0.1",
        "artifacts": {key: {"url": f"http://cdn/{key}", "sha256": "0" * 64} for key in keys},
    }


class FakeOperations:
    def __init__(
        self,
        release: Optional[dict] = None,
        host: str = "linux/arm64",
        metadata: Optional[dict] = None,
    ):
        self.release = release
        self.host = host
        self.metadata = metadata or {}
        self.fetched: list[str] = []
        self.builds: list[list[str]] = []
        self.staged: list[Path] = []

    def fetch_release(self, version: str) -> Optional[dict]:
        self.fetched.append(version)
        return self.release

    def host_platform(self) -> str:
        return self.host

    @contextmanager
    def staged_deb(self, deb: Path) -> Iterator[Path]:
        self.staged.append(deb)
        yield Path("/staged")

    def run_buildx(self, argv: list[str]) -> dict:
        self.builds.append(argv)
        return self.metadata


class PushTest(unittest.TestCase):
    def _push(self, ops: "FakeOperations", **request) -> "ImageResult":
        with _servers("amd64", "arm64") as servers:
            self.servers = servers
            return build_image(
                ImageRequest(version="0.52.0.1", push=True, server_dir=servers,
                             revision=REVISION, **request),
                ops, CONTEXT,
            )

    def test_push_tags_the_commit_and_moves_the_version_alias(self):
        ops = FakeOperations(
            release=_release("x64_deb", "arm64_deb"),
            metadata={"containerimage.digest": "sha256:abc"},
        )

        result = self._push(ops)

        self.assertEqual(
            ops.builds,
            [
                [
                    "--platform", "linux/amd64,linux/arm64",
                    "--build-arg", "NEO_VERSION=0.52.0.1",
                    "--tag", "ghcr.io/browseros-ai/neo:0.52.0.1-169e41f",
                    "--tag", "ghcr.io/browseros-ai/neo:0.52.0.1",
                    "--label", "org.opencontainers.image.version=0.52.0.1-169e41f",
                    "--label", f"org.opencontainers.image.revision={REVISION}",
                    "--label", "com.browseros.claw-server.version=0.0.77",
                    "--push",
                    "--provenance", "mode=min",
                    "--annotation",
                    "index:org.opencontainers.image.source=https://github.com/browseros-ai/BrowserOS",
                    "--build-context", f"neo-server={self.servers}",
                    "--no-cache-filter", "neo",
                    str(CONTEXT),
                ]
            ],
        )
        self.assertEqual(
            result.tags,
            ("ghcr.io/browseros-ai/neo:0.52.0.1-169e41f", "ghcr.io/browseros-ai/neo:0.52.0.1"),
        )
        self.assertEqual(result.platforms, ("linux/amd64", "linux/arm64"))
        self.assertEqual(result.digest, "sha256:abc")

    def test_push_refuses_a_version_missing_an_architecture(self):
        ops = FakeOperations(release=_release("x64_deb"))

        with self.assertRaisesRegex(ImageError, r"0\.52\.0\.1 has no arm64_deb"):
            self._push(ops)
        self.assertEqual(ops.builds, [])

    def test_push_refuses_a_version_without_a_linux_release(self):
        ops = FakeOperations(release=None)

        with self.assertRaisesRegex(ImageError, "no Linux release"):
            self._push(ops)
        self.assertEqual(ops.builds, [])

    def test_latest_adds_the_latest_tag(self):
        ops = FakeOperations(release=_release("x64_deb", "arm64_deb"))

        result = self._push(ops, latest=True)

        self.assertEqual(result.tags[-1], "ghcr.io/browseros-ai/neo:latest")
        self.assertIn("ghcr.io/browseros-ai/neo:latest", ops.builds[0])

    def test_push_requires_this_commits_server(self):
        # Without it the image would run the package's server under a commit tag.
        ops = FakeOperations(release=_release("x64_deb", "arm64_deb"))

        with self.assertRaisesRegex(ImageError, "--server-dir"):
            build_image(
                ImageRequest(version="0.52.0.1", push=True, revision=REVISION), ops, CONTEXT
            )
        self.assertEqual(ops.builds, [])

    def test_a_commit_tag_requires_that_commits_server(self):
        # A local build would otherwise run the package's older server under the commit tag.
        ops = FakeOperations(release=_release("arm64_deb"))

        with self.assertRaisesRegex(ImageError, "--server-dir"):
            build_image(ImageRequest(version="0.52.0.1", revision=REVISION), ops, CONTEXT)
        self.assertEqual(ops.builds, [])

    def test_push_requires_a_full_commit_revision(self):
        ops = FakeOperations(release=_release("x64_deb", "arm64_deb"))
        for revision in ("", "169e41f", "main"):
            with self.subTest(revision=revision), _servers("amd64", "arm64") as servers:
                with self.assertRaisesRegex(ImageError, "revision"):
                    build_image(
                        ImageRequest(version="0.52.0.1", push=True, server_dir=servers,
                                     revision=revision),
                        ops, CONTEXT,
                    )
        self.assertEqual(ops.builds, [])


class LoadTest(unittest.TestCase):
    def test_load_builds_only_the_host_architecture_into_local_docker(self):
        ops = FakeOperations(release=_release("arm64_deb"), host="linux/arm64")

        result = build_image(ImageRequest(version="0.52.0.1"), ops, CONTEXT)

        self.assertEqual(
            ops.builds[0],
            [
                "--platform", "linux/arm64",
                "--build-arg", "NEO_VERSION=0.52.0.1",
                "--tag", "ghcr.io/browseros-ai/neo:0.52.0.1",
                "--label", "org.opencontainers.image.version=0.52.0.1",
                "--load",
                "--no-cache-filter", "neo",
                str(CONTEXT),
            ],
        )
        self.assertEqual(result.platforms, ("linux/arm64",))
        self.assertEqual(result.digest, "")

    def test_load_refuses_more_than_one_architecture(self):
        ops = FakeOperations(release=_release("x64_deb", "arm64_deb"))
        request = ImageRequest(version="0.52.0.1", platforms=("linux/amd64", "linux/arm64"))

        with self.assertRaisesRegex(ImageError, "one platform"):
            build_image(request, ops, CONTEXT)

    def test_latest_is_only_for_published_images(self):
        ops = FakeOperations(release=_release("arm64_deb"))

        with self.assertRaisesRegex(ImageError, "--latest requires --push"):
            build_image(ImageRequest(version="0.52.0.1", latest=True), ops, CONTEXT)

    def test_unknown_platform_is_rejected(self):
        ops = FakeOperations(release=_release("x64_deb", "arm64_deb"))

        with self.assertRaisesRegex(ImageError, "linux/riscv64"):
            build_image(
                ImageRequest(version="0.52.0.1", platforms=("linux/riscv64",)), ops, CONTEXT
            )

    def test_malformed_version_is_rejected_before_any_lookup(self):
        ops = FakeOperations(release=_release("x64_deb", "arm64_deb"))

        with self.assertRaisesRegex(ImageError, "version"):
            build_image(ImageRequest(version="latest", push=True), ops, CONTEXT)
        self.assertEqual(ops.fetched, [])


class LocalDebTest(unittest.TestCase):
    DEB = Path("/downloads/BrowserOS_neo_v0.51.1.4_arm64.deb")

    def test_local_deb_builds_its_own_version_and_architecture_without_the_cdn(self):
        ops = FakeOperations(host="linux/amd64")

        result = build_image(ImageRequest(deb=self.DEB), ops, CONTEXT)

        self.assertEqual(ops.fetched, [])
        self.assertEqual(ops.staged, [self.DEB])
        self.assertEqual(
            ops.builds[0],
            [
                "--platform", "linux/arm64",
                "--build-arg", "NEO_VERSION=0.51.1.4",
                "--tag", "ghcr.io/browseros-ai/neo:0.51.1.4",
                "--label", "org.opencontainers.image.version=0.51.1.4",
                "--load",
                "--no-cache-filter", "neo",
                "--build-context", "neo-deb=/staged",
                str(CONTEXT),
            ],
        )
        self.assertEqual(result.tags, ("ghcr.io/browseros-ai/neo:0.51.1.4",))

    def test_local_deb_is_never_pushed(self):
        with self.assertRaisesRegex(ImageError, "never pushed"):
            build_image(ImageRequest(deb=self.DEB, push=True), FakeOperations(), CONTEXT)

    def test_local_deb_must_match_the_requested_platform(self):
        request = ImageRequest(deb=self.DEB, platforms=("linux/amd64",))

        with self.assertRaisesRegex(ImageError, "arm64"):
            build_image(request, FakeOperations(), CONTEXT)

    def test_local_deb_version_must_match_its_file_name(self):
        request = ImageRequest(deb=self.DEB, version="0.52.0.1")

        with self.assertRaisesRegex(ImageError, "0.51.1.4"):
            build_image(request, FakeOperations(), CONTEXT)

    def test_renamed_deb_needs_an_explicit_version(self):
        renamed = Path("/downloads/neo.deb")

        with self.assertRaisesRegex(ImageError, "--version"):
            build_image(ImageRequest(deb=renamed), FakeOperations(), CONTEXT)
        result = build_image(ImageRequest(deb=renamed, version="0.51.1.4"), FakeOperations(), CONTEXT)
        self.assertEqual(result.platforms, ("linux/arm64",))


class ImageContentsTest(unittest.TestCase):
    def test_provider_keys_use_secret_mounts_and_always_rebuild_credentials(self):
        ops = FakeOperations(release=_release("arm64_deb"))
        with tempfile.TemporaryDirectory() as directory:
            capsolver = Path(directory) / "capsolver"
            twocaptcha = Path(directory) / "2captcha"
            capsolver.write_text("fake-capsolver-value")
            twocaptcha.write_text("fake-2captcha-value")
            request = ImageRequest(
                version="0.52.0.1", capsolver_key_file=capsolver,
                twocaptcha_key_file=twocaptcha,
            )

            build_image(request, ops, CONTEXT)

            argv = ops.builds[0]
            self.assertIn(f"id=capsolver_api_key,src={capsolver}", argv)
            self.assertIn(f"id=twocaptcha_api_key,src={twocaptcha}", argv)
            self.assertIn("--no-cache-filter", argv)
            self.assertIn("neo", argv)
            self.assertNotIn("fake-capsolver-value", " ".join(argv))
            self.assertNotIn("fake-2captcha-value", " ".join(argv))

    def test_missing_key_file_is_rejected_before_building(self):
        ops = FakeOperations(release=_release("arm64_deb"))
        request = ImageRequest(version="0.52.0.1", capsolver_key_file=Path("/missing/key"))

        with self.assertRaisesRegex(ImageError, "key file"):
            build_image(request, ops, CONTEXT)

        self.assertEqual(ops.builds, [])

    def test_server_override_supplies_only_the_requested_architectures(self):
        ops = FakeOperations(release=_release("arm64_deb"))
        with _servers("arm64") as servers:
            build_image(ImageRequest(version="0.52.0.1", server_dir=servers), ops, CONTEXT)

            self.assertIn(f"neo-server={servers}", ops.builds[0])
            self.assertIn("com.browseros.claw-server.version=0.0.77", ops.builds[0])

            with self.assertRaisesRegex(ImageError, "amd64"):
                build_image(
                    ImageRequest(version="0.52.0.1", server_dir=servers, push=True,
                                 revision=REVISION),
                    FakeOperations(release=_release("x64_deb", "arm64_deb")), CONTEXT,
                )

    def test_local_build_with_a_revision_gets_the_commit_tag(self):
        ops = FakeOperations(release=_release("arm64_deb"))
        with _servers("arm64") as servers:
            result = build_image(
                ImageRequest(version="0.52.0.1", server_dir=servers, revision=REVISION),
                ops, CONTEXT,
            )

        self.assertEqual(
            result.tags,
            ("ghcr.io/browseros-ai/neo:0.52.0.1-169e41f", "ghcr.io/browseros-ai/neo:0.52.0.1"),
        )
        self.assertIn(f"org.opencontainers.image.revision={REVISION}", ops.builds[0])

    def test_server_without_a_version_marker_is_rejected(self):
        ops = FakeOperations(release=_release("arm64_deb"))
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "arm64/browseros-claw-server-rs"
            binary.parent.mkdir()
            binary.write_bytes(b"not a claw-server")

            with self.assertRaisesRegex(ImageError, "version marker"):
                build_image(ImageRequest(version="0.52.0.1", server_dir=Path(directory)), ops, CONTEXT)
        self.assertEqual(ops.builds, [])

    def test_architectures_must_carry_the_same_server_version(self):
        ops = FakeOperations(release=_release("x64_deb", "arm64_deb"))
        with _servers("amd64") as servers:
            arm64 = servers / "arm64/browseros-claw-server-rs"
            arm64.parent.mkdir()
            arm64.write_bytes(b"browseros-claw-server-version=0.0.78;")

            with self.assertRaisesRegex(ImageError, "0.0.77.*0.0.78|0.0.78.*0.0.77"):
                build_image(
                    ImageRequest(version="0.52.0.1", push=True, server_dir=servers,
                                 revision=REVISION),
                    ops, CONTEXT,
                )
        self.assertEqual(ops.builds, [])

if __name__ == "__main__":
    unittest.main()
