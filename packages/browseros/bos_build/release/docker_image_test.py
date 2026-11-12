#!/usr/bin/env python3
"""Neo Docker image build contract, exercised through build_image."""

import tempfile
import unittest
from contextlib import contextmanager
from pathlib import Path
from typing import Iterator, Optional

from bos_build.release.docker_image import (
    ImageError,
    ImageRequest,
    ImageResult,
    build_image,
    publish_index,
)

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
        self.indexes: list[list[str]] = []
        # What the registry reports per reference; anything else gets `published`.
        self.images: dict[str, dict] = {}
        # What the registry reports for a tag after an index is created.
        self.published = {
            "digest": "sha256:" + "f" * 64,
            "configs": {
                platform: {"config": {"Labels": {"org.opencontainers.image.revision": REVISION}}}
                for platform in ("linux/amd64", "linux/arm64")
            },
        }

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

    def create_index(self, argv: list[str]) -> None:
        self.indexes.append(argv)

    def inspect_image(self, reference: str) -> dict:
        if reference in self.images:
            return self.images[reference]
        if "@sha256:" in reference:
            # A pushed architecture is its own one-platform image from this commit.
            platform = "linux/amd64" if reference.endswith(AMD64) else "linux/arm64"
            labels = {"org.opencontainers.image.revision": REVISION}
            return {"digest": reference.split("@")[1],
                    "configs": {platform: {"config": {"Labels": labels}}}}
        return self.published


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

AMD64 = "sha256:" + "a" * 64
ARM64 = "sha256:" + "b" * 64


class PushByDigestTest(unittest.TestCase):
    """verify pushes the one image it tested, without tags; publish names it later."""

    def _push(self, ops: FakeOperations, **request) -> ImageResult:
        with _servers("arm64") as servers:
            self.servers = servers
            return build_image(
                ImageRequest(version="0.52.0.1", push_by_digest=True, server_dir=servers,
                             revision=REVISION, platforms=("linux/arm64",), **request),
                ops, CONTEXT,
            )

    def test_pushes_one_architecture_untagged_from_the_tested_build(self):
        ops = FakeOperations(
            release=_release("x64_deb", "arm64_deb"),
            metadata={"containerimage.digest": ARM64},
        )

        result = self._push(ops)

        self.assertEqual(
            ops.builds,
            [
                [
                    "--platform", "linux/arm64",
                    "--build-arg", "NEO_VERSION=0.52.0.1",
                    "--label", "org.opencontainers.image.version=0.52.0.1-169e41f",
                    "--label", f"org.opencontainers.image.revision={REVISION}",
                    "--label", "com.browseros.claw-server.version=0.0.77",
                    "--output",
                    "type=image,name=ghcr.io/browseros-ai/neo,push-by-digest=true,name-canonical=true,push=true",
                    "--provenance", "mode=min",
                    "--annotation",
                    "index:org.opencontainers.image.source=https://github.com/browseros-ai/BrowserOS",
                    "--build-context", f"neo-server={self.servers}",
                    str(CONTEXT),
                ]
            ],
        )
        # No --no-cache-filter: every layer, the credentials stage included, comes from the
        # cache of the build that was just tested, so the pushed image is that image.
        self.assertNotIn("--no-cache-filter", ops.builds[0])
        self.assertEqual(result.tags, ())
        self.assertEqual(result.digest, ARM64)

    def test_needs_exactly_one_architecture(self):
        ops = FakeOperations(release=_release("x64_deb", "arm64_deb"))
        with _servers("amd64", "arm64") as servers:
            for platforms in ((), ("linux/amd64", "linux/arm64")):
                with self.assertRaisesRegex(ImageError, "one architecture"):
                    build_image(
                        ImageRequest(version="0.52.0.1", push_by_digest=True, server_dir=servers,
                                     revision=REVISION, platforms=platforms),
                        ops, CONTEXT,
                    )
        self.assertEqual(ops.builds, [])

    def test_needs_this_commits_server_and_revision(self):
        ops = FakeOperations(release=_release("arm64_deb"))
        with self.assertRaisesRegex(ImageError, "--server-dir"):
            build_image(
                ImageRequest(version="0.52.0.1", push_by_digest=True, revision=REVISION,
                             platforms=("linux/arm64",)),
                ops, CONTEXT,
            )
        self.assertEqual(ops.builds, [])

    def test_is_not_combined_with_a_tagged_push(self):
        with self.assertRaisesRegex(ImageError, "--push-by-digest"):
            self._push(FakeOperations(release=_release("arm64_deb")), push=True)


