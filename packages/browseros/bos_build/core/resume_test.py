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
                "BROWSEROS_SERVER_RESOURCE_VERSION": "0.0.138",
                "BUNDLED_PRODUCT_EXTENSION_VERSION": "0.0.132.0",
                "BROWSERCLAW_ONBOARD_RESOURCE_VERSION": "0.0.36",
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

    def _context(self, resume_from: str = "sign_macos") -> Context:
        ctx = Context(
            root_dir=self.browseros.root,
            chromium_src=self.chromium.src,
            architecture="x64",
            plan_architectures=("x64",),
            build_type="release",
        )
        ctx.resume_state = make_resume_state(
            ctx,
            self.plan,
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

    def test_unpinned_published_resources_make_strict_resume_unprovable(self):
        with mock.patch.dict(os.environ, {}, clear=True):
            resumed = self._context()

        with self.assertRaisesRegex(ResumeValidationError, "exact component pins"):
            validate_resume_before_execution([(resumed, ("sign_macos",))])


if __name__ == "__main__":
    unittest.main()
