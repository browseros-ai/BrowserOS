#!/usr/bin/env python3
"""Neo Docker image build contract, exercised through build_image."""

import unittest
from contextlib import contextmanager
from pathlib import Path
from typing import Iterator, Optional

from bos_build.release.docker_image import ImageError, ImageRequest, build_image

CONTEXT = Path("/repo/packages/browseros/docker/neo")


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
    def test_push_builds_both_published_architectures_under_the_version_tag(self):
        ops = FakeOperations(
            release=_release("x64_deb", "arm64_deb"),
            metadata={"containerimage.digest": "sha256:abc"},
        )

        result = build_image(ImageRequest(version="0.52.0.1", push=True), ops, CONTEXT)

        self.assertEqual(
            ops.builds,
            [
                [
                    "--platform", "linux/amd64,linux/arm64",
                    "--build-arg", "NEO_VERSION=0.52.0.1",
                    "--tag", "ghcr.io/browseros-ai/neo:0.52.0.1",
                    "--push",
                    "--provenance", "mode=min",
                    "--annotation",
                    "index:org.opencontainers.image.source=https://github.com/browseros-ai/BrowserOS",
                    str(CONTEXT),
                ]
            ],
        )
        self.assertEqual(result.tags, ("ghcr.io/browseros-ai/neo:0.52.0.1",))
        self.assertEqual(result.platforms, ("linux/amd64", "linux/arm64"))
        self.assertEqual(result.digest, "sha256:abc")

    def test_push_refuses_a_version_missing_an_architecture(self):
        ops = FakeOperations(release=_release("x64_deb"))

        with self.assertRaisesRegex(ImageError, r"0\.52\.0\.1 has no arm64_deb"):
            build_image(ImageRequest(version="0.52.0.1", push=True), ops, CONTEXT)
        self.assertEqual(ops.builds, [])

    def test_push_refuses_a_version_without_a_linux_release(self):
        ops = FakeOperations(release=None)

        with self.assertRaisesRegex(ImageError, "no Linux release"):
            build_image(ImageRequest(version="0.52.0.1", push=True), ops, CONTEXT)
        self.assertEqual(ops.builds, [])

    def test_latest_adds_the_latest_tag(self):
        ops = FakeOperations(release=_release("x64_deb", "arm64_deb"))

        result = build_image(
            ImageRequest(version="0.52.0.1", push=True, latest=True), ops, CONTEXT
        )

        self.assertEqual(
            result.tags,
            ("ghcr.io/browseros-ai/neo:0.52.0.1", "ghcr.io/browseros-ai/neo:latest"),
        )
        self.assertIn("ghcr.io/browseros-ai/neo:latest", ops.builds[0])


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
                "--load",
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
                "--load",
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


if __name__ == "__main__":
    unittest.main()
