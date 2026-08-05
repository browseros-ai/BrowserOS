#!/usr/bin/env python3
"""CLI surface tests for the release subcommands."""

import re
import os
import subprocess
import tempfile
import unittest
from contextlib import contextmanager
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from typer.testing import CliRunner

from bos_build.browseros import app
from bos_build.cli import release as release_cli
from bos_build.cli import release_feeds
from bos_build.release import github as github_module
from bos_build.release import publish as publish_module

runner = CliRunner()
ANSI_RE = re.compile(r"\x1b\[[0-9;]*m")


def invoke(*args: str):
    return runner.invoke(app, ["release", *args])


def combined(result) -> str:
    """stdout + stderr across click versions (8.2 split them)."""
    out = result.output
    try:
        out += result.stderr
    except (ValueError, AttributeError):
        pass
    return out


def plain_output(result) -> str:
    return ANSI_RE.sub("", combined(result))


class ReleaseHelpTest(unittest.TestCase):
    def test_lists_subcommands_without_flag_soup(self):
        result = invoke("--help")

        self.assertEqual(result.exit_code, 0, combined(result))
        for command in ("list", "appcast", "publish", "download", "github"):
            self.assertIn(command, result.output)
        for flag in ("--list", "--appcast", "--publish", "--download"):
            self.assertNotIn(flag, result.output)

    def test_bare_release_shows_help(self):
        result = runner.invoke(app, ["release"])

        self.assertEqual(result.exit_code, 0, combined(result))
        self.assertIn("Usage", result.output)

    def test_show_modules_lists_modules(self):
        result = invoke("--show-modules")

        self.assertEqual(result.exit_code, 0, combined(result))
        self.assertIn("appcast", combined(result))

    def test_list_help_shows_bounding_options(self):
        result = invoke("list", "--help")

        self.assertEqual(result.exit_code, 0, combined(result))
        help_text = plain_output(result)
        for token in ("--product", "--limit", "--all", "--version", "VERSION"):
            self.assertIn(token, help_text)


class ReleaseListInvocationTest(unittest.TestCase):
    def test_conflicting_positional_and_option_version_errors(self):
        result = invoke("list", "0.31.0", "--version", "0.32.0")

        self.assertNotEqual(result.exit_code, 0)
        self.assertIn("Conflicting", combined(result))

    def test_all_flag_removes_limit_and_spans_products(self):
        with mock.patch.object(release_cli, "execute_module") as em:
            result = invoke("list", "--all")

        self.assertEqual(result.exit_code, 0, combined(result))
        module = em.call_args[0][1]
        self.assertIsNone(module.limit)
        self.assertEqual(
            [product.id for product in module.products],
            ["browseros", "browserclaw"],
        )

    def test_limit_and_product_filter(self):
        with mock.patch.object(release_cli, "execute_module") as em:
            result = invoke("list", "--product", "browserclaw", "-n", "3")

        self.assertEqual(result.exit_code, 0, combined(result))
        module = em.call_args[0][1]
        self.assertEqual(module.limit, 3)
        self.assertEqual([product.id for product in module.products], ["browserclaw"])

    def test_version_detail_uses_product_context(self):
        with mock.patch.object(release_cli, "execute_module") as em:
            result = invoke("list", "0.31.0", "--product", "browserclaw")

        self.assertEqual(result.exit_code, 0, combined(result))
        ctx, module = em.call_args[0]
        self.assertEqual(ctx.release_version, "0.31.0")
        self.assertEqual(ctx.product.id, "browserclaw")
        self.assertIsNone(module.products)

    def test_matching_positional_and_option_version_is_not_a_conflict(self):
        with mock.patch.object(release_cli, "execute_module"):
            result = invoke("list", "0.31.0", "--version", "0.31.0")

        self.assertNotIn("Conflicting", combined(result))

    def test_unknown_product_names_valid_ids(self):
        result = invoke("list", "--product", "nosuch")

        self.assertNotEqual(result.exit_code, 0)
        self.assertIn("browserclaw", combined(result))


class ReleaseVersionRequiredTest(unittest.TestCase):
    def test_appcast_requires_version(self):
        self.assertNotEqual(invoke("appcast").exit_code, 0)

    def test_publish_requires_version(self):
        self.assertNotEqual(invoke("publish").exit_code, 0)

    def test_download_requires_version(self):
        self.assertNotEqual(invoke("download").exit_code, 0)


