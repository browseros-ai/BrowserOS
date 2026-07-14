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

BROWSEROS_AGENT_EXTENSION_ID = "bflpfmnmnokmjhmgnolecpppdbdophmk"
BROWSEROS_BUG_REPORTER_EXTENSION_ID = "adlpneommgkgeanpaekgoaolcpncohkf"
BROWSERCLAW_EXTENSION_ID = "pjimfkbpehlcllblajnpfamdfjhhlgkc"

EXPECTED_BROWSEROS = ProductDescriptor(
    id="browseros",
    gn_product="browseros",
    display_name="BrowserOS",
    dev_display_name="BrowserOS Dev",
    company_full_name="BrowserOS",
    company_short_name="BrowserOS",
    installer_full_name="BrowserOS Installer",
    dev_installer_full_name="BrowserOS Dev Installer",
    app_base_name="BrowserOS",
    artifact_prefix="BrowserOS",
    release_prefix="browseros",
    homepage_url="https://www.browseros.com/",
    support_url="https://docs.browseros.com/",
    bugtracker_url="https://github.com/browseros-ai/BrowserOS/issues",
    summary="The open source agentic browser",
    description="BrowserOS is a privacy-focused web browser built on Chromium.",
    string_replacements=_replacements("BrowserOS"),
    required_extension_ids=(
        (BROWSEROS_AGENT_EXTENSION_ID, "BrowserOS agent"),
        (BROWSEROS_BUG_REPORTER_EXTENSION_ID, "BrowserOS bug reporter"),
    ),
    server_bundle_ids=("browseros-server",),
    mac=MacProductIdentity(
        bundle_id="com.browseros.BrowserOS",
        dev_bundle_id="com.browseros.dev.BrowserOS",
        signing_identifier="com.browseros.BrowserOS",
        dev_signing_identifier="com.browseros.dev.BrowserOS",
        framework_name="BrowserOS Framework.framework",
        dev_framework_name="BrowserOS Dev Framework.framework",
        dmg_volume_name="BrowserOS",
    ),
    linux=LinuxProductIdentity(
        package_name="browseros",
        launcher_name="browseros",
        desktop_id="browseros.desktop",
        icon_name="browseros",
        lib_dir="/usr/lib/browseros",
        appimage_dir="/opt/browseros",
        apparmor_profile_name="browseros",
        metainfo_id="browseros.desktop",
    ),
    windows=WindowsProductIdentity(
        app_user_model_id="BrowserOS.BrowserOS",
        installer_app_id="{5d8d08af-2df9-4da2-86c1-eac353a0ca32}",
    ),
)

EXPECTED_BROWSERCLAW = ProductDescriptor(
    id="browserclaw",
    gn_product="browserclaw",
    display_name="BrowserClaw",
    dev_display_name="BrowserClaw Dev",
    company_full_name="BrowserOS",
    company_short_name="BrowserOS",
    installer_full_name="BrowserClaw Installer",
    dev_installer_full_name="BrowserClaw Dev Installer",
    app_base_name="BrowserClaw",
    artifact_prefix="BrowserClaw",
    release_prefix="browserclaw",
    homepage_url="https://www.browseros.com/",
    support_url="https://docs.browseros.com/",
    bugtracker_url="https://github.com/browseros-ai/BrowserOS/issues",
    summary="The open source browser for web agents",
    description="BrowserClaw is a Chromium-based browser for agent workflows.",
    string_replacements=_replacements("BrowserClaw"),
    required_extension_ids=(
        (BROWSERCLAW_EXTENSION_ID, "BrowserClaw app"),
        (BROWSEROS_BUG_REPORTER_EXTENSION_ID, "BrowserOS bug reporter"),
    ),
    server_bundle_ids=("browserclaw-server",),
    mac=MacProductIdentity(
        bundle_id="com.browseros.BrowserClaw",
        dev_bundle_id="com.browseros.dev.BrowserClaw",
        signing_identifier="com.browseros.BrowserClaw",
        dev_signing_identifier="com.browseros.dev.BrowserClaw",
        framework_name="BrowserClaw Framework.framework",
        dev_framework_name="BrowserClaw Dev Framework.framework",
        dmg_volume_name="BrowserClaw",
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
    def test_browseros_matches_expected_descriptor(self):
        self.assertEqual(get_product_descriptor("browseros"), EXPECTED_BROWSEROS)

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
    def test_registry_has_both_products_and_default(self):
        self.assertEqual(set(PRODUCTS), {"browseros", "browserclaw"})
        self.assertEqual(DEFAULT_PRODUCT_ID, "browseros")

    def test_unknown_product_raises(self):
        with self.assertRaisesRegex(ValueError, "Unknown build.product"):
            get_product_descriptor("netscape")


if __name__ == "__main__":
    unittest.main()
