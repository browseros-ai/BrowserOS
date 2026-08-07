#!/usr/bin/env python3
"""Tests for the ext release orchestration module and pipeline assembly."""

import hashlib
import io
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from typing import cast
from unittest.mock import MagicMock, patch

from ...core.context import Context
from ...core.step import ValidationError
from ..feeds.render import render_update_manifest
from .release import (
    ExtensionReleaseModule,
    _validate_manifest_update_url,
    build_pipeline,
    increment_extension_version,
    materialize_bound_extension_crx,
    normalize_extension_version,
    resolve_extension_version,
    upload_bound_extension_crx,
    verify_versioned_crx_objects,
)
from .specs import spec_by_name

MODULE = "bos_build.release.extensions.release"

ALL_KEYS = {
    "BROWSEROS_AGENT_V2_KEY": "agent-pem",
    "BROWSEROS_CONTROLLER_KEY": "controller-pem",
    "BUGREPORTER_KEY": "bugreporter-pem",
    "BROWSERCLAW_KEY": "claw-pem",
    "VITE_CLAW_POSTHOG_KEY": "phc-claw-app",
}

SOURCE_SHA = "a" * 40


class MissingObjectError(Exception):
    def __init__(self):
        self.response = {
            "Error": {"Code": "NoSuchKey"},
            "ResponseMetadata": {"HTTPStatusCode": 404},
        }


class PreconditionFailedError(Exception):
    def __init__(self):
        self.response = {
            "Error": {"Code": "PreconditionFailed"},
            "ResponseMetadata": {"HTTPStatusCode": 412},
        }


def _binding(name: str, version: str, source_sha: str, data: bytes) -> dict:
    return {
        "binding-schema": "browseros-extension-crx-v1",
        "extension": name,
        "version": version,
        "source-sha": source_sha,
        "sha256": hashlib.sha256(data).hexdigest(),
    }


class BoundCrxClient:
    def __init__(self):
        self.objects = {}
        self.gets = []
        self.puts = []

    def get_object(self, *, Bucket, Key):
        self.gets.append((Bucket, Key))
        if Key not in self.objects:
            raise MissingObjectError()
        data, metadata = self.objects[Key]
        return {"Body": io.BytesIO(data), "Metadata": dict(metadata)}

    def put_object(self, **kwargs):
        self.puts.append(kwargs)
        key = kwargs["Key"]
        if key in self.objects:
            raise PreconditionFailedError()
        self.objects[key] = (kwargs["Body"], dict(kwargs["Metadata"]))


class RacingBoundCrxClient(BoundCrxClient):
    def __init__(self, canonical: bytes, metadata: dict):
        super().__init__()
        self.canonical = canonical
        self.metadata = metadata

    def put_object(self, **kwargs):
        self.puts.append(kwargs)
        self.objects[kwargs["Key"]] = (self.canonical, dict(self.metadata))
        raise PreconditionFailedError()


