#!/usr/bin/env python3
"""Tests for the shared server-binary sign table."""

import unittest
from pathlib import Path

from .server_binaries import (
    all_server_bundles,
    expected_windows_bundle_binary_paths,
    expected_windows_binary_paths,
    macos_sign_spec_for,
    server_ota_bundles_for_product,
    server_bundles_for_product,
)
from .browserclaw.product import BROWSERCLAW_SERVER_BUNDLE

SERVER_BUNDLES = all_server_bundles()
MACOS_SERVER_BINARIES = {
    stem: spec
    for bundle in SERVER_BUNDLES
    for stem, spec in bundle.macos_binaries.items()
}
WINDOWS_SERVER_BINARIES = [
    binary for bundle in SERVER_BUNDLES for binary in bundle.windows_binaries
]

ENTITLEMENTS_DIR = Path(__file__).resolve().parents[2] / "resources" / "entitlements"


class MacosServerBinariesTest(unittest.TestCase):
    def test_server_bundles_use_rust_claw_for_browser_builds(self):
        self.assertEqual(all_server_bundles(), (BROWSERCLAW_SERVER_BUNDLE,))

    def test_neo_server_bundle_resource_roots(self):
        self.assertEqual(
            BROWSERCLAW_SERVER_BUNDLE.local_resources_root,
            Path("resources/binaries/browseros_claw_server_rust"),
        )
        self.assertEqual(
            BROWSERCLAW_SERVER_BUNDLE.chromium_resources_root,
            Path("chrome/browser/browseros/claw_server/resources"),
        )
        self.assertEqual(
            BROWSERCLAW_SERVER_BUNDLE.macos_bundle_resources_root,
            Path("Contents/Resources/BrowserClawServer/default/resources"),
        )
        self.assertFalse(BROWSERCLAW_SERVER_BUNDLE.required_in_chromium_output)
        self.assertEqual(BROWSERCLAW_SERVER_BUNDLE.source_builder, "cargo")
        self.assertEqual(
            BROWSERCLAW_SERVER_BUNDLE.source_component, "claw-server-rust"
        )
        self.assertEqual(
            BROWSERCLAW_SERVER_BUNDLE.runtime_binary_name,
            "browseros-claw-server",
        )
        self.assertEqual(
            BROWSERCLAW_SERVER_BUNDLE.unsigned_artifact_key("darwin-arm64"),
            "claw-server-rust/prod-resources/latest/browseros-claw-server-rust-resources-darwin-arm64.zip",
        )
        self.assertEqual(
            BROWSERCLAW_SERVER_BUNDLE.unsigned_artifact_key(
                "darwin-arm64", version="0.0.9"
            ),
            "claw-server-rust/prod-resources/0.0.9/browseros-claw-server-rust-resources-darwin-arm64.zip",
        )

    def test_server_bundles_filter_by_product(self):
        self.assertEqual(
            server_bundles_for_product("browserclaw"),
            (BROWSERCLAW_SERVER_BUNDLE,),
        )
        # The original BrowserOS server bundle moved to browseros-human.
        self.assertEqual(server_bundles_for_product("browseros"), ())

    def test_server_ota_bundles_use_the_active_rust_claw_bundle(self):
        self.assertEqual(
            server_ota_bundles_for_product("browserclaw"),
            (BROWSERCLAW_SERVER_BUNDLE,),
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

    def test_macos_sign_spec_for_rejects_unregistered_stems(self):
        self.assertIsNone(macos_sign_spec_for(Path("/x/not_a_known_binary")))

    def test_macos_sign_spec_for_resolves_claw_server_binary(self):
        spec = macos_sign_spec_for(Path("/x/browseros-claw-server"))
        assert spec is not None
        self.assertEqual(spec.identifier_suffix, "browseros_claw_server")
        self.assertEqual(spec.options, "runtime")
        self.assertEqual(
            spec.entitlements, "browseros-executable-entitlements.plist"
        )

    def test_original_server_binaries_are_not_registered(self):
        # The original BrowserOS server bundle (browseros_server, bun, rg)
        # moved to browseros-human; neo signs only its own server binary.
        for binary in ("browseros_server", "bun", "rg", "codex", "claude"):
            self.assertIsNone(macos_sign_spec_for(Path(f"/x/{binary}")), binary)

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
                rel == "browseros-claw-server.exe" or rel.startswith("third_party/"),
                f"{rel} outside expected layout",
            )

    def test_expected_windows_binary_paths_resolves_browserclaw_descriptor(self):
        root = Path("/tmp/fake/resources/bin")
        self.assertEqual(
            expected_windows_binary_paths(root, BROWSERCLAW_SERVER_BUNDLE),
            [root / "browseros-claw-server.exe"],
        )

    def test_expected_windows_bundle_binary_paths_includes_claw(self):
        build_output_dir = Path("/tmp/out/Default")

        self.assertEqual(
            expected_windows_bundle_binary_paths(build_output_dir),
            [
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
