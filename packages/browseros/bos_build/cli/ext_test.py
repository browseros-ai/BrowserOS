#!/usr/bin/env python3
"""CLI surface tests for extension releases."""

import re
import unittest

from typer.testing import CliRunner

from bos_build.browseros import app

runner = CliRunner()
ANSI_RE = re.compile(r"\x1b\[[0-9;]*m")


class ExtensionReleaseHelpTest(unittest.TestCase):
    def test_release_help_is_crx_only(self):
        result = runner.invoke(app, ["ext", "release", "--help"])

        self.assertEqual(result.exit_code, 0, result.output)
        help_text = ANSI_RE.sub("", result.output)
        self.assertIn("browseros release extensions", help_text)
        options = help_text.split("Options", 1)[1]
        self.assertNotIn("--channel", options)
        self.assertNotIn("--publish-manifest", options)


if __name__ == "__main__":
    unittest.main()