class ExtensionVersionResolutionTest(unittest.TestCase):
    def test_normalizes_and_increments_four_part_versions(self) -> None:
        self.assertEqual(normalize_extension_version("0.1.6"), "0.1.6.0")
        self.assertEqual(increment_extension_version("0.1.6.9"), "0.1.7.0")
        self.assertEqual(increment_extension_version("0.0.123.0"), "0.0.124.0")

    def test_allocates_above_tag_and_all_live_manifest_versions(self) -> None:
        records = [
            {
                "tag_name": "ext-browserclaw/v0.1.6.0",
                "release_sha": "older-sha",
            }
        ]
        manifests = [
            render_update_manifest({"browserclaw": "0.1.5.0"}),
            render_update_manifest({"browserclaw": "0.1.7.3"}),
            render_update_manifest({"browserclaw": "0.1.4.0"}),
        ]

        self.assertEqual(
            resolve_extension_version(
                extension="browserclaw",
                requested_version="",
                release_sha="new-sha",
                release_records=records,
                manifest_contents=manifests,
            ),
            "0.1.8.0",
        )

    def test_same_source_reuses_highest_matching_tag_or_release(self) -> None:
        records = [
            {
                "tag_name": "ext-browserclaw/v0.1.6.0",
                "target_commitish": "same-sha",
                "tag_object": "tag",
            },
            {
                "tag_name": "ext-browserclaw/v0.1.7.0",
                "release_sha": "same-sha",
                "tag_object": "tag",
            },
            {
                "tag_name": "ext-browserclaw/v0.1.9.0",
                "release_sha": "other-sha",
                "tag_object": "tag",
            },
        ]

        self.assertEqual(
            resolve_extension_version(
                extension="browserclaw",
                requested_version="",
                release_sha="same-sha",
                release_records=records,
                manifest_contents=[render_update_manifest({"browserclaw": "0.1.8.0"})],
            ),
            "0.1.7.0",
        )

    def test_private_draft_without_a_tag_is_reusable(self) -> None:
        self.assertEqual(
            resolve_extension_version(
                extension="browserclaw",
                requested_version="",
                release_sha="same-sha",
                release_records=[
                    {
                        "tag_name": "ext-browserclaw/v0.1.7.0",
                        "draft": True,
                        "target_commitish": "same-sha",
                        "tag_object": "missing",
                    }
                ],
                manifest_contents=[render_update_manifest({"browserclaw": "0.1.6.0"})],
            ),
            "0.1.7.0",
        )

    def test_historical_lightweight_same_source_allocates_above_history(self) -> None:
        records = [
            {
                "tag_name": "ext-browserclaw/v0.1.7.0",
                "draft": False,
                "target_commitish": "main",
                "tag_object": "commit",
                "tag_release_sha": "same-sha",
            },
            {
                "tag_name": "ext-browserclaw/v0.1.7.0",
                "release_sha": "same-sha",
                "tag_object": "commit",
            },
        ]

        self.assertEqual(
            resolve_extension_version(
                extension="browserclaw",
                requested_version="",
                release_sha="same-sha",
                release_records=records,
                manifest_contents=[render_update_manifest({"browserclaw": "0.1.7.0"})],
            ),
            "0.1.8.0",
        )

    def test_private_draft_with_lightweight_tag_is_not_reusable(self) -> None:
        self.assertEqual(
            resolve_extension_version(
                extension="browserclaw",
                requested_version="",
                release_sha="same-sha",
                release_records=[
                    {
                        "tag_name": "ext-browserclaw/v0.1.7.0",
                        "draft": True,
                        "target_commitish": "same-sha",
                        "tag_object": "commit",
                        "tag_release_sha": "same-sha",
                    }
                ],
                manifest_contents=[],
            ),
            "0.1.8.0",
        )

    def test_different_source_allocates_above_global_maximum(self) -> None:
        self.assertEqual(
            resolve_extension_version(
                extension="agent",
                requested_version="",
                release_sha="new-sha",
                release_records=[
                    {
                        "tag_name": "ext-agent/v0.0.123.0",
                        "release_sha": "old-sha",
                    }
                ],
                manifest_contents=[render_update_manifest({"agent": "0.0.124.4"})],
            ),
            "0.0.125.0",
        )

    def test_committed_manifest_version_is_an_allocation_floor(self) -> None:
        self.assertEqual(
            resolve_extension_version(
                extension="agent",
                requested_version="",
                release_sha="new-sha",
                release_records=[],
                manifest_contents=[],
                committed_version="0.0.130",
            ),
            "0.0.130.0",
        )

    def test_malformed_historical_versions_are_ignored(self) -> None:
        self.assertEqual(
            resolve_extension_version(
                extension="agent",
                requested_version="",
                release_sha="new-sha",
                release_records=[
                    {
                        "tag_name": "ext-agent/vnot-a-version",
                        "release_sha": "old-sha",
                    },
                    {
                        "tag_name": "ext-agent/v0.0.123.0",
                        "release_sha": "old-sha",
                    },
                ],
                manifest_contents=[],
            ),
            "0.0.124.0",
        )

    def test_explicit_version_is_validated_and_normalized(self) -> None:
        self.assertEqual(
            resolve_extension_version(
                extension="controller",
                requested_version="2.3.4",
                release_sha="source",
                release_records=[],
                manifest_contents=[],
            ),
            "2.3.4.0",
        )
        for version in ("", "1.2.3.4.5", "1.2.beta", "1..2"):
            with (
                self.subTest(version=version),
                self.assertRaisesRegex(ValueError, "version"),
            ):
                normalize_extension_version(version)

    def test_all_and_external_extensions_refuse_automatic_versions(self) -> None:
        for extension in ("all", "controller", "bugreporter"):
            with (
                self.subTest(extension=extension),
                self.assertRaisesRegex(ValueError, "explicit version"),
            ):
                resolve_extension_version(
                    extension=extension,
                    requested_version="",
                    release_sha="source",
                    release_records=[],
                    manifest_contents=[],
                )


