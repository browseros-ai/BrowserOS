#!/usr/bin/env python3
"""Tests for immutable and idempotent server OTA payload generation."""

import hashlib
import io
import json
import tempfile
import unittest
import zipfile
from pathlib import Path
from types import SimpleNamespace
from typing import cast
from unittest import mock

from ...core.context import Context
from ..feeds.render import ExistingAppcast, SignedArtifact, render_server_appcast
from ..feeds.spec import server_feed
from ..resource_pins import ResourceObjectPin, ResourcePin
from . import server as ota_server
from .common import SERVER_PLATFORMS
from .server import ServerOTAModule


SOURCE_SHA = "a" * 40


class _PreconditionFailure(Exception):
    response = {
        "Error": {"Code": "PreconditionFailed"},
        "ResponseMetadata": {"HTTPStatusCode": 412},
    }


def _complete_appcast(version: str = "0.0.9") -> str:
    artifacts = [
        SignedArtifact(
            platform=platform["name"],
            zip_path=Path(f"{platform['name']}.zip"),
            signature=f"signature-{platform['name']}",
            length=100,
            os=platform["os"],
            arch=platform["arch"],
        )
        for platform in SERVER_PLATFORMS
    ]
    return render_server_appcast(
        server_feed("browseros-server", "alpha"),
        version,
        artifacts,
        ExistingAppcast(
            version=version,
            pub_date="Wed, 05 Aug 2026 12:00:00 +0000",
            artifacts={},
        ),
    )


class LiveReleaseReuseTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.output = Path(self._tmp.name) / "appcast-server.alpha.xml"
        self.registry = SimpleNamespace(add=mock.Mock())
        self.ctx = cast(
            Context,
            SimpleNamespace(
                env=SimpleNamespace(r2_bucket="browseros"),
                artifact_registry=self.registry,
            ),
        )

    def test_stages_same_version_live_feed_without_rebuilding_payloads(self):
        live = _complete_appcast()
        module = ServerOTAModule(
            version="0.0.9",
            channel="alpha",
            platform_filter="darwin_arm64",
            release_sha=SOURCE_SHA,
        )
        publisher = SimpleNamespace(fetch_live=lambda _: live)

        with (
            mock.patch.object(ota_server, "FeedPublisher", return_value=publisher),
            mock.patch.object(ota_server, "get_appcast_path", return_value=self.output),
        ):
            reused = module._reuse_live_release(self.ctx)

        self.assertTrue(reused)
        self.assertEqual(self.output.read_text(), live)
        payloads = self.registry.add.call_args_list[0].args[1]
        self.assertEqual([item.platform for item in payloads], ["darwin_arm64"])

    def test_refuses_to_replace_incomplete_same_version_live_payloads(self):
        live = render_server_appcast(
            server_feed("browseros-server", "alpha"),
            "0.0.9",
            [],
            ExistingAppcast(
                version="0.0.9",
                pub_date="Wed, 05 Aug 2026 12:00:00 +0000",
                artifacts={},
            ),
        )
        module = ServerOTAModule(
            version="0.0.9",
            platform_filter="darwin_arm64",
            release_sha=SOURCE_SHA,
        )
        publisher = SimpleNamespace(fetch_live=lambda _: live)

        with mock.patch.object(ota_server, "FeedPublisher", return_value=publisher):
            with self.assertRaisesRegex(RuntimeError, "missing same-version"):
                module._reuse_live_release(self.ctx)

    def test_execute_verifies_source_binding_before_live_reuse(self):
        module = ServerOTAModule(
            version="0.0.9",
            platform_filter="linux_x64",
            release_sha=SOURCE_SHA,
        )
        client = object()
        pin = ResourcePin("browseros_server", "0.0.9", ())

        with (
            mock.patch.object(ota_server, "get_r2_client", return_value=client),
            mock.patch.object(
                ota_server,
                "verify_prepared_resource_pin",
                return_value=pin,
            ) as verify,
            mock.patch.object(
                module, "_reuse_live_release", return_value=True
            ) as reuse,
            mock.patch.object(module, "_download_artifacts") as download,
        ):
            module.execute(self.ctx)

        verify.assert_called_once_with(
            client,
            "browseros",
            "browseros_server",
            "0.0.9",
            SOURCE_SHA,
        )
        reuse.assert_called_once_with(self.ctx)
        download.assert_not_called()


class ImmutableSourceDownloadTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        self.archive = self.root / "source.zip"
        server = b"server"
        metadata = {
            "files": [
                {
                    "path": "resources/bin/browseros-server",
                    "sha256": hashlib.sha256(server).hexdigest(),
                    "size": len(server),
                }
            ]
        }
        with zipfile.ZipFile(self.archive, "w") as output:
            output.writestr("artifact-metadata.json", json.dumps(metadata))
            output.writestr("resources/bin/browseros-server", server)
        self.data = self.archive.read_bytes()
        self.key = "artifacts/server/0.0.9/browseros-server-resources-linux-x64.zip"
        self.pin = ResourcePin(
            "browseros_server",
            "0.0.9",
            (
                ResourceObjectPin(
                    target="linux-x64",
                    key=self.key,
                    etag='"etag"',
                    size=len(self.data),
                    sha256=hashlib.sha256(self.data).hexdigest(),
                    release_sha=SOURCE_SHA,
                ),
            ),
        )
        self.ctx = cast(
            Context,
            SimpleNamespace(env=SimpleNamespace(r2_bucket="browseros")),
        )
        self.module = ServerOTAModule(
            version="0.0.9",
            platform_filter="linux_x64",
            release_sha=SOURCE_SHA,
        )

    def test_downloads_the_pinned_versioned_object_and_checks_checksum(self):
        def download(_client, key, destination, bucket):
            self.assertEqual(key, self.key)
            self.assertEqual(bucket, "browseros")
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(self.data)
            return True

        destination = self.root / "download"
        with mock.patch.object(
            ota_server, "download_file_from_r2", side_effect=download
        ):
            self.module._download_artifacts(
                self.ctx,
                destination,
                object(),
                self.pin,
            )

        self.assertEqual(
            (destination / "linux-x64/resources/bin/browseros-server").read_bytes(),
            b"server",
        )

    def test_refuses_download_whose_bytes_do_not_match_the_binding(self):
        bad_pin = ResourcePin(
            self.pin.name,
            self.pin.version,
            (
                ResourceObjectPin(
                    target="linux-x64",
                    key=self.key,
                    etag='"etag"',
                    size=len(self.data),
                    sha256="0" * 64,
                    release_sha=SOURCE_SHA,
                ),
            ),
        )

        def download(_client, _key, destination, _bucket):
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(self.data)
            return True

        with mock.patch.object(
            ota_server, "download_file_from_r2", side_effect=download
        ):
            with self.assertRaisesRegex(RuntimeError, "checksum mismatch"):
                self.module._download_artifacts(
                    self.ctx,
                    self.root / "bad-download",
                    object(),
                    bad_pin,
                )


class BoundPayloadUploadTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        self.path = self.root / "browseros_server_0.0.9_linux_x64.zip"
        self.path.write_bytes(b"signed-payload")
        self.artifact = SignedArtifact(
            platform="linux_x64",
            zip_path=self.path,
            signature="local-signature",
            length=self.path.stat().st_size,
            os="linux",
            arch="x86_64",
        )
        self.module = ServerOTAModule(
            version="0.0.9",
            release_sha=SOURCE_SHA,
        )
        self.ctx = cast(
            Context,
            SimpleNamespace(env=SimpleNamespace(r2_bucket="browseros")),
        )

    def _metadata(self, data: bytes, signature: str) -> dict[str, str]:
        return {
            "binding-schema": "browseros-server-ota-v1",
            "bundle-id": "browseros-server",
            "version": "0.0.9",
            "release-sha": SOURCE_SHA,
            "platform": "linux_x64",
            "os": "linux",
            "arch": "x86_64",
            "sha256": hashlib.sha256(data).hexdigest(),
            "sparkle-signature": signature,
            "length": str(len(data)),
        }

    def test_creates_payload_with_a_write_once_binding(self):
        client = SimpleNamespace(put_object=mock.Mock())

        result = self.module._upload_bound_payload(
            self.ctx,
            client,
            self.artifact,
        )

        self.assertEqual(result, self.artifact)
        request = client.put_object.call_args.kwargs
        self.assertEqual(request["IfNoneMatch"], "*")
        self.assertEqual(
            request["Metadata"], self._metadata(b"signed-payload", "local-signature")
        )

    def test_reuses_the_canonical_payload_after_a_write_race(self):
        canonical = b"canonical-payload"
        client = SimpleNamespace(
            put_object=mock.Mock(side_effect=_PreconditionFailure()),
            get_object=mock.Mock(
                return_value={
                    "Body": io.BytesIO(canonical),
                    "Metadata": self._metadata(canonical, "canonical-signature"),
                }
            ),
        )

        result = self.module._upload_bound_payload(
            self.ctx,
            client,
            self.artifact,
        )

        self.assertEqual(result.signature, "canonical-signature")
        self.assertEqual(result.length, len(canonical))
        self.assertEqual(result.zip_path.name, self.path.name)

    def test_rejects_a_colliding_payload_with_another_binding(self):
        canonical = b"canonical-payload"
        metadata = self._metadata(canonical, "canonical-signature")
        metadata["release-sha"] = "b" * 40
        client = SimpleNamespace(
            put_object=mock.Mock(side_effect=_PreconditionFailure()),
            get_object=mock.Mock(
                return_value={
                    "Body": io.BytesIO(canonical),
                    "Metadata": metadata,
                }
            ),
        )

        with self.assertRaisesRegex(RuntimeError, "binding mismatch"):
            self.module._upload_bound_payload(
                self.ctx,
                client,
                self.artifact,
            )

    def test_appcast_uses_the_canonical_payload_after_collision(self):
        canonical = SignedArtifact(
            platform="linux_x64",
            zip_path=Path(self.path.name),
            signature="canonical-signature",
            length=999,
            os="linux",
            arch="x86_64",
        )
        output = self.root / "appcast-server.alpha.xml"
        publisher = SimpleNamespace(fetch_live=lambda _: None)
        ctx = cast(
            Context,
            SimpleNamespace(
                env=SimpleNamespace(r2_bucket="browseros"),
                artifact_registry=SimpleNamespace(add=mock.Mock()),
            ),
        )

        with (
            mock.patch.object(ota_server, "get_r2_client", return_value=object()),
            mock.patch.object(
                self.module,
                "_upload_bound_payload",
                return_value=canonical,
            ),
            mock.patch.object(ota_server, "FeedPublisher", return_value=publisher),
            mock.patch.object(ota_server, "get_appcast_path", return_value=output),
        ):
            self.module._finalize_release(ctx, [self.artifact])

        content = output.read_text()
        self.assertIn('sparkle:edSignature="canonical-signature"', content)
        self.assertIn('length="999"', content)
        self.assertNotIn("local-signature", content)


if __name__ == "__main__":
    unittest.main()