class FeedPublisherCliTest(unittest.TestCase):
    def test_publish_local_defaults_to_dry_run(self):
        with (
            _configured_r2_env(),
            mock.patch.object(release_feeds, "FeedPublisher") as publisher,
        ):
            publisher.return_value.publish_staged.return_value = True

            result = invoke(
                "feeds",
                "publish-local",
                "appcast.xml",
                "extensions/bundled-manifest.xml",
            )

        self.assertEqual(result.exit_code, 0, plain_output(result))
        publisher.return_value.publish_staged.assert_called_once_with(
            ["appcast.xml", "extensions/bundled-manifest.xml"],
            publish=False,
            allow_downgrade=False,
            repair_invalid_live=False,
        )

    def test_publish_local_forwards_explicit_write_flags(self):
        with (
            _configured_r2_env(),
            mock.patch.object(release_feeds, "FeedPublisher") as publisher,
        ):
            publisher.return_value.publish_staged.return_value = True

            result = invoke(
                "feeds",
                "publish-local",
                "appcast.xml",
                "--publish",
                "--allow-downgrade",
            )

        self.assertEqual(result.exit_code, 0, plain_output(result))
        publisher.return_value.publish_staged.assert_called_once_with(
            ["appcast.xml"],
            publish=True,
            allow_downgrade=True,
            repair_invalid_live=False,
        )

    def test_publish_local_forwards_repair_flag_without_implying_publish(self):
        with (
            _configured_r2_env(),
            mock.patch.object(release_feeds, "FeedPublisher") as publisher,
        ):
            publisher.return_value.publish_staged.return_value = True

            result = invoke(
                "feeds",
                "publish-local",
                "appcast-server.xml",
                "--repair-invalid-live",
            )

        self.assertEqual(result.exit_code, 0, plain_output(result))
        publisher.return_value.publish_staged.assert_called_once_with(
            ["appcast-server.xml"],
            publish=False,
            allow_downgrade=False,
            repair_invalid_live=True,
        )

    def test_publish_local_forwards_repair_and_publish_independently(self):
        with (
            _configured_r2_env(),
            mock.patch.object(release_feeds, "FeedPublisher") as publisher,
        ):
            publisher.return_value.publish_staged.return_value = True

            result = invoke(
                "feeds",
                "publish-local",
                "appcast-server.xml",
                "--repair-invalid-live",
                "--publish",
            )

        self.assertEqual(result.exit_code, 0, plain_output(result))
        publisher.return_value.publish_staged.assert_called_once_with(
            ["appcast-server.xml"],
            publish=True,
            allow_downgrade=False,
            repair_invalid_live=True,
        )

    def test_publish_local_failure_exits_nonzero(self):
        with (
            _configured_r2_env(),
            mock.patch.object(release_feeds, "FeedPublisher") as publisher,
        ):
            publisher.return_value.publish_staged.return_value = False

            result = invoke("feeds", "publish-local", "unknown.xml")

        self.assertNotEqual(result.exit_code, 0)

    def test_upload_menu_expands_grouped_extension_choices(self):
        repo_root = Path(__file__).resolve().parents[4]
        upload_script = repo_root / "updates" / "upload.sh"
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            capture = tmp_path / "args"
            fake_uv = tmp_path / "uv"
            fake_uv.write_text('#!/bin/sh\nprintf "%s\\n" "$*" >> "$CAPTURE"\n')
            fake_uv.chmod(0o755)
            env = {
                **os.environ,
                "CAPTURE": str(capture),
                "PATH": f"{tmp_path}:{os.environ['PATH']}",
            }

            result = subprocess.run(
                ["bash", str(upload_script)],
                input="13,15\nn\n",
                text=True,
                capture_output=True,
                cwd=tmp_path,
                env=env,
            )

            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(
                capture.read_text().splitlines(),
                [
                    "run browseros release feeds publish-local "
                    "extensions/extensions.json "
                    "extensions/update-manifest.xml "
                    "extensions/bundled-manifest.xml"
                ],
            )
            script = upload_script.read_text()
            self.assertNotIn("--repair-invalid-live", script)
            self.assertNotIn("--allow-downgrade", script)

    def test_repair_flag_is_absent_from_routine_commands_and_workflows(self):
        self.assertNotIn(
            "--repair-invalid-live", plain_output(invoke("appcast", "--help"))
        )
        self.assertNotIn(
            "--repair-invalid-live", plain_output(invoke("extensions", "--help"))
        )
        repo_root = Path(__file__).resolve().parents[4]
        for workflow in (
            "release-browseros.yml",
            "release-browserclaw.yml",
        ):
            self.assertNotIn(
                "--repair-invalid-live",
                (repo_root / ".github" / "workflows" / workflow).read_text(),
            )