class BoundCrxTest(unittest.TestCase):
    def setUp(self) -> None:
        self.spec = spec_by_name("browserclaw")
        self.version = "0.1.8.0"
        self.key = self.spec.crx_key(self.version)

    def test_absent_crx_uploads_with_immutable_source_binding(self) -> None:
        client = BoundCrxClient()

        with tempfile.TemporaryDirectory() as temp_dir:
            path = Path(temp_dir) / self.spec.crx_filename(self.version)
            path.write_bytes(b"first-build")

            result = upload_bound_extension_crx(
                client,
                "browseros",
                self.spec,
                self.version,
                SOURCE_SHA,
                path,
            )

        self.assertEqual(result, "uploaded")
        self.assertEqual(len(client.puts), 1)
        self.assertEqual(
            client.puts[0],
            {
                "Bucket": "browseros",
                "Key": self.key,
                "Body": b"first-build",
                "ContentType": "application/x-chrome-extension",
                "Metadata": _binding(
                    "browserclaw", self.version, SOURCE_SHA, b"first-build"
                ),
                "IfNoneMatch": "*",
            },
        )

    def test_post_upload_retry_materializes_canonical_attachment(self) -> None:
        client = BoundCrxClient()
        canonical = b"first-build"

        with tempfile.TemporaryDirectory() as temp_dir:
            path = Path(temp_dir) / self.spec.crx_filename(self.version)
            path.write_bytes(canonical)
            self.assertEqual(
                upload_bound_extension_crx(
                    client,
                    "browseros",
                    self.spec,
                    self.version,
                    SOURCE_SHA,
                    path,
                ),
                "uploaded",
            )
            path.write_bytes(b"mtime-changed-rebuild")

            reused = materialize_bound_extension_crx(
                client,
                "browseros",
                self.spec,
                self.version,
                SOURCE_SHA,
                path,
            )

            self.assertTrue(reused)
            self.assertEqual(path.read_bytes(), canonical)
        self.assertEqual(len(client.puts), 1)

    def test_missing_or_mismatched_binding_fails_closed(self) -> None:
        for metadata in (
            {},
            _binding("browserclaw", self.version, "b" * 40, b"canonical"),
            _binding("agent", self.version, SOURCE_SHA, b"canonical"),
        ):
            with self.subTest(metadata=metadata):
                client = BoundCrxClient()
                client.objects[self.key] = (b"canonical", metadata)
                with tempfile.TemporaryDirectory() as temp_dir:
                    path = Path(temp_dir) / self.spec.crx_filename(self.version)
                    with self.assertRaisesRegex(RuntimeError, "binding"):
                        materialize_bound_extension_crx(
                            client,
                            "browseros",
                            self.spec,
                            self.version,
                            SOURCE_SHA,
                            path,
                        )

    def test_concurrent_creator_supplies_canonical_retry_asset(self) -> None:
        canonical = b"concurrent-build"
        client = RacingBoundCrxClient(
            canonical,
            _binding("browserclaw", self.version, SOURCE_SHA, canonical),
        )

        with tempfile.TemporaryDirectory() as temp_dir:
            path = Path(temp_dir) / self.spec.crx_filename(self.version)
            path.write_bytes(b"our-mtime-dependent-build")

            result = upload_bound_extension_crx(
                client,
                "browseros",
                self.spec,
                self.version,
                SOURCE_SHA,
                path,
            )

            self.assertEqual(result, "reused")
            self.assertEqual(path.read_bytes(), canonical)
        self.assertEqual(len(client.puts), 1)


