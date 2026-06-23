#!/usr/bin/env python3
"""Tests for the shared server-binary sign table."""

import unittest
from pathlib import Path

from .server_binaries import (
    MACOS_SERVER_BINARIES,
    WINDOWS_SERVER_BINARIES,
    expected_windows_binary_paths,
    macos_sign_spec_for,
)

ENTITLEMENTS_DIR = Path(__file__).resolve().parents[2] / "resources" / "entitlements"


class MacosServerBinariesTest(unittest.TestCase):
    def test_every_entry_has_identifier_and_options(self):
        for stem, spec in MACOS_SERVER_BINARIES.items():
            self.assertTrue(spec.identifier_suffix, f"{stem} missing identifier_suffix")
            self.assertTrue(spec.options, f"{stem} missing options")

    def test_every_entitlements_plist_exists_on_disk(self):
        for stem, spec in MACOS_SERVER_BINARIES.items():
            if spec.entitlements is None:
                continue
            plist = ENTITLEMENTS_DIR / spec.entitlements
            self.assertTrue(plist.exists(), f"{stem}: entitlements {plist} missing")

    def test_macos_sign_spec_for_resolves_by_stem(self):
        spec = macos_sign_spec_for(Path("/x/codex"))
        assert spec is not None
        self.assertEqual(spec.identifier_suffix, "codex")
        self.assertIsNone(macos_sign_spec_for(Path("/x/not_a_known_binary")))

    def test_agent_cli_entries_use_plain_hardened_runtime(self):
        for binary in ["codex"]:
            spec = macos_sign_spec_for(Path(f"/x/{binary}"))
            assert spec is not None
            self.assertEqual(spec.identifier_suffix, binary)
            self.assertEqual(spec.options, "runtime")
            self.assertIsNone(spec.entitlements)
        self.assertIsNone(macos_sign_spec_for(Path("/x/claude")))

    def test_lima_is_not_registered_for_signing(self):
        keys = set(MACOS_SERVER_BINARIES.keys())
        forbidden = {
            "limactl",
            "podman",
            "gvproxy",
            "vfkit",
            "krunkit",
            "podman-mac-helper",
        }
        leftover = forbidden & keys
        self.assertFalse(leftover, f"stale VM entries still present: {leftover}")
        self.assertIsNone(macos_sign_spec_for(Path("/x/third_party/lima/bin/limactl")))


class WindowsServerBinariesTest(unittest.TestCase):
    def test_no_duplicates(self):
        self.assertEqual(
            len(WINDOWS_SERVER_BINARIES), len(set(WINDOWS_SERVER_BINARIES))
        )

    def test_paths_within_expected_layout(self):
        for rel in WINDOWS_SERVER_BINARIES:
            self.assertTrue(
                rel == "browseros_server.exe" or rel.startswith("third_party/"),
                f"{rel} outside expected layout",
            )

    def test_windows_includes_agent_cli_entries(self):
        self.assertIn("third_party/codex.exe", WINDOWS_SERVER_BINARIES)

    def test_expected_windows_binary_paths_joins_root(self):
        root = Path("/tmp/fake/resources/bin")
        resolved = expected_windows_binary_paths(root)
        self.assertEqual(len(resolved), len(WINDOWS_SERVER_BINARIES))
        for rel, abs_path in zip(WINDOWS_SERVER_BINARIES, resolved):
            self.assertEqual(abs_path, root / rel)

    def test_windows_has_no_stale_third_party(self):
        forbidden = {
            "third_party/podman/podman.exe",
            "third_party/podman/gvproxy.exe",
            "third_party/podman/win-sshproxy.exe",
            "third_party/bun.exe",
            "third_party/rg.exe",
            "third_party/claude.exe",
        }
        leftover = forbidden & set(WINDOWS_SERVER_BINARIES)
        self.assertFalse(leftover, f"stale entries still present: {leftover}")


if __name__ == "__main__":
    unittest.main()
