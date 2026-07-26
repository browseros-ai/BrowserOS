#!/usr/bin/env python3
"""Tests for extension build workspace helpers."""

import dataclasses
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from .specs import ExternalRepoSource, InRepoSource, spec_by_name
from .workspace import (
    _format_git_error,
    clone_url,
    resolve_source,
    run_command,
    update_manifest_version,
    write_env_file,
)


class RecordingGit:
    def __init__(self):
        self.calls = []

    def __call__(self, args, cwd=None):
        self.calls.append((tuple(args), cwd))


class ResolveSourceTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.monorepo = Path(self.tmp.name) / "monorepo"
        (self.monorepo / "packages/browseros-agent").mkdir(parents=True)
        self.work_root = Path(self.tmp.name) / "work"

    def test_in_repo_resolves_to_working_tree_without_git(self):
        git = RecordingGit()
        path = resolve_source(
            spec_by_name("agent"),
            monorepo_root=self.monorepo,
            work_root=self.work_root,
            branch_override=None,
            run_git=git,
        )
        self.assertEqual(path, self.monorepo / "packages/browseros-agent")
        self.assertEqual(git.calls, [])

    def test_in_repo_missing_path_raises(self):
        broken = dataclasses.replace(
            spec_by_name("agent"), source=InRepoSource(path="packages/gone")
        )
        with self.assertRaisesRegex(FileNotFoundError, "packages/gone"):
            resolve_source(
                broken,
                monorepo_root=self.monorepo,
                work_root=self.work_root,
                branch_override=None,
                run_git=RecordingGit(),
            )

    def test_external_fresh_clone_command(self):
        git = RecordingGit()
        path = resolve_source(
            spec_by_name("bugreporter"),
            monorepo_root=self.monorepo,
            work_root=self.work_root,
            branch_override=None,
            run_git=git,
        )
        dest = self.work_root / "repos" / "BrowserOS-feedback-extension"
        self.assertEqual(path, dest)
        self.assertEqual(
            git.calls,
            [
                (
                    (
                        "clone",
                        "--branch",
                        "main",
                        "https://github.com/browseros-ai/BrowserOS-feedback-extension.git",
                        str(dest),
                    ),
                    None,
                )
            ],
        )

    def test_external_existing_updates_and_branch_override(self):
        git = RecordingGit()
        dest = self.work_root / "repos" / "BrowserOS-agent"
        dest.mkdir(parents=True)
        path = resolve_source(
            spec_by_name("controller"),
            monorepo_root=self.monorepo,
            work_root=self.work_root,
            branch_override="canary",
            run_git=git,
        )
        self.assertEqual(path, dest)
        self.assertEqual(
            git.calls,
            [
                (("fetch", "origin", "canary"), dest),
                (("checkout", "canary"), dest),
                (("reset", "--hard", "origin/canary"), dest),
            ],
        )

    def test_clone_url_embeds_gh_token_when_set(self):
        with patch.dict("os.environ", {"GH_TOKEN": "secret123"}):
            self.assertEqual(
                clone_url("browseros-ai/BrowserOS-agent"),
                "https://x-access-token:secret123@github.com/browseros-ai/BrowserOS-agent.git",
            )
        with patch.dict("os.environ", {}, clear=False):
            import os

            os.environ.pop("GH_TOKEN", None)
            self.assertEqual(
                clone_url("browseros-ai/BrowserOS-agent"),
                "https://github.com/browseros-ai/BrowserOS-agent.git",
            )

    def test_branch_override_applies_to_fresh_clone(self):
        git = RecordingGit()
        resolve_source(
            spec_by_name("bugreporter"),
            monorepo_root=self.monorepo,
            work_root=self.work_root,
            branch_override="release-candidate",
            run_git=git,
        )
        args, _ = git.calls[0]
        self.assertIn("release-candidate", args)
        self.assertNotIn("main", args)