class VerifyVersionedObjectsTest(unittest.TestCase):
    def test_checks_every_selected_versioned_key(self) -> None:
        client = BoundCrxClient()
        for name in ("agent", "browserclaw"):
            spec = spec_by_name(name)
            key = spec.crx_key("0.1.7.0")
            data = f"{name}-crx".encode()
            client.objects[key] = (
                data,
                _binding(name, "0.1.7.0", SOURCE_SHA, data),
            )

        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir)
            keys = verify_versioned_crx_objects(
                client,
                "browseros",
                "0.1.7.0",
                ("agent", "browserclaw"),
                SOURCE_SHA,
                output_dir,
            )

            self.assertEqual(
                (output_dir / "agent-0.1.7.0.crx").read_bytes(),
                b"agent-crx",
            )
            self.assertEqual(
                (output_dir / "browserclaw-0.1.7.0.crx").read_bytes(),
                b"browserclaw-crx",
            )

        self.assertEqual(
            keys,
            (
                "extensions/agent-0.1.7.0.crx",
                "extensions/browserclaw-0.1.7.0.crx",
            ),
        )
        self.assertEqual(
            client.gets,
            [
                ("browseros", "extensions/agent-0.1.7.0.crx"),
                ("browseros", "extensions/browserclaw-0.1.7.0.crx"),
            ],
        )

    def test_missing_object_fails(self) -> None:
        client = BoundCrxClient()

        with self.assertRaisesRegex(RuntimeError, "agent-0.1.7.0.crx"):
            verify_versioned_crx_objects(
                client,
                "browseros",
                "0.1.7.0",
                ("agent",),
                SOURCE_SHA,
            )

    def test_mismatched_source_binding_fails(self) -> None:
        client = BoundCrxClient()
        spec = spec_by_name("browserclaw")
        key = spec.crx_key("0.1.7.0")
        client.objects[key] = (
            b"canonical",
            _binding("browserclaw", "0.1.7.0", "b" * 40, b"canonical"),
        )

        with self.assertRaisesRegex(RuntimeError, "binding mismatch"):
            verify_versioned_crx_objects(
                client,
                "browseros",
                "0.1.7.0",
                ("browserclaw",),
                SOURCE_SHA,
            )


def _ctx(has_r2=True, bucket="browseros"):
    return cast(
        Context,
        SimpleNamespace(
            env=SimpleNamespace(has_r2_config=lambda: has_r2, r2_bucket=bucket)
        ),
    )


class BuildPipelineTest(unittest.TestCase):
    def test_unknown_name_raises_with_valid_names(self):
        with self.assertRaisesRegex(ValueError, "bugreporter"):
            build_pipeline(
                version="1.0.0",
                name="agent-v2",
                branch=None,
                chrome_binary=None,
                source_sha=SOURCE_SHA,
            )

    def test_named_extension_gets_one_release_step(self):
        steps = build_pipeline(
            version="1.2.3",
            name="agent",
            branch=None,
            chrome_binary=None,
            source_sha=SOURCE_SHA,
        )
        self.assertEqual(len(steps), 1)
        release = steps[0]
        self.assertIsInstance(release, ExtensionReleaseModule)
        self.assertEqual(release.names, ("agent",))

    def test_all_extensions_get_one_release_step(self):
        steps = build_pipeline(
            version="2.0.0",
            name=None,
            branch=None,
            chrome_binary=None,
            source_sha=SOURCE_SHA,
        )
        self.assertEqual(len(steps), 1)
        release = steps[0]
        self.assertIsInstance(release, ExtensionReleaseModule)
        self.assertEqual(
            release.names, ("agent", "controller", "bugreporter", "browserclaw")
        )

    def test_dash_prefixed_branch_rejected_at_assembly(self):
        with self.assertRaisesRegex(ValueError, "branch"):
            build_pipeline(
                version="1.0.0",
                name="controller",
                branch="--upload-pack=/bin/sh",
                chrome_binary=None,
                source_sha=SOURCE_SHA,
            )

    def test_malformed_version_rejected_at_assembly(self):
        for version in ("1.0.0-beta", "abc", "", "1..2"):
            with self.assertRaisesRegex(ValueError, "version"):
                build_pipeline(
                    version=version,
                    name="agent",
                    branch=None,
                    chrome_binary=None,
                    source_sha=SOURCE_SHA,
                )

    def test_non_commit_source_sha_rejected_at_assembly(self):
        with self.assertRaisesRegex(ValueError, "source commit SHA"):
            build_pipeline(
                version="1.0.0",
                name="agent",
                branch=None,
                chrome_binary=None,
                source_sha="main",
            )


