#!/usr/bin/env python3
"""Tests for the merge_universal step."""

import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from . import universal
from .universal import UNIVERSAL_ARCHITECTURES, MergeUniversalModule
from ...core.context import Context
from ...core.step import ValidationError, all_steps


def _universal_ctx(chromium_src: Path) -> Context:
    return Context(
        chromium_src=chromium_src,
        architecture="universal",
        build_type="release",
    )


def _arch_app(ctx: Context, arch: str) -> Path:
    """Expected input path, derived independently via the Context contract."""
    return Context(
        root_dir=ctx.root_dir,
        chromium_src=ctx.chromium_src,
        architecture=arch,
        build_type=ctx.build_type,
        product=ctx.product,
    ).get_app_path()


class MergeUniversalTestBase(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.ctx = _universal_ctx(Path(tmp.name))

    def _create_arch_apps(self, arches=UNIVERSAL_ARCHITECTURES) -> list[Path]:
        apps = []
        for arch in arches:
            app = _arch_app(self.ctx, arch)
            app.mkdir(parents=True)
            apps.append(app)
        return apps


class ValidateTest(MergeUniversalTestBase):
    def test_rejects_non_universal_context(self):
        arch_ctx = Context(
            chromium_src=self.ctx.chromium_src,
            architecture="arm64",
            build_type="release",
        )
        with self.assertRaisesRegex(ValidationError, "universal"):
            MergeUniversalModule().validate(arch_ctx)

    def test_rejects_missing_arch_app_by_name(self):
        self._create_arch_apps(arches=("arm64",))
        with self.assertRaisesRegex(ValidationError, "x64"):
            MergeUniversalModule().validate(self.ctx)

    def test_passes_when_both_arch_apps_exist(self):
        self._create_arch_apps()
        MergeUniversalModule().validate(self.ctx)


class ExecuteTest(MergeUniversalTestBase):
    def test_merges_arch_apps_into_universal_app_path(self):
        arm64_app, x64_app = self._create_arch_apps()
        with mock.patch.object(
            universal, "merge_architectures", return_value=True
        ) as merge:
            MergeUniversalModule().execute(self.ctx)

        merge.assert_called_once_with(
            arch1_path=arm64_app,
            arch2_path=x64_app,
            output_path=self.ctx.get_app_path(),
            universalizer_script=self.ctx.root_dir
            / "bos_build/steps/package/universalizer_patched.py",
        )
        self.assertEqual(
            self.ctx.artifact_registry.get("built_app"), self.ctx.get_app_path()
        )

    def test_raises_when_merge_fails(self):
        self._create_arch_apps()
        with mock.patch.object(universal, "merge_architectures", return_value=False):
            with self.assertRaisesRegex(RuntimeError, "merge"):
                MergeUniversalModule().execute(self.ctx)
        self.assertFalse(self.ctx.artifact_registry.has("built_app"))


class PreflightTest(MergeUniversalTestBase):
    def test_passes_against_vendored_universalizer(self):
        MergeUniversalModule().preflight(self.ctx)

    def test_rejects_missing_universalizer(self):
        missing = self.ctx.chromium_src / "missing_universalizer.py"
        with mock.patch.object(
            universal, "_universalizer_script", return_value=missing
        ):
            with self.assertRaisesRegex(ValidationError, "not found"):
                MergeUniversalModule().preflight(self.ctx)


class ServerBundleSkewTest(unittest.TestCase):
    """The merge-time guard against version-skewed per-arch server bundles."""

    def _write_bundle(
        self,
        family_dir: Path,
        arch_dir: str,
        version: str,
        generated_at: str = "2026-01-01T00:00:00.000Z",
    ) -> None:
        arch_path = family_dir / arch_dir
        arch_path.mkdir(parents=True)
        (arch_path / "artifact-metadata.json").write_text(
            json.dumps(
                {"version": version, "generatedAt": generated_at, "files": []}
            )
        )

    def test_matching_versions_pass(self):
        with tempfile.TemporaryDirectory() as tmp:
            binaries = Path(tmp)
            family = binaries / "browseros_server"
            self._write_bundle(family, "darwin-arm64", "0.0.10")
            self._write_bundle(family, "darwin-x64", "0.0.10")

            universal._assert_server_bundles_aligned(binaries)  # no raise

    def test_mismatched_versions_raise_with_both_versions_and_timestamps(self):
        # Mirrors the BrowserClaw 0.47.11 skew: fresh arm64 vs stale x64.
        with tempfile.TemporaryDirectory() as tmp:
            binaries = Path(tmp)
            family = binaries / "browseros_claw_server"
            self._write_bundle(
                family, "darwin-arm64", "0.0.10", "2026-07-14T00:00:00.000Z"
            )
            self._write_bundle(
                family, "darwin-x64", "0.0.3", "2026-06-30T00:00:00.000Z"
            )

            with self.assertRaises(ValidationError) as caught:
                universal._assert_server_bundles_aligned(binaries)

            message = str(caught.exception)
            self.assertIn("browseros_claw_server", message)
            self.assertIn("0.0.10", message)
            self.assertIn("0.0.3", message)
            self.assertIn("2026-07-14T00:00:00.000Z", message)
            self.assertIn("2026-06-30T00:00:00.000Z", message)

    def test_absent_x64_metadata_skips_family(self):
        with tempfile.TemporaryDirectory() as tmp:
            binaries = Path(tmp)
            family = binaries / "browseros_server"
            self._write_bundle(family, "darwin-arm64", "0.0.10")
            # x64 dir exists but carries no metadata (partial/older layout).
            (family / "darwin-x64").mkdir(parents=True)

            universal._assert_server_bundles_aligned(binaries)  # no raise

    def test_missing_binaries_dir_is_noop(self):
        with tempfile.TemporaryDirectory() as tmp:
            universal._assert_server_bundles_aligned(Path(tmp) / "absent")  # no raise


class RegistrationTest(unittest.TestCase):
    def test_step_metadata(self):
        cls = all_steps()["merge_universal"]
        self.assertIs(cls, MergeUniversalModule)
        self.assertEqual(cls.phase, "build")
        self.assertEqual(cls.platforms, ("macos",))
        self.assertTrue(cls.optional)
        self.assertEqual(cls.env, ())
        self.assertEqual(cls.produces, ["built_app"])
        self.assertEqual(cls.requires, [])


if __name__ == "__main__":
    unittest.main()
