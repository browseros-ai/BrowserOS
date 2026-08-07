#!/usr/bin/env python3
"""CLI surface tests for extension releases."""

import json
import re
import tempfile
import unittest
from pathlib import Path

from typer.testing import CliRunner

from bos_build.browseros import app
from bos_build.release.extensions.manifests import render_update_manifest

runner = CliRunner()
ANSI_RE = re.compile(r"\x1b\[[0-9;]*m")


class ExtensionReleaseHelpTest(unittest.TestCase):
    def test_release_help_is_crx_only(self):
        result = runner.invoke(app, ["ext", "release", "--help"])

        self.assertEqual(result.exit_code, 0, result.output)
        help_text = ANSI_RE.sub("", result.output)
        self.assertIn("browseros release extensions", help_text)
        options = help_text.split("Options", 1)[1]
        self.assertIn("--source-sha", options)
        self.assertNotIn("--channel", options)
        self.assertNotIn("--publish-manifest", options)

    def test_resolve_version_writes_reusable_workflow_outputs(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            records = root / "records.json"
            records.write_text("[]")
            manifest = root / "bundled.xml"
            manifest.write_text(render_update_manifest({"browserclaw": "0.1.6.0"}))
            output = root / "github-output"

            result = runner.invoke(
                app,
                [
                    "ext",
                    "resolve-version",
                    "--name",
                    "browserclaw",
                    "--source-sha",
                    "abc123",
                    "--release-records",
                    str(records),
                    "--manifest",
                    str(manifest),
                    "--github-output",
                    str(output),
                ],
            )

            self.assertEqual(result.exit_code, 0, result.output)
            values = dict(
                line.split("=", 1) for line in output.read_text().splitlines()
            )
            self.assertEqual(
                values,
                {
                    "version": "0.1.7.0",
                    "tag": "ext-browserclaw/v0.1.7.0",
                    "release_sha": "abc123",
                    "names": "browserclaw",
                },
            )

    def test_resolve_version_refuses_external_automatic_release(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            records = Path(temp_dir) / "records.json"
            records.write_text(json.dumps([]))

            result = runner.invoke(
                app,
                [
                    "ext",
                    "resolve-version",
                    "--name",
                    "controller",
                    "--source-sha",
                    "abc123",
                    "--release-records",
                    str(records),
                ],
            )

        self.assertNotEqual(result.exit_code, 0)
        self.assertIn("explicit version", result.output)


if __name__ == "__main__":
    unittest.main()