class ValidateTest(unittest.TestCase):
    def _module(self, names=("agent",), **kwargs):
        return ExtensionReleaseModule(
            version="1.0.0", names=names, source_sha=SOURCE_SHA, **kwargs
        )

    def test_missing_signing_key_env_names_the_variable(self):
        with patch.dict("os.environ", {}, clear=False):
            import os

            os.environ.pop("BROWSEROS_AGENT_V2_KEY", None)
            with patch(f"{MODULE}.find_chrome_binary", return_value="chrome"):
                with self.assertRaisesRegex(ValidationError, "BROWSEROS_AGENT_V2_KEY"):
                    self._module().validate(_ctx())

    def test_missing_r2_config_fails(self):
        with patch.dict("os.environ", ALL_KEYS):
            with patch(f"{MODULE}.find_chrome_binary", return_value="chrome"):
                with self.assertRaisesRegex(ValidationError, "R2"):
                    self._module().validate(_ctx(has_r2=False))

    def test_missing_browserclaw_build_env_names_the_variable(self):
        env = dict(ALL_KEYS)
        env.pop("VITE_CLAW_POSTHOG_KEY")
        with patch.dict("os.environ", env, clear=True):
            with patch(f"{MODULE}.find_chrome_binary", return_value="chrome"):
                with self.assertRaisesRegex(ValidationError, "VITE_CLAW_POSTHOG_KEY"):
                    self._module(names=("browserclaw",)).validate(_ctx())

    def test_chrome_resolution_failure_fails_validation(self):
        with patch.dict("os.environ", ALL_KEYS):
            with patch(
                f"{MODULE}.find_chrome_binary",
                side_effect=RuntimeError("no chrome anywhere"),
            ):
                with self.assertRaisesRegex(ValidationError, "no chrome"):
                    self._module().validate(_ctx())

    def test_unknown_name_fails_validation(self):
        with self.assertRaisesRegex(ValidationError, "Unknown extension"):
            self._module(names=("nope",)).validate(_ctx())


class ManifestUpdateUrlTest(unittest.TestCase):
    def setUp(self):
        self.dist_path = Path("apps/claw-app/dist/chrome-mv3")
        self.expected_url = "https://cdn.browseros.com/extensions/update-manifest.xml"

    def test_in_feed_extension_without_update_url_fails(self):
        with self.assertRaisesRegex(
            RuntimeError, "browserclaw.*apps/claw-app/dist/chrome-mv3"
        ):
            _validate_manifest_update_url(
                spec_by_name("browserclaw"), {}, self.dist_path
            )

    def test_in_feed_extension_with_wrong_update_url_fails(self):
        with self.assertRaisesRegex(
            RuntimeError, "browserclaw.*apps/claw-app/dist/chrome-mv3"
        ):
            _validate_manifest_update_url(
                spec_by_name("browserclaw"),
                {"update_url": "https://example.com/update.xml"},
                self.dist_path,
            )

    def test_in_feed_extension_with_expected_update_url_passes(self):
        _validate_manifest_update_url(
            spec_by_name("browserclaw"),
            {"update_url": self.expected_url},
            self.dist_path,
        )

    def test_controller_without_update_url_passes(self):
        _validate_manifest_update_url(
            spec_by_name("controller"), {}, Path("apps/controller-ext/dist")
        )


