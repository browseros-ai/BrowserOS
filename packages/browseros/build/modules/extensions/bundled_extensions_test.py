#!/usr/bin/env python3
"""Tests for bundled extension manifest handling."""

import unittest
from pathlib import Path

from build.modules.extensions.bundled_extensions import BundledExtensionsModule


class BundledExtensionsManifestTest(unittest.TestCase):
    def test_bundled_manifest_parses_requested_alpha_entries(self) -> None:
        repo_root = Path(__file__).resolve().parents[5]
        manifest_path = repo_root / "updates" / "extensions" / "bundled-manifest.xml"

        extensions = BundledExtensionsModule()._parse_manifest_xml(
            manifest_path.read_text()
        )

        self.assertEqual(
            extensions,
            [
                (
                    "adlpneommgkgeanpaekgoaolcpncohkf",
                    "52.0.0.0",
                    "https://cdn.browseros.com/extensions/bugreporter-52.0.0.0.crx",
                ),
                (
                    "bflpfmnmnokmjhmgnolecpppdbdophmk",
                    "0.0.115.0",
                    "https://cdn.browseros.com/extensions/agent-0.0.115.0.crx",
                ),
            ],
        )


if __name__ == "__main__":
    unittest.main()
