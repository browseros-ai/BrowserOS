#!/usr/bin/env python3
"""Tests for the product doctor."""

import tempfile
import unittest
from pathlib import Path

from bos_build.lib.paths import get_package_root
from bos_build.core.products import ProductDescriptor
from bos_build.products import PRODUCTS
from bos_build.products.doctor import (
    REQUIRED_OVERLAY_FILES,
    check_product,
    check_uniqueness,
    diagnose,
)


def _mock_root(product_ids) -> Path:
    tmp = tempfile.mkdtemp()
    root = Path(tmp)
    for pid in product_ids:
        overlay = root / "chromium_files" / "products" / pid
        for rel in REQUIRED_OVERLAY_FILES:
            path = overlay / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("x")
    return root


class CheckProductTest(unittest.TestCase):
    def test_complete_product_is_healthy(self):
        root = _mock_root(["browseros"])
        self.assertEqual(check_product(PRODUCTS["browseros"], root), [])

    def test_missing_overlay_dir_reported_with_product_id(self):
        root = _mock_root([])
        findings = check_product(PRODUCTS["browserclaw"], root)
        self.assertEqual(len(findings), 1)
        self.assertIn("browserclaw", findings[0])
        self.assertIn("overlay dir missing", findings[0])

    def test_each_missing_branding_file_reported(self):
        root = _mock_root(["browseros"])
        (
            root
            / "chromium_files"
            / "products"
            / "browseros"
            / "chrome"
            / "updater"
            / "branding.gni"
        ).unlink()
        findings = check_product(PRODUCTS["browseros"], root)
        self.assertEqual(len(findings), 1)
        self.assertIn("chrome/updater/branding.gni", findings[0])

    def test_malformed_guid_and_extension_id_reported(self):
        bad = ProductDescriptor.define(
            id="badfox",
            display_name="BadFox",
            windows_installer_guid="not-a-guid",
            summary="s",
            description="d",
            required_extensions=(("tooshort", "Bad ext"),),
        )
        root = _mock_root(["badfox"])
        findings = check_product(bad, root)
        self.assertTrue(any("malformed extension id" in f for f in findings))
        self.assertTrue(any("malformed windows installer GUID" in f for f in findings))

    def test_repo_products_pass_against_real_tree(self):
        # The shipped products must stay healthy against the actual repo.
        self.assertEqual(diagnose(get_package_root()), [])


class UniquenessTest(unittest.TestCase):
    def test_registered_products_have_no_collisions(self):
        self.assertEqual(check_uniqueness(list(PRODUCTS.values())), [])

    def test_duplicate_bundle_id_detected(self):
        clone = ProductDescriptor.define(
            id="browseros2",
            display_name="BrowserOS",  # same display name → same bundle_id
            windows_installer_guid="{11111111-2222-3333-4444-555555555555}",
            summary="s",
            description="d",
        )
        findings = check_uniqueness([PRODUCTS["browseros"], clone])
        self.assertTrue(any("duplicate mac bundle_id" in f for f in findings))


if __name__ == "__main__":
    unittest.main()
