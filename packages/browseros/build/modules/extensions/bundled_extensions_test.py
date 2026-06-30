#!/usr/bin/env python3
"""Tests for bundled extension manifest handling."""

import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from typing import cast

from build.common.context import Context
from build.common.products import get_product_descriptor
from build.modules.extensions.bundled_extensions import (
    BundledExtensionsModule,
    ExtensionInfo,
)

CLAW_EXTENSION_ID = "pjimfkbpehlcllblajnpfamdfjhhlgkc"


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
                (
                    CLAW_EXTENSION_ID,
                    "0.0.1",
                    "https://cdn.browseros.com/extensions/browserclaw-0.0.1.crx",
                ),
            ],
        )

    def test_required_ids_cover_agent_bug_reporter_and_claw(self) -> None:
        self.assertEqual(
            dict(get_product_descriptor("browseros").required_extension_ids),
            {
                "adlpneommgkgeanpaekgoaolcpncohkf": "BrowserOS bug reporter",
                "bflpfmnmnokmjhmgnolecpppdbdophmk": "BrowserOS agent",
            },
        )
        self.assertEqual(
            dict(get_product_descriptor("browserclaw").required_extension_ids),
            {CLAW_EXTENSION_ID: "BrowserClaw app"},
        )

    def test_generated_json_maps_claw_id_to_crx(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            output_dir = Path(tmp)
            BundledExtensionsModule()._generate_json(
                [
                    ExtensionInfo(
                        id=CLAW_EXTENSION_ID,
                        version="0.0.1",
                        codebase=(
                            "https://cdn.browseros.com/extensions/"
                            "browserclaw-0.0.1.crx"
                        ),
                    )
                ],
                output_dir,
            )

            data = json.loads((output_dir / "bundled_extensions.json").read_text())

        self.assertEqual(
            data[CLAW_EXTENSION_ID],
            {
                "external_crx": f"{CLAW_EXTENSION_ID}.crx",
                "external_version": "0.0.1",
            },
        )

    def test_missing_claw_app_fails_validation(self) -> None:
        extensions = [
            ExtensionInfo(
                id="adlpneommgkgeanpaekgoaolcpncohkf",
                version="52.0.0.0",
                codebase="https://cdn.browseros.com/extensions/bugreporter.crx",
            ),
            ExtensionInfo(
                id="bflpfmnmnokmjhmgnolecpppdbdophmk",
                version="0.0.115.0",
                codebase="https://cdn.browseros.com/extensions/agent.crx",
            ),
        ]

        with self.assertRaisesRegex(
            RuntimeError,
            f"BrowserClaw app \\({CLAW_EXTENSION_ID}\\)",
        ):
            BundledExtensionsModule()._validate_required_extensions(
                extensions, self._ctx("browserclaw")
            )

    def test_browserclaw_selects_only_claw_extension(self) -> None:
        extensions = [
            ExtensionInfo(
                id="adlpneommgkgeanpaekgoaolcpncohkf",
                version="52.0.0.0",
                codebase="https://cdn.browseros.com/extensions/bugreporter.crx",
            ),
            ExtensionInfo(
                id="bflpfmnmnokmjhmgnolecpppdbdophmk",
                version="0.0.115.0",
                codebase="https://cdn.browseros.com/extensions/agent.crx",
            ),
            ExtensionInfo(
                id=CLAW_EXTENSION_ID,
                version="0.0.1",
                codebase="https://cdn.browseros.com/extensions/browserclaw.crx",
            ),
        ]

        selected = BundledExtensionsModule()._select_product_extensions(
            extensions, self._ctx("browserclaw")
        )

        self.assertEqual([ext.id for ext in selected], [CLAW_EXTENSION_ID])

    def _ctx(self, product: str):
        return cast(
            Context, SimpleNamespace(product=get_product_descriptor(product))
        )


if __name__ == "__main__":
    unittest.main()
