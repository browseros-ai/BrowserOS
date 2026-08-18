#!/usr/bin/env python3
"""Reusable extension build tests."""

import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from bos_build.release.extensions.build import build_extension_crx
from bos_build.release.extensions.specs import spec_by_name


MODULE = "bos_build.release.extensions.build"


class ExtensionBuildTest(unittest.TestCase):
    def test_builds_and_packs_without_stamping_candidate_checkout(self) -> None:
        spec = spec_by_name("agent")
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            source = root / "packages/browseros-agent"
            manifest = source / spec.manifest_path
            dist = source / spec.dist_path
            manifest.parent.mkdir(parents=True)
            manifest.write_text(json.dumps({"version": "0.0.101"}))
            dist.mkdir(parents=True)
            (dist / "manifest.json").write_text(
                json.dumps(
                    {
                        "version": "0.0.101",
                        "update_url": "https://cdn.browseros.com/extensions/update-manifest.xml",
                    }
                )
            )
            output = root / "agent.crx"

            with (
                patch(f"{MODULE}.resolve_source", return_value=source),
                patch(f"{MODULE}.write_env_file"),
                patch(f"{MODULE}.run_command") as run,
                patch(f"{MODULE}.require_env", return_value="private-key"),
                patch(f"{MODULE}.pack_crx") as pack,
            ):
                pack.side_effect = lambda dist, key, chrome, path: (
                    path.write_bytes(b"crx") or path
                )
                built = build_extension_crx(
                    spec=spec,
                    version="0.0.101.0",
                    output_path=output,
                    monorepo_root=root,
                    work_root=root / "work",
                    chrome_binary="chrome",
                    stamp_version=False,
                )

            self.assertEqual(json.loads(manifest.read_text())["version"], "0.0.101")
            self.assertEqual(
                json.loads((dist / "manifest.json").read_text())["version"],
                "0.0.101.0",
            )
            self.assertEqual(built.path, output)
            self.assertEqual(built.version, "0.0.101.0")
            self.assertEqual(run.call_count, 2)
            pack.assert_called_once()

    def test_standalone_build_stamps_only_the_built_manifest(self) -> None:
        spec = spec_by_name("browserclaw")
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            source = root / "packages/browseros-agent"
            manifest = source / spec.manifest_path
            dist = source / spec.dist_path
            manifest.parent.mkdir(parents=True)
            manifest.write_text(json.dumps({"version": "0.1.7"}))
            dist.mkdir(parents=True)
            (dist / "manifest.json").write_text(
                json.dumps(
                    {
                        "version": "0.1.7",
                        "update_url": "https://cdn.browseros.com/extensions/update-manifest.xml",
                    }
                )
            )

            with (
                patch(f"{MODULE}.resolve_source", return_value=source),
                patch(f"{MODULE}.write_env_file"),
                patch(f"{MODULE}.run_command"),
                patch(f"{MODULE}.require_env", return_value="private-key"),
                patch(f"{MODULE}.pack_crx") as pack,
            ):
                pack.side_effect = lambda dist, key, chrome, path: (
                    path.write_bytes(b"crx") or path
                )
                build_extension_crx(
                    spec=spec,
                    version="0.1.8.0",
                    output_path=root / "browserclaw.crx",
                    monorepo_root=root,
                    work_root=root / "work",
                    chrome_binary="chrome",
                    stamp_version=True,
                )

            self.assertEqual(json.loads(manifest.read_text())["version"], "0.1.7")
            self.assertEqual(
                json.loads((dist / "manifest.json").read_text())["version"],
                "0.1.8.0",
            )

    def test_refuses_version_drift_before_build(self) -> None:
        spec = spec_by_name("agent")
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            source = root / "packages/browseros-agent"
            manifest = source / spec.manifest_path
            manifest.parent.mkdir(parents=True)
            manifest.write_text(json.dumps({"version": "0.0.100"}))

            with patch(f"{MODULE}.resolve_source", return_value=source):
                with self.assertRaisesRegex(ValueError, "version"):
                    build_extension_crx(
                        spec=spec,
                        version="0.0.101.0",
                        output_path=root / "agent.crx",
                        monorepo_root=root,
                        work_root=root / "work",
                        chrome_binary="chrome",
                        stamp_version=False,
                    )


if __name__ == "__main__":
    unittest.main()
