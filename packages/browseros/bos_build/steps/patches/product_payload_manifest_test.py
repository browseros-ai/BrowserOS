#!/usr/bin/env python3
"""Tests for product payload manifest patches."""

import re
import unittest

from ...lib.paths import get_package_root


PATCHES = get_package_root() / "chromium_patches"


def _patched_source(relative_path: str) -> str:
    """Reconstruct the changed source regions from a unified diff."""
    source_lines: list[str] = []
    in_hunk = False

    for line in (PATCHES / relative_path).read_text().splitlines():
        if line.startswith("@@"):
            in_hunk = True
            continue
        if not in_hunk:
            continue
        if line.startswith("diff --git "):
            in_hunk = False
            continue
        if line.startswith(("+", " ")):
            source_lines.append(line[1:])

    return "\n".join(source_lines)


def _source_literals(source: str) -> set[str]:
    return set(re.findall(r'"([^"\n]+\.(?:crx|json))"', source))


def _conditional_sources(source: str, condition: str) -> set[str]:
    match = re.search(
        rf"if \({re.escape(condition)}\) \{{\n(?P<body>.*?)\n\}}",
        source,
        re.DOTALL,
    )
    if match is None:
        raise AssertionError(f"missing GN condition: {condition}")
    return _source_literals(match.group("body"))


class ProductPayloadManifestPatchTest(unittest.TestCase):
    def test_windows_manifest_carries_both_server_layouts(self) -> None:
        manifest = _patched_source("chrome/installer/mini_installer/chrome.release")
        browseros_block = "\n".join(
            (
                "BrowserOSServer\\default\\resources\\*.*: %(VersionDir)s\\BrowserOSServer\\default\\resources\\",
                "BrowserOSServer\\default\\resources\\bin\\*.*: %(VersionDir)s\\BrowserOSServer\\default\\resources\\bin\\",
                "BrowserOSServer\\default\\resources\\bin\\third_party\\*.*: %(VersionDir)s\\BrowserOSServer\\default\\resources\\bin\\third_party\\",
                "BrowserOSServer\\default\\resources\\db\\migrations\\*.*: %(VersionDir)s\\BrowserOSServer\\default\\resources\\db\\migrations\\",
                "BrowserOSServer\\default\\resources\\db\\migrations\\meta\\*.*: %(VersionDir)s\\BrowserOSServer\\default\\resources\\db\\migrations\\meta\\",
            )
        )
        browserclaw_block = "\n".join(
            (
                "BrowserClawServer\\default\\resources\\*.*: %(VersionDir)s\\BrowserClawServer\\default\\resources\\",
                "BrowserClawServer\\default\\resources\\bin\\*.*: %(VersionDir)s\\BrowserClawServer\\default\\resources\\bin\\",
                "BrowserClawServer\\default\\resources\\db\\migrations\\*.*: %(VersionDir)s\\BrowserClawServer\\default\\resources\\db\\migrations\\",
                "BrowserClawServer\\default\\resources\\db\\migrations\\meta\\*.*: %(VersionDir)s\\BrowserClawServer\\default\\resources\\db\\migrations\\meta\\",
            )
        )

        self.assertIn(browseros_block, manifest)
        self.assertIn(browserclaw_block, manifest)
        self.assertNotIn(
            "BrowserClawServer\\default\\resources\\bin\\third_party", manifest
        )

    def test_windows_manifest_carries_bundled_extensions(self) -> None:
        manifest = _patched_source("chrome/installer/mini_installer/chrome.release")

        self.assertIn(
            "browseros_extensions\\*.*: %(VersionDir)s\\browseros_extensions\\",
            manifest.splitlines(),
        )

    def test_gn_sources_follow_product_matrix(self) -> None:
        build = _patched_source("chrome/browser/browseros/bundled_extensions/BUILD.gn")
        assignment = re.search(
            r"_bundled_extensions_sources = \[(?P<body>.*?)\n\]",
            build,
            re.DOTALL,
        )
        self.assertIsNotNone(assignment)
        assert assignment is not None

        manifest = "bundled_extensions.json"
        agent = "bflpfmnmnokmjhmgnolecpppdbdophmk.crx"
        bug_reporter = "adlpneommgkgeanpaekgoaolcpncohkf.crx"
        browserclaw = "pjimfkbpehlcllblajnpfamdfjhhlgkc.crx"
        base_sources = _source_literals(assignment.group("body"))
        browseros_sources = _conditional_sources(
            build,
            "browseros_allow_runtime_product_override || browseros_product_browseros",
        )
        browserclaw_sources = _conditional_sources(
            build,
            "browseros_allow_runtime_product_override || browseros_product_browserclaw",
        )

        self.assertIn('import("//chrome/browser/browseros/buildflags.gni")', build)
        self.assertEqual(base_sources, {manifest, bug_reporter})
        self.assertEqual(browseros_sources, {agent})
        self.assertEqual(browserclaw_sources, {browserclaw})
        self.assertEqual(
            base_sources | browseros_sources,
            {manifest, agent, bug_reporter},
        )
        self.assertEqual(
            base_sources | browserclaw_sources,
            {manifest, browserclaw, bug_reporter},
        )
        self.assertEqual(
            base_sources | browseros_sources | browserclaw_sources,
            {manifest, agent, browserclaw, bug_reporter},
        )


if __name__ == "__main__":
    unittest.main()
