#!/usr/bin/env python3
"""Tests for the shared server-binary sign table."""

import unittest
from pathlib import Path

from .server_binaries import (
    BROWSEROS_CLAW_SERVER_BUNDLE,
    BROWSEROS_SERVER_BUNDLE,
    MACOS_SERVER_BINARIES,
    SERVER_BUNDLES,
    WINDOWS_SERVER_BINARIES,
    expected_windows_bundle_binary_paths,
    expected_windows_binary_paths,
    macos_sign_spec_for,
    server_bundles_for_product,
)

ENTITLEMENTS_DIR = Path(__file__).resolve().parents[2] / "resources" / "entitlements"


class MacosServerBinariesTest(unittest.TestCase):
    def test_server_bundles_have_separate_resource_roots(self):
        self.assertEqual(SERVER_BUNDLES[0], BROWSEROS_SERVER_BUNDLE)
        self.assertEqual(SERVER_BUNDLES[1], BROWSEROS_CLAW_SERVER_BUNDLE)
        self.assertEqual(
            BROWSEROS_SERVER_BUNDLE.local_resources_root,
            Path("resources/binaries/browseros_server"),
        )
        self.assertEqual(
            BROWSEROS_CLAW_SERVER_BUNDLE.local_resources_root,
            Path("resources/binaries/browseros_claw_server"),
        )
        self.assertEqual(
            BROWSEROS_SERVER_BUNDLE.chromium_resources_root,
            Path("chrome/browser/browseros/server/resources"),
        )
        self.assertEqual(
            BROWSEROS_CLAW_SERVER_BUNDLE.chromium_resources_root,
            Path("chrome/browser/browseros/claw_server/resources"),
        )
        self.assertEqual(
            BROWSEROS_SERVER_BUNDLE.macos_bundle_resources_root,
            Path("Contents/Resources/BrowserOSServer/default/resources"),
        )
        self.assertEqual(
            BROWSEROS_CLAW_SERVER_BUNDLE.macos_bundle_resources_root,
            Path("Contents/Resources/BrowserClawServer/default/resources"),
        )
        self.assertTrue(BROWSEROS_SERVER_BUNDLE.required_in_chromium_output)
        self.assertFalse(BROWSEROS_CLAW_SERVER_BUNDLE.required_in_chromium_output)

    def test_server_bundles_filter_by_product(self):
        self.assertEqual(
            server_bundles_for_product("browseros"), (BROWSEROS_SERVER_BUNDLE,)
        )
        self.assertEqual(
            server_bundles_for_product("browserclaw"),
            (BROWSEROS_CLAW_SERVER_BUNDLE,),
        )

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
        spec = macos_sign_spec_for(Path("/x/browseros_server"))
        assert spec is not None
        self.assertEqual(spec.identifier_suffix, "browseros_server")
        self.assertIsNone(macos_sign_spec_for(Path("/x/not_a_known_binary")))

    def test_macos_sign_spec_for_resolves_claw_server_binary(self):
        spec = macos_sign_spec_for(Path("/x/browseros-claw-server"))
        assert spec is not None
        self.assertEqual(spec.identifier_suffix, "browseros_claw_server")
        self.assertEqual(spec.options, "runtime")
        self.assertEqual(
            spec.entitlements, "browseros-executable-entitlements.plist"
        )

    def test_third_party_tool_entries_use_plain_hardened_runtime(self):
        for binary in ["rg"]:
            spec = macos_sign_spec_for(Path(f"/x/{binary}"))
            assert spec is not None
            self.assertEqual(spec.identifier_suffix, binary)
            self.assertEqual(spec.options, "runtime")
            self.assertIsNone(spec.entitlements)
        self.assertIsNone(macos_sign_spec_for(Path("/x/codex")))
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

    def test_expected_windows_binary_paths_joins_root(self):
        root = Path("/tmp/fake/resources/bin")
        resolved = expected_windows_binary_paths(root)
        self.assertEqual(len(resolved), len(WINDOWS_SERVER_BINARIES))
        for rel, abs_path in zip(WINDOWS_SERVER_BINARIES, resolved):
            self.assertEqual(abs_path, root / rel)

    def test_expected_windows_bundle_binary_paths_includes_claw(self):
        build_output_dir = Path("/tmp/out/Default")

        self.assertEqual(
            expected_windows_bundle_binary_paths(build_output_dir),
            [
                build_output_dir
                / "BrowserOSServer"
                / "default"
                / "resources"
                / "bin"
                / "browseros_server.exe",
                build_output_dir
                / "BrowserClawServer"
                / "default"
                / "resources"
                / "bin"
                / "browseros-claw-server.exe",
            ],
        )

    def test_expected_windows_bundle_binary_paths_can_filter_by_product(self):
        build_output_dir = Path("/tmp/out/Default")

        self.assertEqual(
            expected_windows_bundle_binary_paths(build_output_dir, "browserclaw"),
            [
                build_output_dir
                / "BrowserClawServer"
                / "default"
                / "resources"
                / "bin"
                / "browseros-claw-server.exe",
            ],
        )

    def test_windows_has_no_stale_third_party(self):
        forbidden = {
            "third_party/podman/podman.exe",
            "third_party/podman/gvproxy.exe",
            "third_party/podman/win-sshproxy.exe",
            "third_party/bun.exe",
            "third_party/rg.exe",
            "third_party/codex.exe",
            "third_party/claude.exe",
        }
        leftover = forbidden & set(WINDOWS_SERVER_BINARIES)
        self.assertFalse(leftover, f"stale entries still present: {leftover}")


if __name__ == "__main__":
    unittest.main()