class CreateReleaseContextTest(unittest.TestCase):
    def test_sets_product_from_registry(self):
        ctx = release_cli.create_release_context("1.0.0", product="browserclaw")

        self.assertEqual(ctx.product.id, "browserclaw")
        self.assertEqual(ctx.release_version, "1.0.0")

    def test_defaults_to_browseros(self):
        ctx = release_cli.create_release_context("1.0.0")

        self.assertEqual(ctx.product.id, "browseros")


def _configured_r2_env():
    return mock.patch.dict(
        os.environ,
        {
            "R2_ACCOUNT_ID": "account",
            "R2_ACCESS_KEY_ID": "key",
            "R2_SECRET_ACCESS_KEY": "secret",
            "R2_BUCKET": "bucket",
        },
        clear=False,
    )


def _github_metadata(product: str = "browserclaw"):
    prefix = "BrowserOS_neo" if product == "browserclaw" else "BrowserOS"
    filename = f"{prefix}_v0.49.0_x64_installer.exe"
    zip_filename = f"{prefix}_v0.49.0_x64_installer.zip"
    return {
        "win": {
            "product": product,
            "version": "0.49.0",
            "platform": "win",
            "chromium_version": "140.0.0.0",
            "source_sha": "a" * 40,
            "workflow_run_id": "123",
            "workflow_run_attempt": "1",
            "artifacts": {
                "x64_installer": {
                    "filename": filename,
                    "url": f"https://cdn.browseros.com/{filename}",
                },
                "x64_zip": {
                    "filename": zip_filename,
                    "url": f"https://cdn.browseros.com/{zip_filename}",
                },
            },
        }
    }