class PublishIndexTest(unittest.TestCase):
    """publish tags the architectures verify pushed, building nothing."""

    def _publish(self, ops: FakeOperations, digests: dict, **request) -> ImageResult:
        return publish_index(
            ImageRequest(version="0.52.0.1", push=True, revision=REVISION, **request),
            digests, ops,
        )

    def test_tags_one_index_over_both_tested_architectures(self):
        ops = FakeOperations()

        result = self._publish(ops, {"linux/amd64": AMD64, "linux/arm64": ARM64})

        self.assertEqual(
            ops.indexes,
            [
                [
                    "--annotation",
                    "index:org.opencontainers.image.source=https://github.com/browseros-ai/BrowserOS",
                    "--tag", "ghcr.io/browseros-ai/neo:0.52.0.1-169e41f",
                    "--tag", "ghcr.io/browseros-ai/neo:0.52.0.1",
                    f"ghcr.io/browseros-ai/neo@{AMD64}",
                    f"ghcr.io/browseros-ai/neo@{ARM64}",
                ]
            ],
        )
        self.assertEqual(ops.builds, [])
        self.assertEqual(result.tags[0], "ghcr.io/browseros-ai/neo:0.52.0.1-169e41f")
        self.assertEqual(result.platforms, ("linux/amd64", "linux/arm64"))
        self.assertEqual(result.digest, "sha256:" + "f" * 64)

    def test_latest_adds_the_latest_tag(self):
        ops = FakeOperations()

        result = self._publish(ops, {"linux/amd64": AMD64, "linux/arm64": ARM64}, latest=True)

        self.assertEqual(result.tags[-1], "ghcr.io/browseros-ai/neo:latest")
        self.assertIn("ghcr.io/browseros-ai/neo:latest", ops.indexes[0])

    def test_refuses_an_index_missing_an_architecture(self):
        ops = FakeOperations()

        with self.assertRaisesRegex(ImageError, "linux/arm64"):
            self._publish(ops, {"linux/amd64": AMD64})
        self.assertEqual(ops.indexes, [])

    def test_refuses_a_malformed_digest(self):
        ops = FakeOperations()

        with self.assertRaisesRegex(ImageError, "digest"):
            self._publish(ops, {"linux/amd64": AMD64, "linux/arm64": "latest"})
        self.assertEqual(ops.indexes, [])

    def test_fails_when_the_published_index_is_not_this_commits(self):
        ops = FakeOperations()
        ops.published["configs"]["linux/arm64"]["config"]["Labels"][
            "org.opencontainers.image.revision"
        ] = "0" * 40

        with self.assertRaisesRegex(ImageError, "linux/arm64"):
            self._publish(ops, {"linux/amd64": AMD64, "linux/arm64": ARM64})

    def test_checks_each_source_before_moving_any_tag(self):
        # A digest from another commit's build must never get this commit's tags.
        ops = FakeOperations()
        ops.images[f"ghcr.io/browseros-ai/neo@{ARM64}"] = {
            "digest": ARM64,
            "configs": {"linux/arm64": {"config": {"Labels": {
                "org.opencontainers.image.revision": "0" * 40}}}},
        }

        with self.assertRaisesRegex(ImageError, "linux/arm64"):
            self._publish(ops, {"linux/amd64": AMD64, "linux/arm64": ARM64}, latest=True)
        self.assertEqual(ops.indexes, [])

    def test_checks_each_source_is_its_own_architecture(self):
        ops = FakeOperations()
        ops.images[f"ghcr.io/browseros-ai/neo@{AMD64}"] = {
            "digest": AMD64,
            "configs": {"linux/arm64": ops.published["configs"]["linux/arm64"]},
        }

        with self.assertRaisesRegex(ImageError, "linux/amd64"):
            self._publish(ops, {"linux/amd64": AMD64, "linux/arm64": ARM64})
        self.assertEqual(ops.indexes, [])

    def test_is_not_combined_with_a_push_by_digest(self):
        with self.assertRaisesRegex(ImageError, "--push-by-digest"):
            self._publish(FakeOperations(), {"linux/amd64": AMD64, "linux/arm64": ARM64},
                          push_by_digest=True)

    def test_fails_when_the_published_index_lacks_an_architecture(self):
        ops = FakeOperations()
        del ops.published["configs"]["linux/amd64"]

        with self.assertRaisesRegex(ImageError, "linux/amd64"):
            self._publish(ops, {"linux/amd64": AMD64, "linux/arm64": ARM64})


if __name__ == "__main__":
    unittest.main()
