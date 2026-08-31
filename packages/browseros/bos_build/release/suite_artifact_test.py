#!/usr/bin/env python3
"""Immutable suite browser artifact publication tests."""

import hashlib
import io
import json
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path

from bos_build.release.suite_artifact import (
    R2ImmutableObjectBackend,
    publish_suite_browser_artifact,
)
from bos_build.release.suite_test import suite_record


class _PreconditionFailed(Exception):
    response = {
        "Error": {"Code": "PreconditionFailed"},
        "ResponseMetadata": {"HTTPStatusCode": 412},
    }


class _FakeR2Client:
    def __init__(self) -> None:
        self.objects: dict[str, tuple[bytes, dict[str, str]]] = {}
        self.puts: list[str] = []

    def put_object(self, *, Bucket, Key, Body, Metadata, IfNoneMatch, **kwargs):
        del Bucket, kwargs
        if IfNoneMatch != "*":
            raise AssertionError("immutable upload requires IfNoneMatch")
        if Key in self.objects:
            raise _PreconditionFailed
        self.objects[Key] = (bytes(Body), dict(Metadata))
        self.puts.append(Key)

    def get_object(self, *, Bucket, Key):
        del Bucket
        content, metadata = self.objects[Key]
        return {"Body": io.BytesIO(content), "Metadata": dict(metadata)}


def _write_artifact(root: Path, *, product: str = "browseros") -> None:
    record = suite_record()
    filename = (
        f"BrowserOS_v{record.browser_version}_arm64.dmg"
        if product == "browseros"
        else f"BrowserOS_neo_v{record.browser_version}_arm64.dmg"
    )
    content = f"signed-{product}".encode()
    components = (
        {
            "server": record.component_versions["server"],
            "agent": record.component_versions["agent"],
            "app-onboard": record.component_versions["app-onboard"],
        }
        if product == "browseros"
        else {
            "claw-server-rust": record.component_versions["claw-server-rust"],
            "browserclaw": record.component_versions["browserclaw"],
            "claw-onboard": record.component_versions["claw-onboard"],
        }
    )
    (root / filename).write_bytes(content)
    (root / "release.json").write_text(
        json.dumps(
            {
                "product": product,
                "platform": "macos",
                "version": record.browser_version,
                "source_sha": record.source_sha,
                "reservation_sha": record.reservation_sha,
                "component_versions": components,
                "artifacts": {
                    "arm64": {
                        "filename": filename,
                        "size": len(content),
                        "sha256": hashlib.sha256(content).hexdigest(),
                        "sparkle_signature": "signature",
                        "sparkle_length": len(content),
                    }
                },
            },
            indent=2,
        ),
        encoding="utf-8",
    )


class SuiteArtifactPublicationTest(unittest.TestCase):
    def test_identical_retry_verifies_objects_without_overwriting(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            _write_artifact(root)
            client = _FakeR2Client()
            backend = R2ImmutableObjectBackend(client, "bucket")
            record = replace(suite_record(state="merged"), draft=False)

            first = publish_suite_browser_artifact(record, "browseros", root, backend)
            second = publish_suite_browser_artifact(record, "browseros", root, backend)

        self.assertEqual(first, second)
        self.assertEqual(client.puts, [first.artifact_key, first.receipt_key])

    def test_conflicting_versioned_bytes_fail_without_overwrite(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            _write_artifact(root)
            client = _FakeR2Client()
            backend = R2ImmutableObjectBackend(client, "bucket")
            record = replace(suite_record(state="merged"), draft=False)
            publication = publish_suite_browser_artifact(
                record, "browseros", root, backend
            )
            original = client.objects[publication.artifact_key]
            client.objects[publication.artifact_key] = (
                b"conflicting-public-bytes",
                original[1],
            )

            with self.assertRaisesRegex(RuntimeError, "conflicts"):
                publish_suite_browser_artifact(record, "browseros", root, backend)

        self.assertEqual(
            client.puts, [publication.artifact_key, publication.receipt_key]
        )

    def test_receipt_must_match_merged_transaction(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            _write_artifact(root, product="browserclaw")
            record = replace(suite_record(state="merged"), draft=False)
            receipt_path = root / "release.json"
            receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
            receipt["reservation_sha"] = "f" * 40
            receipt_path.write_text(json.dumps(receipt), encoding="utf-8")

            with self.assertRaisesRegex(ValueError, "reservation_sha"):
                publish_suite_browser_artifact(
                    record,
                    "browserclaw",
                    root,
                    R2ImmutableObjectBackend(_FakeR2Client(), "bucket"),
                )


if __name__ == "__main__":
    unittest.main()