class GithubCreateCliIntegrityTest(unittest.TestCase):
    def invoke_create(self, *extra: str):
        return invoke(
            "github",
            "create",
            "--version",
            "0.49.0",
            "--product",
            "browserclaw",
            "--repo",
            "browseros-ai/BrowserOS",
            *extra,
        )

    @contextmanager
    def patches(self, metadata=None):
        with (
            _configured_r2_env(),
            mock.patch.object(github_module, "BOTO3_AVAILABLE", True),
            mock.patch.object(github_module, "check_gh_cli", return_value=True),
            mock.patch.object(
                github_module,
                "fetch_all_release_metadata",
                return_value=_github_metadata() if metadata is None else metadata,
            ),
        ):
            yield

    def test_create_failure_makes_actual_cli_exit_nonzero(self):
        with (
            self.patches(),
            mock.patch.object(
                github_module,
                "create_github_release",
                return_value=(False, "GitHub API unavailable"),
            ),
        ):
            result = self.invoke_create()

        self.assertNotEqual(result.exit_code, 0)
        self.assertIn("GitHub API unavailable", plain_output(result))

    def test_missing_metadata_makes_actual_cli_exit_nonzero(self):
        with self.patches(metadata={}):
            result = self.invoke_create()

        self.assertNotEqual(result.exit_code, 0)
        self.assertIn("No release metadata", plain_output(result))

    def test_failed_asset_transfer_makes_actual_cli_exit_nonzero(self):
        with (
            self.patches(),
            mock.patch.object(
                github_module,
                "create_github_release",
                return_value=(True, "https://github.com/release"),
            ),
            mock.patch.object(
                github_module,
                "download_and_upload_artifacts",
                return_value=[("BrowserOS_neo_installer.exe", False)],
            ),
        ):
            result = self.invoke_create()

        self.assertNotEqual(result.exit_code, 0)
        self.assertIn("BrowserOS_neo_installer.exe", plain_output(result))

    def test_zero_asset_candidates_makes_actual_cli_exit_nonzero(self):
        with (
            self.patches(),
            mock.patch.object(
                github_module,
                "create_github_release",
                return_value=(True, "https://github.com/release"),
            ),
            mock.patch.object(
                github_module,
                "download_and_upload_artifacts",
                return_value=[],
            ),
        ):
            result = self.invoke_create()

        self.assertNotEqual(result.exit_code, 0)
        self.assertIn("No release artifacts", plain_output(result))

    def test_success_pins_target_and_uses_product_notes(self):
        target = "a" * 40
        with (
            self.patches(),
            mock.patch.object(
                github_module,
                "create_github_release",
                return_value=(True, "https://github.com/release"),
            ) as create,
            mock.patch.object(
                github_module,
                "download_and_upload_artifacts",
                return_value=[("BrowserOS_neo_installer.exe", True)],
            ),
            mock.patch.object(
                github_module,
                "resolve_github_tag_target",
                return_value=target,
            ),
        ):
            result = self.invoke_create("--target", target)

        self.assertEqual(result.exit_code, 0, plain_output(result))
        args = create.call_args.args
        self.assertIn("## BrowserOS neo v0.49.0", args[3])
        self.assertEqual(args[0], "browserclaw/v0.49.0")
        self.assertEqual(args[5], target)

    def test_created_release_target_mismatch_blocks_asset_upload(self):
        target = "a" * 40
        with (
            self.patches(),
            mock.patch.object(
                github_module,
                "create_github_release",
                return_value=(True, "https://github.com/release"),
            ),
            mock.patch.object(
                github_module,
                "resolve_github_tag_target",
                return_value="b" * 40,
            ),
            mock.patch.object(
                github_module,
                "download_and_upload_artifacts",
            ) as upload,
        ):
            result = self.invoke_create("--target", target)

        self.assertNotEqual(result.exit_code, 0)
        self.assertIn("not requested target", plain_output(result))
        upload.assert_not_called()

    def test_existing_release_remains_refreshable_when_uploads_succeed(self):
        with (
            self.patches(),
            mock.patch.object(
                github_module,
                "create_github_release",
                return_value=(False, "Release v0.49.0 already exists"),
            ),
            mock.patch.object(
                github_module,
                "download_and_upload_artifacts",
                return_value=[("BrowserOS_neo_installer.exe", True)],
            ),
            mock.patch.object(
                github_module,
                "inspect_github_release",
                return_value={"isDraft": True, "assets": []},
            ),
            mock.patch.object(github_module, "edit_github_release") as edit,
        ):
            result = self.invoke_create()

        self.assertEqual(result.exit_code, 0, plain_output(result))
        self.assertEqual(edit.call_args.args[0], "browserclaw/v0.49.0")

    def test_existing_published_release_is_never_refreshable(self):
        with (
            self.patches(),
            mock.patch.object(
                github_module,
                "create_github_release",
                return_value=(False, "Release browserclaw/v0.49.0 already exists"),
            ),
            mock.patch.object(
                github_module,
                "inspect_github_release",
                return_value={"isDraft": False, "assets": []},
            ),
            mock.patch.object(
                github_module,
                "download_and_upload_artifacts",
            ) as upload,
        ):
            result = self.invoke_create()

        self.assertNotEqual(result.exit_code, 0)
        self.assertIn("not a draft", plain_output(result))
        upload.assert_not_called()

    def test_contracted_refresh_reconciles_and_verifies_exact_assets(self):
        metadata = _github_metadata()
        expected = {
            artifact["filename"] for artifact in metadata["win"]["artifacts"].values()
        }
        initial_assets = [sorted(expected)[0]]
        with (
            self.patches(metadata),
            mock.patch.object(
                github_module,
                "create_github_release",
                return_value=(False, "Release browserclaw/v0.49.0 already exists"),
            ),
            mock.patch.object(
                github_module,
                "inspect_github_release",
                side_effect=[
                    {"isDraft": True, "assets": initial_assets},
                    {"isDraft": True, "assets": sorted(expected)},
                ],
            ),
            mock.patch.object(
                github_module,
                "delete_github_release_asset",
            ) as delete,
            mock.patch.object(github_module, "edit_github_release"),
            mock.patch.object(
                github_module,
                "download_and_upload_artifacts",
                return_value=[(filename, True) for filename in sorted(expected)],
            ),
        ):
            result = self.invoke_create(
                "--platforms",
                "windows",
                "--source-sha",
                "a" * 40,
                "--workflow-run-id",
                "123",
                "--workflow-run-attempt",
                "1",
            )

        self.assertEqual(result.exit_code, 0, plain_output(result))
        self.assertEqual(
            [call.args[2] for call in delete.call_args_list],
            initial_assets,
        )

    def test_partial_refresh_never_deletes_other_platform_assets(self):
        existing_assets = [
            "BrowserOS_neo_v0.49.0_universal.dmg",
            "BrowserOS_neo_v0.49.0_x64_installer.exe",
        ]
        with (
            self.patches(),
            mock.patch.object(
                github_module,
                "create_github_release",
                return_value=(False, "Release browserclaw/v0.49.0 already exists"),
            ),
            mock.patch.object(
                github_module,
                "inspect_github_release",
                return_value={"isDraft": True, "assets": existing_assets},
            ),
            mock.patch.object(
                github_module,
                "delete_github_release_asset",
            ) as delete,
            mock.patch.object(github_module, "edit_github_release") as edit,
            mock.patch.object(
                github_module,
                "download_and_upload_artifacts",
            ) as upload,
        ):
            result = self.invoke_create(
                "--platforms",
                "windows",
                "--source-sha",
                "a" * 40,
                "--workflow-run-id",
                "123",
                "--workflow-run-attempt",
                "1",
            )

        self.assertNotEqual(result.exit_code, 0)
        self.assertIn("Refusing a partial refresh", plain_output(result))
        delete.assert_not_called()
        edit.assert_not_called()
        upload.assert_not_called()

    def test_skip_upload_preserves_existing_draft_assets(self):
        with (
            self.patches(),
            mock.patch.object(
                github_module,
                "create_github_release",
                return_value=(False, "Release browserclaw/v0.49.0 already exists"),
            ),
            mock.patch.object(
                github_module,
                "inspect_github_release",
                return_value={"isDraft": True, "assets": ["keep.exe", "keep.zip"]},
            ),
            mock.patch.object(
                github_module,
                "delete_github_release_asset",
            ) as delete,
            mock.patch.object(github_module, "edit_github_release"),
            mock.patch.object(
                github_module,
                "download_and_upload_artifacts",
            ) as upload,
        ):
            result = self.invoke_create(
                "--platforms",
                "windows",
                "--source-sha",
                "a" * 40,
                "--workflow-run-id",
                "123",
                "--workflow-run-attempt",
                "1",
                "--skip-upload",
            )

        self.assertEqual(result.exit_code, 0, plain_output(result))
        delete.assert_not_called()
        upload.assert_not_called()

    def test_tag_resolver_treats_only_missing_ref_422_as_absent(self):
        tag = "browserclaw/v0.49.0"
        missing = SimpleNamespace(
            returncode=1,
            stdout=(
                "HTTP/2.0 422 Unprocessable Entity\n"
                "Content-Type: application/json\n\n"
                f'{{"message":"No commit found for SHA: {tag}"}}'
            ),
            stderr="gh: no commit",
        )
        unrelated = SimpleNamespace(
            returncode=1,
            stdout=(
                "HTTP/2.0 422 Unprocessable Entity\n"
                "Content-Type: application/json\n\n"
                '{"message":"Validation Failed"}'
            ),
            stderr="gh: validation failed",
        )

        with mock.patch.object(
            github_module.subprocess,
            "run",
            side_effect=[missing, unrelated],
        ) as run:
            self.assertIsNone(
                github_module.resolve_github_tag_target(
                    tag,
                    "browseros-ai/BrowserOS",
                )
            )
            with self.assertRaisesRegex(RuntimeError, "Could not resolve Git tag"):
                github_module.resolve_github_tag_target(
                    tag,
                    "browseros-ai/BrowserOS",
                )

        self.assertIn(
            "commits/browserclaw%2Fv0.49.0",
            run.call_args_list[0].args[0][-1],
        )