class ExecuteTest(unittest.TestCase):
    def setUp(self):
        self.monorepo = Path("/mono")
        self.work = Path("/work")

        patches = {
            "build_extension_crx": MagicMock(),
            "materialize_bound_extension_crx": MagicMock(return_value=False),
            "upload_bound_extension_crx": MagicMock(return_value="uploaded"),
            "get_r2_client": MagicMock(return_value="r2-client"),
            "find_chrome_binary": MagicMock(return_value="chrome-bin"),
        }
        self.mocks = {}
        self.tracker = MagicMock()
        for name, mock in patches.items():
            patcher = patch(f"{MODULE}.{name}", mock)
            patcher.start()
            self.addCleanup(patcher.stop)
            self.mocks[name] = mock
            self.tracker.attach_mock(mock, name)

        env_patcher = patch.dict("os.environ", ALL_KEYS)
        env_patcher.start()
        self.addCleanup(env_patcher.stop)

    def _module(self, names, **kwargs):
        return ExtensionReleaseModule(
            version="1.0.0",
            names=names,
            source_sha=SOURCE_SHA,
            monorepo_root=self.monorepo,
            work_root=self.work,
            **kwargs,
        )

    def test_agent_flow_order_and_arguments(self):
        self._module(("agent",)).execute(_ctx())

        called = [c[0] for c in self.tracker.mock_calls]
        self.assertEqual(
            called,
            [
                "get_r2_client",
                "find_chrome_binary",
                "materialize_bound_extension_crx",
                "build_extension_crx",
                "upload_bound_extension_crx",
            ],
        )

        build_kwargs = self.mocks["build_extension_crx"].call_args.kwargs
        self.assertEqual(build_kwargs["spec"], spec_by_name("agent"))
        self.assertEqual(build_kwargs["version"], "1.0.0")
        self.assertEqual(
            build_kwargs["output_path"], self.work / "dist" / "agent-1.0.0.crx"
        )
        self.assertEqual(build_kwargs["monorepo_root"], self.monorepo)
        self.assertEqual(build_kwargs["work_root"], self.work)
        self.assertIsNone(build_kwargs["branch_override"])
        self.assertEqual(build_kwargs["chrome_binary"], "chrome-bin")
        self.assertTrue(build_kwargs["stamp_version"])

        self.mocks["upload_bound_extension_crx"].assert_called_once_with(
            "r2-client",
            "browseros",
            spec_by_name("agent"),
            "1.0.0",
            SOURCE_SHA,
            self.work / "dist" / "agent-1.0.0.crx",
        )

    def test_external_extension_uses_shared_builder(self):
        self._module(("bugreporter",)).execute(_ctx())
        build_kwargs = self.mocks["build_extension_crx"].call_args.kwargs
        self.assertEqual(build_kwargs["spec"], spec_by_name("bugreporter"))

    def test_upload_failure_stops_later_extensions(self):
        self.mocks["upload_bound_extension_crx"].side_effect = RuntimeError(
            "binding mismatch for extensions/agent-1.0.0.crx"
        )
        with self.assertRaisesRegex(RuntimeError, "extensions/agent-1.0.0.crx"):
            self._module(("agent", "bugreporter")).execute(_ctx())

        built = [
            call.kwargs["spec"].name
            for call in self.mocks["build_extension_crx"].call_args_list
        ]
        self.assertEqual(built, ["agent"])

    def test_branch_override_reaches_resolver(self):
        self._module(("bugreporter",), branch_override="canary").execute(_ctx())
        self.assertEqual(
            self.mocks["build_extension_crx"].call_args.kwargs["branch_override"],
            "canary",
        )

    def test_injected_r2_client_skips_factory(self):
        self._module(("agent",), r2_client="prebuilt").execute(_ctx())
        self.mocks["get_r2_client"].assert_not_called()
        self.assertEqual(
            self.mocks["upload_bound_extension_crx"].call_args.args[0], "prebuilt"
        )

    def test_retry_materializes_canonical_crx_without_rebuilding(self):
        self.mocks["materialize_bound_extension_crx"].return_value = True

        self._module(("browserclaw",)).execute(_ctx())

        self.mocks["build_extension_crx"].assert_not_called()
        self.mocks["upload_bound_extension_crx"].assert_not_called()
        self.mocks["materialize_bound_extension_crx"].assert_called_once_with(
            "r2-client",
            "browseros",
            spec_by_name("browserclaw"),
            "1.0.0",
            SOURCE_SHA,
            self.work / "dist" / "browserclaw-1.0.0.crx",
        )


if __name__ == "__main__":
    unittest.main()
