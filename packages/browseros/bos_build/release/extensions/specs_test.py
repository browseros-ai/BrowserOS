#!/usr/bin/env python3
"""Tests for the extension packaging spec table."""

import unittest

from ...core.products import (
    BROWSEROS_AGENT_EXTENSION_ID,
    BROWSEROS_BUG_REPORTER_EXTENSION_ID,
    BROWSEROS_CONTROLLER_EXTENSION_ID,
    BROWSERCLAW_EXTENSION_ID,
)
from ...lib.paths import get_package_root
from ..feeds.spec import EXTENSIONS as FEED_EXTENSIONS
from .specs import (
    EXTENSION_SPECS,
    ExternalRepoSource,
    InRepoSource,
    select_specs,
    spec_by_name,
)

PRODUCT_IDS = {
    BROWSEROS_AGENT_EXTENSION_ID,
    BROWSEROS_BUG_REPORTER_EXTENSION_ID,
    BROWSEROS_CONTROLLER_EXTENSION_ID,
    BROWSERCLAW_EXTENSION_ID,
}


class SpecTableTest(unittest.TestCase):
    def test_table_holds_exactly_the_four_extensions(self):
        self.assertEqual(
            {spec.name for spec in EXTENSION_SPECS},
            {"agent", "controller", "bugreporter", "browserclaw"},
        )

    def test_names_ids_and_signing_envs_are_unique(self):
        names = [spec.name for spec in EXTENSION_SPECS]
        ids = [spec.extension_id for spec in EXTENSION_SPECS]
        keys = [spec.signing_key_env for spec in EXTENSION_SPECS]
        self.assertEqual(len(names), len(set(names)))
        self.assertEqual(len(ids), len(set(ids)))
        self.assertEqual(len(keys), len(set(keys)))

    def test_every_id_is_a_products_py_constant(self):
        for spec in EXTENSION_SPECS:
            self.assertIn(spec.extension_id, PRODUCT_IDS, spec.name)

    def test_controller_id_constant_value(self):
        self.assertEqual(
            BROWSEROS_CONTROLLER_EXTENSION_ID,
            "nlnihljpboknmfagkikhkdblbedophja",
        )
        self.assertEqual(
            spec_by_name("controller").extension_id,
            BROWSEROS_CONTROLLER_EXTENSION_ID,
        )

    def test_monorepo_extensions_are_in_repo_sources(self):
        monorepo_root = get_package_root().parent.parent
        for name in ("agent", "browserclaw"):
            source = spec_by_name(name).source
            self.assertIsInstance(source, InRepoSource, name)
            self.assertEqual(source.path, "packages/browseros-agent")
            self.assertTrue((monorepo_root / source.path).is_dir())

    def test_external_extensions_keep_their_repos(self):
        controller = spec_by_name("controller").source
        bugreporter = spec_by_name("bugreporter").source
        self.assertEqual(
            controller,
            ExternalRepoSource(repo="browseros-ai/BrowserOS-agent", branch="main"),
        )
        self.assertEqual(
            bugreporter,
            ExternalRepoSource(
                repo="browseros-ai/BrowserOS-feedback-extension", branch="main"
            ),
        )

    def test_crx_key_matches_feeds_spec_formula(self):
        feed_by_name = {ext.name: ext for ext in FEED_EXTENSIONS}
        for spec in EXTENSION_SPECS:
            self.assertEqual(
                spec.crx_key("1.2.3.4"), f"extensions/{spec.name}-1.2.3.4.crx"
            )
            feed_ext = feed_by_name.get(spec.name)
            if feed_ext is not None:
                self.assertEqual(spec.crx_key("9.9.9"), feed_ext.crx_key("9.9.9"))
                self.assertEqual(spec.extension_id, feed_ext.extension_id)

    def test_controller_is_not_a_feed_extension(self):
        self.assertNotIn(
            "controller", {ext.name for ext in FEED_EXTENSIONS}
        )

    def test_paths_are_relative_and_point_at_build_outputs(self):
        for spec in EXTENSION_SPECS:
            for path in (spec.dist_path, spec.manifest_path):
                self.assertFalse(path.startswith("/"), spec.name)
        for name in ("agent", "browserclaw"):
            spec = spec_by_name(name)
            self.assertTrue(spec.dist_path.endswith("dist/chrome-mv3"), name)
            self.assertTrue(spec.manifest_path.endswith("package.json"), name)
            self.assertTrue(spec.env_dir)

    def test_browserclaw_forwards_posthog_env_to_claw_app(self):
        browserclaw = spec_by_name("browserclaw")
        self.assertEqual(
            browserclaw.env,
            (
                "VITE_CLAW_POSTHOG_KEY",
                "VITE_CLAW_POSTHOG_HOST",
                "NODE_ENV",
            ),
        )
        self.assertEqual(browserclaw.required_env, ("VITE_CLAW_POSTHOG_KEY",))
        self.assertEqual(browserclaw.env_dir, "apps/claw-app")

    def test_non_claw_extensions_have_no_required_build_env(self):
        for name in ("agent", "controller", "bugreporter"):
            self.assertEqual(spec_by_name(name).required_env, (), name)

    def test_in_repo_manifest_paths_exist_in_working_tree(self):
        monorepo_root = get_package_root().parent.parent
        for name in ("agent", "browserclaw"):
            spec = spec_by_name(name)
            source = spec.source
            self.assertIsInstance(source, InRepoSource)
            manifest = monorepo_root / source.path / spec.manifest_path
            self.assertTrue(manifest.is_file(), str(manifest))

    def test_spec_by_name_rejects_unknown_with_valid_names(self):
        with self.assertRaisesRegex(ValueError, "agent.*browserclaw|browserclaw.*agent"):
            spec_by_name("nope")

    def test_select_specs_all_and_single(self):
        self.assertEqual(select_specs(None), EXTENSION_SPECS)
        self.assertEqual(select_specs("agent"), (spec_by_name("agent"),))
        with self.assertRaises(ValueError):
            select_specs("agent-v2")


if __name__ == "__main__":
    unittest.main()
