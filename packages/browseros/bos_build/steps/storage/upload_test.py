#!/usr/bin/env python3
"""Tests for release artifact upload metadata helpers."""

import unittest
import tempfile
import hashlib
import json
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from bos_build.core.context import ArtifactRegistry
from bos_build.release.prepared_resources import PreparedResourcesManifest
from bos_build.steps.package.linux_packaging import LinuxPackagingError
from bos_build.steps.storage.upload import (
    _get_artifact_key,
    generate_release_json,
    merge_release_metadata,
    upload_release_artifacts,
)


def _upload_ctx(
    dist_dir: Path,
    *,
    product_id: str = "browseros",
    display_name: str = "BrowserOS",
    artifact_prefix: str = "BrowserOS",
) -> SimpleNamespace:
    return SimpleNamespace(
        env=SimpleNamespace(
            onboarding_resource_version=None,
            browserclaw_server_resource_version=None,
            browseros_server_resource_version=None,
            bundled_product_extension_version=None,
            r2_bucket="browseros",
            r2_cdn_base_url="https://cdn.browseros.com",
            has_r2_config=lambda: True,
        ),
        artifact_registry=ArtifactRegistry(),
        product=SimpleNamespace(
            id=product_id,
            display_name=display_name,
            artifact_prefix=artifact_prefix,
        ),
        architecture="x64",
        chromium_version="136.0.0.0",
        browseros_chromium_version="136.0.0.0.1",
        get_dist_dir=lambda: dist_dir,
        get_semantic_version=lambda: "1.2.3",
        get_artifact_name=lambda kind: (
            f"{artifact_prefix}_v1.2.3_x64.AppImage"
            if kind == "appimage"
            else f"{artifact_prefix}_v1.2.3_amd64.deb"
        ),
        get_sparkle_version=lambda: "10000.1.2.3",
        get_release_path=lambda platform: (f"releases/{product_id}/1.2.3/{platform}/"),
    )


