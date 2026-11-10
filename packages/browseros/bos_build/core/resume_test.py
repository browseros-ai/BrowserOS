#!/usr/bin/env python3
"""Tests for strict resume checkpoints."""

import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from .context import Context
from .resume import (
    ResumeValidationError,
    checkpoint_path,
    make_resume_state,
    validate_resume_before_execution,
    write_step_checkpoint,
)
from .step import Step
from ..lib.testing import MockBrowserOSRoot, MockChromium


class _CompileStep(Step):
    name = "compile"
    produces = ["built_app"]


class _SignStep(Step):
    name = "sign_macos"
    produces = ["signed_app"]


class _PatchesStep(Step):
    name = "patches"


class _ChromiumReplaceStep(Step):
    name = "chromium_replace"


class _StringReplacesStep(Step):
    name = "string_replaces"


class ResumeCheckpointTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.repo_root = self.base / "repo"
        self.browseros = MockBrowserOSRoot(self.repo_root / "packages/browseros")
        self._init_repo(self.repo_root)
        self.chromium = MockChromium(self.base / "chromium").with_git()
        self.env = mock.patch.dict(
            os.environ,
            {
                "BROWSERCLAW_SERVER_RESOURCE_VERSION": "0.0.138",
                "BUNDLED_PRODUCT_EXTENSION_VERSION": "0.0.132.0",
                "ONBOARDING_RESOURCE_VERSION": "0.0.36",
            },
        )
        self.env.start()
        self.addCleanup(self.env.stop)
        self.plan = (("x64", ("compile", "sign_macos")),)

    def _init_repo(self, root: Path) -> None:
        subprocess.run(
            ["git", "init", "--initial-branch=main"],
            cwd=root,
            check=True,
            capture_output=True,
        )
        subprocess.run(["git", "config", "user.name", "Resume test"], cwd=root, check=True)
        subprocess.run(
            ["git", "config", "user.email", "resume@example.invalid"],
            cwd=root,
            check=True,
        )
        subprocess.run(["git", "add", "."], cwd=root, check=True)
        subprocess.run(
            ["git", "commit", "-m", "initial"],
            cwd=root,
            check=True,
            capture_output=True,
        )

    def _context(
        self,
        resume_from: str = "sign_macos",
        plan: tuple[tuple[str, tuple[str, ...]], ...] | None = None,
        patch_root: Path | None = None,
    ) -> Context:
        ctx = Context(
            root_dir=self.browseros.root,
            patch_root=patch_root,
            chromium_src=self.chromium.src,
            architecture="x64",
            plan_architectures=("x64",),
            build_type="release",
        )
        ctx.resume_state = make_resume_state(
            ctx,
            plan or self.plan,
            resume_from=resume_from,
            strict=True,
        )
        return ctx

    def _write_compile_checkpoint(self) -> tuple[Context, Path]:
        ctx = self._context()
        app = ctx.get_app_path()
        app.parent.mkdir(parents=True)
        app.write_bytes(b"browser")
        ctx.artifact_registry.add("built_app", app)
        write_step_checkpoint(ctx, _CompileStep(), self.plan[0][1])
        return ctx, app

    def test_valid_checkpoint_restores_required_artifact(self):
        _ctx, app = self._write_compile_checkpoint()
        resumed = self._context()

        validate_resume_before_execution([(resumed, ("sign_macos",))])

        self.assertEqual(resumed.artifact_registry.get("built_app"), app)

    def test_published_checkpoint_records_product_owned_onboarding_pin(self):
        ctx = self._context()

        self.assertEqual(
            ctx.resume_state.candidate["resources"]["component_versions"],
            {
                "claw-server-rust": "0.0.138",
                "browserclaw": "0.0.132.0",
                "claw-onboard": "0.0.36",
            },
        )

    def test_modified_artifact_fails_before_resume(self):
        _ctx, app = self._write_compile_checkpoint()
        app.write_bytes(b"stale")
        resumed = self._context()

        with self.assertRaisesRegex(ResumeValidationError, "checksum mismatch"):
            validate_resume_before_execution([(resumed, ("sign_macos",))])

    def test_cross_architecture_checkpoint_fails(self):
        ctx, _app = self._write_compile_checkpoint()
        path = checkpoint_path(ctx, "compile")
        document = json.loads(path.read_text(encoding="utf-8"))
        document["architecture"] = "arm64"
        path.write_text(json.dumps(document), encoding="utf-8")
        resumed = self._context()

        with self.assertRaisesRegex(ResumeValidationError, "architecture mismatch"):
            validate_resume_before_execution([(resumed, ("sign_macos",))])

    def test_corrupt_checkpoint_fails(self):
        ctx, _app = self._write_compile_checkpoint()
        checkpoint_path(ctx, "compile").write_text("{", encoding="utf-8")
        resumed = self._context()

        with self.assertRaisesRegex(ResumeValidationError, "corrupt"):
            validate_resume_before_execution([(resumed, ("sign_macos",))])

    def test_source_identity_mismatch_fails(self):
        _ctx, _app = self._write_compile_checkpoint()
        version = self.browseros.root / "resources" / "BROWSEROS_VERSION"
        version.write_text(version.read_text() + "\n")
        resumed = self._context()

        with self.assertRaisesRegex(ResumeValidationError, "BrowserOS source"):
            validate_resume_before_execution([(resumed, ("sign_macos",))])

    def test_external_patch_edits_invalidate_resume_without_main_changes(self):
        package = MockBrowserOSRoot(
            self.base / "external", chromium_version="158.0.8127.18"
        ).with_patch_store()
        patch = package.add_patch("chrome/example.cc", "first patch")
        package.with_v8_store()
        v8 = package.root / "v8_patches"
        (v8 / "example.cc").write_text("first V8 patch")
        ctx = self._context(patch_root=package.root)
        self.assertTrue(ctx.resume_state.enabled, ctx.resume_state.unresumable_reason)
        app = ctx.get_app_path()
        app.parent.mkdir(parents=True)
        app.write_bytes(b"browser")
        ctx.artifact_registry.add("built_app", app)
        write_step_checkpoint(ctx, _CompileStep(), self.plan[0][1])
        unchanged = self._context(patch_root=package.root)
        validate_resume_before_execution([(unchanged, ("sign_macos",))])

        for path in (
            patch,
            package.root / "chromium_patches/.features.yaml",
            package.root / "series_patches/series",
            v8 / "example.cc",
            v8 / ".store.yaml",
            package.root / "CHROMIUM_VERSION",
        ):
            with self.subTest(path=path):
                original = path.read_text()
                path.write_text(original + "\n")
                resumed = self._context(patch_root=package.root)
                self.assertEqual(
                    ctx.resume_state.candidate["browseros_source"],
                    resumed.resume_state.candidate["browseros_source"],
                )
                with self.assertRaisesRegex(ResumeValidationError, "Chromium pin"):
                    validate_resume_before_execution([(resumed, ("sign_macos",))])
                path.write_text(original)

    def test_unpinned_published_resources_make_strict_resume_unprovable(self):
        with mock.patch.dict(os.environ, {}, clear=True):
            resumed = self._context()

        with self.assertRaisesRegex(ResumeValidationError, "exact component pins"):
            validate_resume_before_execution([(resumed, ("sign_macos",))])

    def test_sliced_resume_checkpoint_records_full_plan(self):
        plan = (("x64", ("compile", "sign_macos", "package")),)
        ctx = self._context(resume_from="sign_macos", plan=plan)
        app = ctx.get_app_path()
        app.parent.mkdir(parents=True)
        app.write_bytes(b"browser")
        ctx.artifact_registry.add("built_app", app)
        write_step_checkpoint(ctx, _CompileStep(), plan[0][1])
        ctx.artifact_registry.add("signed_app", app)
        write_step_checkpoint(ctx, _SignStep(), ("sign_macos", "package"))

        document = json.loads(
            checkpoint_path(ctx, "sign_macos").read_text(encoding="utf-8")
        )
        self.assertEqual(document["run_steps"], list(plan[0][1]))
        resumed = self._context(resume_from="package", plan=plan)

        validate_resume_before_execution([(resumed, ("package",))])

    def test_later_chromium_mutation_checkpoint_supersedes_earlier_digest(self):
        plan = (("x64", ("chromium_replace", "string_replaces", "configure")),)
        ctx = self._context(resume_from="configure", plan=plan)
        target = self.chromium.src / "chrome" / "browser" / "browseros" / "patched.cc"
        target.parent.mkdir(parents=True)
        target.write_text("first")
        write_step_checkpoint(ctx, _ChromiumReplaceStep(), plan[0][1])
        target.write_text("second")
        write_step_checkpoint(ctx, _StringReplacesStep(), plan[0][1])
        resumed = self._context(resume_from="configure", plan=plan)

        validate_resume_before_execution([(resumed, ("configure",))])

    def _assert_v8_mutation_rejected(self, mutate, *, dirty=True, untracked=False):
        v8 = MockChromium(self.base / "v8-fixture").with_git()
        destination = self.chromium.src / "v8"
        v8.src.rename(destination)
        v8.src = destination
        base = v8._git("rev-parse", "HEAD").stdout.strip()
        self.chromium._git("update-index", "--add", "--cacheinfo", "160000", base, "v8")
        self.chromium._git("commit", "-m", "pin V8")
        target = v8.src / ("untracked.cc" if untracked else "chrome/VERSION")
        if dirty or untracked:
            target.write_text("patched V8")
        plan = (("x64", ("patches", "configure")),)
        ctx = self._context(resume_from="configure", plan=plan)
        write_step_checkpoint(ctx, _PatchesStep(), plan[0][1])
        validate_resume_before_execution(
            [(self._context(resume_from="configure", plan=plan), ("configure",))]
        )
        mutate(v8, target)
        resumed = self._context(resume_from="configure", plan=plan)
        with self.assertRaisesRegex(
            ResumeValidationError, "Chromium mutation digest mismatch"
        ):
            validate_resume_before_execution([(resumed, ("configure",))])

    def test_resume_rejects_changed_v8_bytes_in_already_dirty_checkout(self):
        self._assert_v8_mutation_rejected(
            lambda _v8, path: path.write_text("different V8")
        )

    def test_resume_rejects_changed_v8_head_even_with_identical_files(self):
        self._assert_v8_mutation_rejected(
            lambda v8, _path: v8.commit_all("advance HEAD", allow_empty=True),
            dirty=False,
        )

    def test_resume_rejects_changed_untracked_v8_bytes(self):
        self._assert_v8_mutation_rejected(
            lambda _v8, path: path.write_text("changed untracked"), untracked=True
        )

    def test_resume_rejects_deleted_v8_source(self):
        self._assert_v8_mutation_rejected(lambda _v8, path: path.unlink())

    def test_resume_rejects_changed_v8_file_mode(self):
        self._assert_v8_mutation_rejected(lambda _v8, path: path.chmod(0o755))

    def test_latest_chromium_mutation_checkpoint_still_detects_stale_tree(self):
        plan = (("x64", ("chromium_replace", "string_replaces", "configure")),)
        ctx = self._context(resume_from="configure", plan=plan)
        target = self.chromium.src / "chrome" / "browser" / "browseros" / "patched.cc"
        target.parent.mkdir(parents=True)
        target.write_text("first")
        write_step_checkpoint(ctx, _ChromiumReplaceStep(), plan[0][1])
        target.write_text("second")
        write_step_checkpoint(ctx, _StringReplacesStep(), plan[0][1])
        target.write_text("stale")
        resumed = self._context(resume_from="configure", plan=plan)

        with self.assertRaisesRegex(
            ResumeValidationError, "Chromium mutation digest mismatch"
        ):
            validate_resume_before_execution([(resumed, ("configure",))])


if __name__ == "__main__":
    unittest.main()
