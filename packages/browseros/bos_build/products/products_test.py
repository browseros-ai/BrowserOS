#!/usr/bin/env python3
"""Golden tests for shipped product descriptors and define() conventions."""

import unittest

from bos_build.core.products import (
    LinuxProductIdentity,
    MacProductIdentity,
    ProductDescriptor,
    WindowsProductIdentity,
    _replacements,
    get_product_descriptor,
)
from bos_build.products import DEFAULT_PRODUCT_ID, PRODUCTS

BROWSEROS_BUG_REPORTER_EXTENSION_ID = "adlpneommgkgeanpaekgoaolcpncohkf"
BROWSERCLAW_EXTENSION_ID = "pjimfkbpehlcllblajnpfamdfjhhlgkc"

EXPECTED_BROWSERCLAW = ProductDescriptor(
    id="browserclaw",
    gn_product="browserclaw",
    display_name="BrowserOS neo",
    dev_display_name="BrowserOS neo Dev",
    company_full_name="BrowserOS",
    company_short_name="BrowserOS",
    installer_full_name="BrowserOS neo Installer",
    dev_installer_full_name="BrowserOS neo Dev Installer",
    app_base_name="BrowserOS neo",
    artifact_prefix="BrowserOS_neo",
    release_prefix="browserclaw",
    homepage_url="https://www.browseros.com/",
    support_url="https://docs.browseros.com/",
    bugtracker_url="https://github.com/browseros-ai/BrowserOS/issues",
    summary="The open source browser for web agents",
    description="BrowserOS neo is a Chromium-based browser for agent workflows.",
    string_replacements=_replacements("BrowserOS neo"),
    required_extension_ids=(
        (BROWSERCLAW_EXTENSION_ID, "BrowserOS neo app"),
        (BROWSEROS_BUG_REPORTER_EXTENSION_ID, "BrowserOS bug reporter"),
    ),
    server_bundle_ids=("browserclaw-server",),
    mac=MacProductIdentity(
        bundle_id="com.browseros.BrowserClaw",
        dev_bundle_id="com.browseros.dev.BrowserClaw",
        signing_identifier="com.browseros.BrowserClaw",
        dev_signing_identifier="com.browseros.dev.BrowserClaw",
        framework_name="BrowserOS neo Framework.framework",
        dev_framework_name="BrowserOS neo Dev Framework.framework",
        dmg_volume_name="BrowserOS neo",
    ),
    linux=LinuxProductIdentity(
        package_name="browserclaw",
        launcher_name="browserclaw",
        desktop_id="browserclaw.desktop",
        icon_name="browserclaw",
        lib_dir="/usr/lib/browserclaw",
        appimage_dir="/opt/browserclaw",
        apparmor_profile_name="browserclaw",
        metainfo_id="browserclaw.desktop",
    ),
    windows=WindowsProductIdentity(
        app_user_model_id="BrowserOS.BrowserClaw",
        installer_app_id="{FA2AFFF8-647B-477C-A5D2-905BA8DB9B82}",
    ),
)


class DefineGoldenTest(unittest.TestCase):
    def test_browserclaw_matches_expected_descriptor(self):
        self.assertEqual(get_product_descriptor("browserclaw"), EXPECTED_BROWSERCLAW)


class DefineBehaviorTest(unittest.TestCase):
    def _minimal(self, **overrides):
        return ProductDescriptor.define(
            id="acmefox",
            display_name="AcmeFox",
            windows_installer_guid="{00000000-0000-0000-0000-000000000000}",
            summary="s",
            description="d",
            **overrides,
        )

    def test_derivations_for_new_product(self):
        p = self._minimal()
        self.assertEqual(p.dev_display_name, "AcmeFox Dev")
        self.assertEqual(p.mac.bundle_id, "com.browseros.AcmeFox")
        self.assertEqual(p.mac.dev_bundle_id, "com.browseros.dev.AcmeFox")
        self.assertEqual(p.mac.framework_name, "AcmeFox Framework.framework")
        self.assertEqual(p.linux.lib_dir, "/usr/lib/acmefox")
        self.assertEqual(p.windows.app_user_model_id, "BrowserOS.AcmeFox")
        self.assertEqual(p.server_bundle_ids, ("acmefox-server",))
        self.assertEqual(p.release_prefix, "acmefox")
        self.assertEqual(p.required_extension_ids, ())

    def test_override_wins_over_derivation(self):
        p = self._minimal(artifact_prefix="Acme")
        self.assertEqual(p.artifact_prefix, "Acme")
        self.assertEqual(p.app_base_name, "AcmeFox")

    def test_unknown_override_raises(self):
        with self.assertRaisesRegex(TypeError, "Unknown ProductDescriptor override"):
            self._minimal(dmg_name="X")


class RegistryTest(unittest.TestCase):
    def test_registry_has_only_neo_as_default(self):
        self.assertEqual(set(PRODUCTS), {"browserclaw"})
        self.assertEqual(DEFAULT_PRODUCT_ID, "browserclaw")
        self.assertEqual(get_product_descriptor(None), EXPECTED_BROWSERCLAW)

    def test_unknown_product_raises(self):
        # The original BrowserOS product moved to browseros-human.
        for product_id in ("netscape", "browseros"):
            with self.subTest(product_id=product_id):
                with self.assertRaisesRegex(
                    ValueError, f"Unknown build.product '{product_id}'"
                ):
                    get_product_descriptor(product_id)


if __name__ == "__main__":
    unittest.main()