class UploadMetadataTest(unittest.TestCase):
    def test_deferred_linux_upload_rejects_partial_registered_pair(self) -> None:
        with (
            tempfile.TemporaryDirectory() as tmp,
            mock.patch("bos_build.steps.storage.upload.IS_MACOS", lambda: False),
            mock.patch("bos_build.steps.storage.upload.IS_WINDOWS", lambda: False),
            mock.patch.dict(
                "os.environ",
                {"BROWSEROS_DEFER_R2_UPLOAD": "1"},
                clear=False,
            ),
        ):
            root = Path(tmp)
            ctx = _upload_ctx(root)
            appimage = root / ctx.get_artifact_name("appimage")
            appimage.write_bytes(b"appimage")
            appimage.chmod(0o755)
            ctx.artifact_registry.add("appimage", appimage)

            with self.assertRaisesRegex(
                LinuxPackagingError,
                "registry is partial",
            ):
                upload_release_artifacts(ctx)

    def test_deferred_linux_upload_uses_only_the_registered_exact_pair(self) -> None:
        with (
            tempfile.TemporaryDirectory() as tmp,
            mock.patch("bos_build.steps.storage.upload.IS_MACOS", lambda: False),
            mock.patch("bos_build.steps.storage.upload.IS_WINDOWS", lambda: False),
            mock.patch.dict(
                "os.environ",
                {"BROWSEROS_DEFER_R2_UPLOAD": "1"},
                clear=False,
            ),
        ):
            root = Path(tmp)
            ctx = _upload_ctx(root)
            appimage = root / ctx.get_artifact_name("appimage")
            deb = root / ctx.get_artifact_name("deb")
            appimage.write_bytes(b"registered-appimage")
            appimage.chmod(0o755)
            deb.write_bytes(b"registered-deb")
            (root / "BrowserOS_v1.2.3_old.AppImage").write_bytes(b"stale")
            (root / "BrowserOS_neo_v1.2.3_x64.AppImage").write_bytes(b"sibling")
            ctx.artifact_registry.add("appimage", appimage)
            ctx.artifact_registry.add("deb", deb)

            success, release = upload_release_artifacts(ctx)

        self.assertTrue(success)
        self.assertEqual(set(release["artifacts"]), {"x64_appimage", "x64_deb"})
        self.assertEqual(
            release["artifacts"]["x64_appimage"]["filename"],
            appimage.name,
        )
        self.assertEqual(release["artifacts"]["x64_deb"]["filename"], deb.name)

    def test_deferred_upload_writes_exact_receipt_without_r2_mutation(self) -> None:
        with (
            tempfile.TemporaryDirectory() as tmp,
            mock.patch("bos_build.steps.storage.upload.BOTO3_AVAILABLE", False),
            mock.patch("bos_build.steps.storage.upload.IS_MACOS", lambda: True),
            mock.patch.dict(
                "os.environ",
                {
                    "BROWSEROS_DEFER_R2_UPLOAD": "1",
                    "BROWSEROS_BUILD_SOURCE_SHA": "a" * 40,
                    "BROWSEROS_BUILD_RESERVATION_SHA": "b" * 40,
                },
                clear=False,
            ),
            mock.patch("bos_build.steps.storage.upload.get_r2_client") as get_client,
            mock.patch(
                "bos_build.steps.storage.upload.upload_file_to_r2"
            ) as upload_file,
        ):
            root = Path(tmp)
            filename = "BrowserOS_v1.2.3_arm64.dmg"
            (root / filename).write_bytes(b"signed-dmg")
            ctx = _upload_ctx(root)

            success, release = upload_release_artifacts(
                ctx,
                {filename: {"sparkle_signature": "SIG==", "sparkle_length": 10}},
            )
            persisted = json.loads((root / "release.json").read_text(encoding="utf-8"))

        self.assertTrue(success)
        self.assertEqual(release["reservation_sha"], "b" * 40)
        self.assertEqual(persisted, release)
        get_client.assert_not_called()
        upload_file.assert_not_called()

    def test_release_json_records_actions_provenance(self) -> None:
        with (
            tempfile.TemporaryDirectory() as tmp,
            mock.patch.dict(
                "os.environ",
                {
                    "GITHUB_SHA": "a" * 40,
                    "GITHUB_RUN_ID": "30418029456",
                    "GITHUB_RUN_ATTEMPT": "2",
                },
                clear=False,
            ),
        ):
            release = generate_release_json(
                _upload_ctx(Path(tmp)),
                [{"filename": "BrowserOS_v1.2.3_x64.AppImage", "size": 12}],
                "linux",
            )

        self.assertEqual(release["source_sha"], "a" * 40)
        self.assertEqual(release["workflow_run_id"], "30418029456")
        self.assertEqual(release["workflow_run_attempt"], "2")

    def test_release_json_uses_explicit_build_source_sha(self) -> None:
        with (
            tempfile.TemporaryDirectory() as tmp,
            mock.patch.dict(
                "os.environ",
                {
                    "BROWSEROS_BUILD_SOURCE_SHA": "b" * 40,
                    "GITHUB_SHA": "a" * 40,
                },
                clear=False,
            ),
        ):
            release = generate_release_json(
                _upload_ctx(Path(tmp)),
                [{"filename": "BrowserOS_v1.2.3_x64.AppImage", "size": 12}],
                "linux",
            )

        self.assertEqual(release["source_sha"], "b" * 40)

    def test_published_release_json_records_exact_component_versions(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            ctx = _upload_ctx(Path(tmp))
            ctx.env.browseros_server_resource_version = "0.0.129"
            ctx.env.bundled_product_extension_version = "0.0.125.0"
            ctx.env.onboarding_resource_version = "0.0.15"

            release = generate_release_json(
                ctx,
                [{"filename": "BrowserOS_v1.2.3_x64.AppImage", "size": 12}],
                "linux",
            )

        self.assertEqual(
            release["component_versions"],
            {
                "server": "0.0.129",
                "agent": "0.0.125.0",
                "app-onboard": "0.0.15",
            },
        )

    def test_browserclaw_release_json_records_its_exact_components(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            ctx = _upload_ctx(
                Path(tmp),
                product_id="browserclaw",
                display_name="BrowserOS neo",
                artifact_prefix="BrowserOS_neo",
            )
            ctx.env.browserclaw_server_resource_version = "0.0.29"
            ctx.env.bundled_product_extension_version = "0.2.2.0"
            ctx.env.onboarding_resource_version = "0.0.15"

            release = generate_release_json(
                ctx,
                [{"filename": "BrowserOS_neo_v1.2.3_x64.AppImage", "size": 12}],
                "linux",
            )

        self.assertEqual(
            release["component_versions"],
            {
                "claw-server-rust": "0.0.29",
                "browserclaw": "0.2.2.0",
                "claw-onboard": "0.0.15",
            },
        )

    def test_source_release_json_uses_candidate_provenance_not_github_sha(self) -> None:
        with (
            tempfile.TemporaryDirectory() as tmp,
            mock.patch.dict("os.environ", {"GITHUB_SHA": "9" * 40}, clear=False),
        ):
            ctx = _upload_ctx(Path(tmp))
            ctx.resource_mode = "source"
            ctx.source_sha = "2" * 40
            manifest = PreparedResourcesManifest(
                product="browseros",
                parent_sha="1" * 40,
                source_sha="2" * 40,
                browser_version="1.2.3",
                component_versions={
                    "server": "0.0.128",
                    "agent": "0.0.116.0",
                    "app-onboard": "0.0.12",
                },
                files={},
            )
            ctx.artifact_registry.add("prepared_resources", manifest)

            release = generate_release_json(
                ctx,
                [{"filename": "BrowserOS_v1.2.3_x64.AppImage", "size": 12}],
                "linux",
            )

        self.assertEqual(release["source_sha"], "2" * 40)
        self.assertEqual(release["parent_sha"], "1" * 40)
        self.assertEqual(release["component_versions"], manifest.component_versions)
        self.assertEqual(release["common_manifest_digest"], manifest.digest())

    def test_linux_x64_artifacts_use_x64_keys(self) -> None:
        self.assertEqual(
            _get_artifact_key("BrowserOS_v1.2.3_x64.AppImage", "linux"),
            "x64_appimage",
        )
        self.assertEqual(
            _get_artifact_key("BrowserOS_v1.2.3_amd64.deb", "linux"),
            "x64_deb",
        )

    def test_linux_arm64_artifacts_use_arm64_keys(self) -> None:
        self.assertEqual(
            _get_artifact_key("BrowserOS_v1.2.3_arm64.AppImage", "linux"),
            "arm64_appimage",
        )
        self.assertEqual(
            _get_artifact_key("BrowserOS_v1.2.3_arm64.deb", "linux"),
            "arm64_deb",
        )
        self.assertEqual(
            _get_artifact_key("BrowserOS_v1.2.3_aarch64.deb", "linux"),
            "arm64_deb",
        )

    def test_windows_installer_artifacts_use_arch_keys(self) -> None:
        self.assertEqual(
            _get_artifact_key("BrowserOS_v1.2.3_x64_installer.exe", "win"),
            "x64_installer",
        )
        self.assertEqual(
            _get_artifact_key("BrowserOS_v1.2.3_x64_installer.zip", "win"),
            "x64_zip",
        )
        self.assertEqual(
            _get_artifact_key("BrowserOS_v1.2.3_arm64_installer.exe", "win"),
            "arm64_installer",
        )
        self.assertEqual(
            _get_artifact_key("BrowserOS_v1.2.3_arm64_installer.zip", "win"),
            "arm64_zip",
        )

    def test_upload_attaches_macos_dmg_signature_metadata_by_filename(self) -> None:
        with (
            tempfile.TemporaryDirectory() as tmp,
            mock.patch("bos_build.steps.storage.upload.BOTO3_AVAILABLE", True),
            mock.patch("bos_build.steps.storage.upload.IS_MACOS", lambda: True),
            mock.patch("bos_build.steps.storage.upload.IS_WINDOWS", lambda: False),
            mock.patch(
                "bos_build.steps.storage.upload.get_r2_client", return_value=object()
            ),
            mock.patch(
                "bos_build.steps.storage.upload.get_release_json", return_value=None
            ),
            mock.patch(
                "bos_build.steps.storage.upload.upload_file_to_r2",
                return_value=True,
            ),
        ):
            dist_dir = Path(tmp)
            dmg_name = "BrowserOS_v1.2.3_arm64.dmg"
            (dist_dir / dmg_name).write_bytes(b"dmg")
            ctx = _upload_ctx(dist_dir)

            success, release = upload_release_artifacts(
                ctx,
                {dmg_name: {"sparkle_signature": "SIG==", "sparkle_length": 3}},
            )

        self.assertTrue(success)
        artifact = release["artifacts"]["arm64"]
        self.assertEqual(artifact["filename"], dmg_name)
        self.assertEqual(artifact["sparkle_signature"], "SIG==")
        self.assertEqual(artifact["sparkle_length"], 3)
        self.assertEqual(artifact["sha256"], hashlib.sha256(b"dmg").hexdigest())
        self.assertEqual(ctx.artifact_registry.get("release_metadata"), release)

    def test_upload_filters_macos_artifacts_to_context_product(self) -> None:
        cases = [
            (
                "browserclaw",
                "BrowserOS neo",
                "BrowserOS_neo",
                "BrowserOS_neo_v1.2.3_arm64.dmg",
                "BrowserOS_v1.2.3_arm64.dmg",
            ),
            (
                "browseros",
                "BrowserOS",
                "BrowserOS",
                "BrowserOS_v1.2.3_arm64.dmg",
                "BrowserOS_neo_v1.2.3_arm64.dmg",
            ),
        ]

        for product_id, display_name, prefix, selected_name, stale_name in cases:
            with (
                self.subTest(product_id=product_id),
                tempfile.TemporaryDirectory() as tmp,
                mock.patch("bos_build.steps.storage.upload.BOTO3_AVAILABLE", True),
                mock.patch("bos_build.steps.storage.upload.IS_MACOS", lambda: True),
                mock.patch("bos_build.steps.storage.upload.IS_WINDOWS", lambda: False),
                mock.patch(
                    "bos_build.steps.storage.upload.get_r2_client",
                    return_value=object(),
                ),
                mock.patch(
                    "bos_build.steps.storage.upload.get_release_json",
                    return_value=None,
                ),
                mock.patch(
                    "bos_build.steps.storage.upload.upload_file_to_r2",
                    return_value=True,
                ) as upload_file,
            ):
                dist_dir = Path(tmp)
                (dist_dir / selected_name).write_bytes(b"selected")
                (dist_dir / stale_name).write_bytes(b"stale")
                ctx = _upload_ctx(
                    dist_dir,
                    product_id=product_id,
                    display_name=display_name,
                    artifact_prefix=prefix,
                )

                success, release = upload_release_artifacts(ctx)

                self.assertTrue(success)
                self.assertEqual(
                    release["artifacts"]["arm64"]["filename"], selected_name
                )
                uploaded_names = [
                    Path(call.args[1]).name for call in upload_file.call_args_list
                ]
                self.assertEqual(uploaded_names, [selected_name, "release.json"])

    def test_upload_keeps_same_product_macos_arch_artifacts(self) -> None:
        with (
            tempfile.TemporaryDirectory() as tmp,
            mock.patch("bos_build.steps.storage.upload.BOTO3_AVAILABLE", True),
            mock.patch("bos_build.steps.storage.upload.IS_MACOS", lambda: True),
            mock.patch("bos_build.steps.storage.upload.IS_WINDOWS", lambda: False),
            mock.patch(
                "bos_build.steps.storage.upload.get_r2_client", return_value=object()
            ),
            mock.patch(
                "bos_build.steps.storage.upload.get_release_json", return_value=None
            ),
            mock.patch(
                "bos_build.steps.storage.upload.upload_file_to_r2",
                return_value=True,
            ) as upload_file,
        ):
            dist_dir = Path(tmp)
            product_names = [
                "BrowserOS_neo_v1.2.3_arm64.dmg",
                "BrowserOS_neo_v1.2.3_x64.dmg",
                "BrowserOS_neo_v1.2.3_universal.dmg",
            ]
            for name in product_names:
                (dist_dir / name).write_bytes(name.encode())
            (dist_dir / "BrowserOS_v1.2.3_arm64.dmg").write_bytes(b"stale")
            ctx = _upload_ctx(
                dist_dir,
                product_id="browserclaw",
                display_name="BrowserOS neo",
                artifact_prefix="BrowserOS_neo",
            )

            success, release = upload_release_artifacts(ctx)

        self.assertTrue(success)
        self.assertEqual(set(release["artifacts"]), {"arm64", "x64", "universal"})
        self.assertEqual(
            {artifact["filename"] for artifact in release["artifacts"].values()},
            set(product_names),
        )
        uploaded_names = [
            Path(call.args[1]).name for call in upload_file.call_args_list
        ]
        self.assertEqual(uploaded_names[:-1], sorted(product_names))
        self.assertEqual(uploaded_names[-1], "release.json")

    def test_merge_release_metadata_preserves_existing_artifacts(self) -> None:
        existing = {
            "platform": "linux",
            "version": "1.2.3",
            "build_date": "old",
            "artifacts": {
                "x64_appimage": {"filename": "BrowserOS_v1.2.3_x64.AppImage"},
                "x64_deb": {"filename": "BrowserOS_v1.2.3_amd64.deb"},
            },
        }
        new = {
            "platform": "linux",
            "version": "1.2.3",
            "build_date": "new",
            "artifacts": {
                "arm64_appimage": {"filename": "BrowserOS_v1.2.3_arm64.AppImage"},
                "arm64_deb": {"filename": "BrowserOS_v1.2.3_arm64.deb"},
            },
        }

        merged = merge_release_metadata(existing, new)

        self.assertEqual(merged["build_date"], "new")
        self.assertEqual(
            sorted(merged["artifacts"]),
            ["arm64_appimage", "arm64_deb", "x64_appimage", "x64_deb"],
        )

    def test_merge_release_metadata_overwrites_matching_artifact_keys(self) -> None:
        existing = {
            "platform": "linux",
            "version": "1.2.3",
            "artifacts": {
                "x64_appimage": {"filename": "old.AppImage", "size": 1},
            },
        }
        new = {
            "platform": "linux",
            "version": "1.2.3",
            "artifacts": {
                "x64_appimage": {"filename": "new.AppImage", "size": 2},
            },
        }

        merged = merge_release_metadata(existing, new)

        self.assertEqual(
            merged["artifacts"]["x64_appimage"]["filename"], "new.AppImage"
        )
        self.assertEqual(merged["artifacts"]["x64_appimage"]["size"], 2)

    def test_merge_release_metadata_keeps_existing_signature_fields(self) -> None:
        existing = {
            "source_sha": "same",
            "artifacts": {
                "arm64": {
                    "filename": "BrowserOS_v1.2.3_arm64.dmg",
                    "sparkle_signature": "SIG==",
                    "sparkle_length": 3,
                }
            },
        }
        new = {
            "source_sha": "same",
            "artifacts": {
                "arm64": {
                    "filename": "BrowserOS_v1.2.3_arm64.dmg",
                    "size": 3,
                    "sha256": "a" * 64,
                }
            },
        }

        merged = merge_release_metadata(existing, new)

        self.assertEqual(merged["artifacts"]["arm64"]["sparkle_signature"], "SIG==")
        self.assertEqual(merged["artifacts"]["arm64"]["sha256"], "a" * 64)

    def test_merge_release_metadata_replaces_an_earlier_run(self) -> None:
        existing = {
            "source_sha": "old",
            "workflow_run_id": "1",
            "workflow_run_attempt": "1",
            "artifacts": {"arm64_deb": {"filename": "stale.deb"}},
        }
        new = {
            "source_sha": "new",
            "workflow_run_id": "2",
            "workflow_run_attempt": "1",
            "artifacts": {"x64_deb": {"filename": "current.deb"}},
        }

        merged = merge_release_metadata(existing, new)

        self.assertEqual(merged, new)
        self.assertNotIn("arm64_deb", merged["artifacts"])

    def test_merge_release_metadata_replaces_an_earlier_attempt(self) -> None:
        existing = {
            "source_sha": "same",
            "workflow_run_id": "2",
            "workflow_run_attempt": "1",
            "artifacts": {"arm64_deb": {"filename": "stale.deb"}},
        }
        new = {
            "source_sha": "same",
            "workflow_run_id": "2",
            "workflow_run_attempt": "2",
            "artifacts": {"x64_deb": {"filename": "current.deb"}},
        }

        merged = merge_release_metadata(existing, new)

        self.assertEqual(merged, new)


if __name__ == "__main__":
    unittest.main()