class PublishCliIntegrityTest(unittest.TestCase):
    def invoke_publish(self, *extra: str):
        return invoke(
            "publish",
            "--version",
            "0.49.0",
            "--product",
            "browserclaw",
            *extra,
        )

    @contextmanager
    def patches(self, metadata):
        with (
            _configured_r2_env(),
            mock.patch.object(publish_module, "BOTO3_AVAILABLE", True),
            mock.patch.object(
                publish_module,
                "fetch_all_release_metadata",
                return_value=metadata,
            ),
        ):
            yield

    def test_missing_requested_platform_makes_actual_cli_exit_nonzero(self):
        with (
            self.patches(_github_metadata()),
            mock.patch.object(publish_module, "get_r2_client", return_value=object()),
        ):
            result = self.invoke_publish()

        self.assertNotEqual(result.exit_code, 0)
        self.assertIn("missing release metadata", plain_output(result).lower())

    def test_copy_failure_makes_actual_cli_exit_nonzero(self):
        with (
            self.patches(_github_metadata()),
            mock.patch.object(publish_module, "get_r2_client", return_value=object()),
            mock.patch.object(
                publish_module,
                "copy_to_download_path",
                return_value=False,
            ),
        ):
            result = self.invoke_publish(
                "--platform",
                "win",
                "--macos-arch",
                "universal",
                "--source-sha",
                "a" * 40,
                "--workflow-run-id",
                "123",
                "--workflow-run-attempt",
                "1",
            )

        self.assertNotEqual(result.exit_code, 0)
        self.assertIn("Failed to publish", plain_output(result))

    def test_explicit_partial_platform_can_succeed(self):
        with (
            self.patches(_github_metadata()),
            mock.patch.object(publish_module, "get_r2_client", return_value=object()),
            mock.patch.object(
                publish_module,
                "copy_to_download_path",
                return_value=True,
            ),
        ):
            result = self.invoke_publish(
                "--platform",
                "win",
                "--macos-arch",
                "universal",
                "--source-sha",
                "a" * 40,
                "--workflow-run-id",
                "123",
                "--workflow-run-attempt",
                "1",
            )

        self.assertEqual(result.exit_code, 0, plain_output(result))

    def test_promotion_provenance_mismatch_blocks_every_copy(self):
        metadata = _github_metadata()
        metadata["win"]["source_sha"] = "b" * 40
        with (
            self.patches(metadata),
            mock.patch.object(publish_module, "get_r2_client", return_value=object()),
            mock.patch.object(
                publish_module,
                "copy_to_download_path",
                return_value=True,
            ) as copy,
        ):
            result = self.invoke_publish(
                "--platform",
                "win",
                "--macos-arch",
                "universal",
                "--source-sha",
                "a" * 40,
                "--workflow-run-id",
                "123",
                "--workflow-run-attempt",
                "1",
            )

        self.assertNotEqual(result.exit_code, 0)
        self.assertIn("source_sha mismatch", plain_output(result))
        copy.assert_not_called()

    def test_mixed_request_fails_when_one_platform_has_no_promotable_assets(self):
        metadata = {
            "macos": {
                "artifacts": {
                    "universal": {
                        "filename": "BrowserOS_neo_v0.49.0_universal.dmg",
                        "url": (
                            "https://cdn.browseros.com/releases/browserclaw/"
                            "0.49.0/macos/BrowserOS_neo_v0.49.0_universal.dmg"
                        ),
                    }
                }
            },
            "win": {
                "artifacts": {
                    "unknown": {
                        "filename": "unknown.bin",
                        "url": "https://cdn.browseros.com/unknown.bin",
                    }
                }
            },
        }
        with (
            self.patches(metadata),
            mock.patch.object(publish_module, "get_r2_client", return_value=object()),
            mock.patch.object(
                publish_module,
                "copy_to_download_path",
                return_value=True,
            ) as copy,
        ):
            result = self.invoke_publish(
                "--platform",
                "macos",
                "--platform",
                "win",
            )

        self.assertNotEqual(result.exit_code, 0)
        self.assertIn("requested platform win", plain_output(result))
        copy.assert_not_called()


if __name__ == "__main__":
    unittest.main()