class GitErrorRedactionTest(unittest.TestCase):
    def test_token_in_clone_args_and_stderr_is_masked(self):
        args = [
            "clone",
            "--branch",
            "main",
            "https://x-access-token:tok3n@github.com/browseros-ai/x.git",
            "/work/repos/x",
        ]
        stderr = (
            "fatal: repository "
            "'https://x-access-token:tok3n@github.com/browseros-ai/x.git' "
            "not found"
        )
        message = _format_git_error(args, stderr)
        self.assertNotIn("tok3n", message)
        self.assertIn("://***@github.com", message)
        self.assertIn("git clone --branch main", message)

    def test_plain_urls_pass_through(self):
        message = _format_git_error(
            ["fetch", "origin", "main"], "fatal: could not read from remote"
        )
        self.assertEqual(
            message,
            "git fetch origin main failed: fatal: could not read from remote",
        )


class UpdateManifestVersionTest(unittest.TestCase):
    def test_bumps_version_and_preserves_other_keys(self):
        with tempfile.TemporaryDirectory() as tmp:
            manifest = Path(tmp) / "package.json"
            manifest.write_text(
                json.dumps({"name": "@browseros/app", "version": "0.0.1", "private": True})
            )
            update_manifest_version(manifest, "9.8.7")
            content = manifest.read_text()
            data = json.loads(content)
            self.assertEqual(data["version"], "9.8.7")
            self.assertEqual(data["name"], "@browseros/app")
            self.assertTrue(data["private"])
            self.assertTrue(content.endswith("\n"))

    def test_missing_manifest_raises(self):
        with self.assertRaises(FileNotFoundError):
            update_manifest_version(Path("/nonexistent/manifest.json"), "1.0")


class WriteEnvFileTest(unittest.TestCase):
    def test_writes_present_vars_skips_missing_and_replaces_stale(self):
        with tempfile.TemporaryDirectory() as tmp:
            env_dir = Path(tmp)
            (env_dir / ".env").write_text("STALE=old\n")
            with patch.dict(
                "os.environ",
                {"NODE_ENV": "production", "POSTHOG_API_KEY": "ph-key"},
            ):
                import os

                os.environ.pop("MISSING_VAR", None)
                path = write_env_file(
                    env_dir, ("NODE_ENV", "POSTHOG_API_KEY", "MISSING_VAR")
                )
            content = path.read_text()
            self.assertIn("NODE_ENV", content)
            self.assertIn("production", content)
            self.assertIn("ph-key", content)
            self.assertNotIn("MISSING_VAR", content)
            self.assertNotIn("STALE", content)

    def test_missing_required_var_fails_before_replacing_existing_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            env_dir = Path(tmp)
            env_path = env_dir / ".env"
            env_path.write_text("PRESERVED=old\n")
            with patch.dict("os.environ", {}, clear=False):
                import os

                os.environ.pop("VITE_CLAW_POSTHOG_KEY", None)
                with self.assertRaisesRegex(
                    EnvironmentError, "VITE_CLAW_POSTHOG_KEY"
                ):
                    write_env_file(
                        env_dir,
                        ("VITE_CLAW_POSTHOG_KEY", "VITE_CLAW_POSTHOG_HOST"),
                        required_names=("VITE_CLAW_POSTHOG_KEY",),
                    )

            self.assertEqual(env_path.read_text(), "PRESERVED=old\n")


class RunCommandTest(unittest.TestCase):
    def test_nonzero_exit_raises_with_command(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(RuntimeError, "exit 7"):
                run_command("exit 7", Path(tmp))

    def test_success_runs_in_cwd(self):
        with tempfile.TemporaryDirectory() as tmp:
            run_command("touch made-here", Path(tmp))
            self.assertTrue((Path(tmp) / "made-here").exists())


class SpecSourceTypesTest(unittest.TestCase):
    def test_external_specs_expose_repo_names_for_work_dirs(self):
        source = spec_by_name("controller").source
        self.assertIsInstance(source, ExternalRepoSource)
        self.assertEqual(source.repo.split("/")[-1], "BrowserOS-agent")


if __name__ == "__main__":
    unittest.main()
