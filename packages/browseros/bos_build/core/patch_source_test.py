"""Patch packages select Chromium inputs without taking over product ownership."""

import tempfile
import unittest
from pathlib import Path
from unittest import mock

import yaml

from .context import Context
from .patch_source import load_external_patch_version, validate_v8_gitlink
from .resume import _context_for_arch
from ..lib.testing import MockBrowserOSRoot, MockChromium
from ..patchkit.extract.common import resolve_base_commit


class PatchSourceTest(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.main = MockBrowserOSRoot(
            Path(temp.name) / "main",
            chromium_version="155.0.8081.12",
            build_offset="258",
            browseros_version="0.51.2.6",
        )
        self.external = MockBrowserOSRoot(
            Path(temp.name) / "internal",
            chromium_version="158.0.8127.18",
            build_offset="999",
            browseros_version="9.9.9.9",
        ).with_patch_store()

    def _context(self, **kwargs):
        return Context(root_dir=self.main.root, patch_root=self.external.root, **kwargs)

    def test_patch_package_changes_upstream_inputs_only(self):
        ctx = self._context()
        self.assertEqual(ctx.chromium_version, "158.0.8127.18")
        self.assertEqual(
            ctx.get_patches_dir(), self.external.root.resolve() / "chromium_patches"
        )
        self.assertEqual(
            ctx.get_series_patches_dir(),
            self.external.root.resolve() / "series_patches",
        )
        self.assertEqual(
            ctx.get_features_yaml_path(),
            self.external.root.resolve() / "chromium_patches/.features.yaml",
        )
        self.assertEqual(resolve_base_commit(ctx, None), "a" * 40)
        self.assertEqual(ctx.browseros_build_offset, "258")
        self.assertEqual(ctx.semantic_version, "0.51.2.6")
        self.assertEqual(
            ctx.get_gn_config_dir(), self.main.root / "bos_build/config/gn"
        )
        self.assertEqual(
            ctx.get_copy_resources_config(),
            self.main.root / "bos_build/config/copy_resources.yaml",
        )
        self.assertEqual(
            ctx.get_download_resources_config(),
            self.main.root / "bos_build/config/download_resources.yaml",
        )
        self.assertTrue(
            all(
                path.is_relative_to(self.main.root)
                for path in ctx.get_chromium_replace_roots()
            )
        )

    def test_default_package_remains_unchanged(self):
        ctx = Context(root_dir=self.main.root)
        self.assertFalse(ctx.uses_external_patches)
        self.assertEqual(ctx.chromium_version, "155.0.8081.12")
        self.assertEqual(
            ctx.get_patches_dir(), self.main.root.resolve() / "chromium_patches"
        )

    def test_explicit_version_cannot_override_external_pin(self):
        with self.assertRaisesRegex(ValueError, "chromium_version disagrees"):
            self._context(chromium_version="155.0.8081.12")

    def test_mismatched_pin_is_rejected(self):
        for name in ("BASE_COMMIT", "CHROMIUM_VERSION"):
            with self.subTest(name=name):
                path = self.external.root / name
                before = path.read_text()
                path.write_text(
                    "b" * 40 if name == "BASE_COMMIT" else before.replace("158", "159")
                )
                with self.assertRaisesRegex(ValueError, "disagree"):
                    load_external_patch_version(self.external.root)
                path.write_text(before)

    def test_incomplete_package_is_rejected(self):
        for name in ("chromium_patches/.features.yaml", "series_patches/series"):
            with self.subTest(name=name):
                path = self.external.root / name
                before = path.read_text()
                path.unlink()
                with self.assertRaisesRegex(ValueError, "Invalid Chromium patch root"):
                    self._context()
                path.write_text(before)

    def test_common_and_all_platform_series_reject_outside_paths(self):
        series = self.external.root / "series_patches"
        outside = self.external.root.parent / "outside.patch"
        outside.write_text("external patch bytes")
        for name in ("series", "series.macos", "series.windows", "series.linux"):
            for entry in (
                "../../outside.patch",
                str(outside),
                "../chromium_patches/patch",
                "C:/shared/fix.patch",
                "C:fix.patch",
                r"..\outside.patch",
                r"\shared\fix.patch",
                "nested/../fix.patch",
            ):
                with self.subTest(name=name, entry=entry):
                    path = series / name
                    path.write_text(entry + "\n")
                    with self.assertRaisesRegex(
                        ValueError, "Series patch path must stay within"
                    ):
                        self._context()
                    path.write_text("")

    def test_contained_series_with_comments_remain_valid(self):
        series = self.external.root / "series_patches"
        (series / "nested").mkdir()
        (series / "nested/fix.patch").write_text("patch bytes")
        (series / "series.macos").write_text(
            "# platform patches\n\nnested/fix.patch # note\n"
        )
        self.assertEqual(self._context().chromium_version, "158.0.8127.18")

    def test_external_input_symlinks_are_rejected(self):
        self.external.with_v8_store()
        self.external.add_patch("chrome/example.cc", "patch")
        nested = self.external.root / "series_patches/nested"
        nested.mkdir()
        for name in (
            "CHROMIUM_VERSION",
            "BASE_COMMIT",
            "chromium_patches",
            "chromium_patches/chrome/example.cc",
            "series_patches/nested",
            "v8_patches",
            "v8_patches/.store.yaml",
        ):
            with self.subTest(name=name):
                path = self.external.root / name
                target = self.external.root.parent / "link-target"
                path.rename(target)
                path.symlink_to(target, target_is_directory=target.is_dir())
                with self.assertRaisesRegex(ValueError, "symlinks are not supported"):
                    self._context()
                path.unlink()
                target.rename(path)

    def test_package_root_symlink_resolves_to_valid_package(self):
        alias = self.external.root.parent / "alias"
        alias.symlink_to(self.external.root, target_is_directory=True)
        ctx = Context(root_dir=self.main.root, patch_root=alias)
        self.assertEqual(ctx.get_patch_root(), self.external.root.resolve())

    def test_v8_metadata_is_validated_without_a_checkout(self):
        self.external.with_v8_store()
        store = self.external.root / "v8_patches"
        self._context()
        cases = (
            (".store.yaml", None),
            (".store.yaml", "[]"),
            (".store.yaml", "["),
            (".store.yaml", "base_commit: short"),
            (
                ".store.yaml",
                yaml.safe_dump({"base_commit": "b" * 40, "base_version": "bad"}),
            ),
            (".features.yaml", None),
            (".features.yaml", "features: []"),
            (".features.yaml", "features: {inspector: {files: wrong}}"),
        )
        for name, invalid in cases:
            with self.subTest(name=name, invalid=invalid):
                path = store / name
                original = path.read_text()
                path.unlink() if invalid is None else path.write_text(invalid)
                with self.assertRaisesRegex(ValueError, "Invalid V8 patch store"):
                    self._context()
                path.write_text(original)

    def test_available_v8_gitlink_is_checked_before_clean(self):
        from ..steps.setup.clean import CleanModule

        chromium = MockChromium(self.external.root.parent / "chromium").with_git()
        base = chromium._git("rev-parse", "HEAD").stdout.strip()
        (self.external.root / "BASE_COMMIT").write_text(base)
        (self.external.root / "chromium_patches/.store.yaml").write_text(
            yaml.safe_dump(
                {
                    "base_commit": base,
                    "base_version": "158.0.8127.18",
                }
            )
        )
        self.external.with_v8_store()
        ctx = self._context(chromium_src=chromium.src)
        from .planner import preflight

        with self.assertRaisesRegex(ValueError, "pinned V8 gitlink"):
            preflight(["clean", "patches"], ctx=ctx)
        with mock.patch.object(CleanModule, "clean_outputs") as clean:
            with self.assertRaisesRegex(ValueError, "pinned V8 gitlink"):
                CleanModule().execute(ctx)
            clean.assert_not_called()
        chromium._git("update-index", "--add", "--cacheinfo", "160000", "b" * 40, "v8")
        chromium._git("commit", "-m", "pin V8")
        base = chromium._git("rev-parse", "HEAD").stdout.strip()
        (self.external.root / "BASE_COMMIT").write_text(base)
        validate_v8_gitlink(self.external.root, chromium.src)

    def test_missing_revision_defers_only_until_provisioning(self):
        self.external.with_v8_store()
        chromium = MockChromium(self.external.root.parent / "chromium").with_git()
        validate_v8_gitlink(self.external.root, chromium.src, allow_missing_base=True)
        with self.assertRaisesRegex(ValueError, "unavailable for V8 validation"):
            validate_v8_gitlink(self.external.root, chromium.src)

    def test_resume_architecture_clone_keeps_external_pin(self):
        sibling = _context_for_arch(self._context(), "arm64")
        self.assertEqual(sibling.get_patch_root(), self.external.root.resolve())
        self.assertEqual(sibling.chromium_version, "158.0.8127.18")


if __name__ == "__main__":
    unittest.main()
