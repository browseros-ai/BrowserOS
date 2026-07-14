#!/usr/bin/env python3
"""Tests for product user-data directory patches."""

import re
import unittest

from ...lib.paths import get_package_root


PATCHES = get_package_root() / "chromium_patches"


def _patch(relative_path: str) -> str:
    return (PATCHES / relative_path).read_text()


def _patched_source(relative_path: str) -> str:
    """Reconstruct the changed source regions from a unified diff."""
    source_lines: list[str] = []
    in_hunk = False

    for line in _patch(relative_path).splitlines():
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


def _product_identity_branches(source: str) -> tuple[str, str]:
    """Return the BrowserClaw and BrowserOS compile-time identity branches."""
    match = re.search(
        r"#if BUILDFLAG\(BROWSEROS_PRODUCT_BROWSERCLAW\)\n"
        r"(?P<browserclaw>.*?)\n#else\n(?P<browseros>.*?)\n#endif",
        source,
        re.DOTALL,
    )
    if match is None:
        raise AssertionError("missing BrowserClaw install identity buildflag branch")
    return match.group("browserclaw"), match.group("browseros")


def _field_initializer(source: str, field: str) -> str:
    """Return a field's string literal or GUID aggregate initializer."""
    match = re.search(
        rf"\.{re.escape(field)}\s*=\s*"
        r'(?P<value>L?"[^"]*"|\{.*?\}\s*\}),',
        source,
        re.DOTALL,
    )
    if match is None:
        raise AssertionError(f"missing initializer for {field}")
    value = match.group("value")
    return value if value.startswith(('L"', '"')) else re.sub(r"\s+", "", value)


def _guid(initializer: str) -> str:
    """Return a GUID initializer in canonical text form."""
    literal = re.search(r"\{([0-9A-Fa-f-]{36})\}", initializer)
    if literal is not None:
        return literal.group(1).upper()

    values = [int(token, 16) for token in re.findall(r"0x[0-9A-Fa-f]+", initializer)]
    if len(values) != 11:
        raise AssertionError(f"invalid GUID initializer: {initializer}")
    return (
        f"{values[0]:08X}-{values[1]:04X}-{values[2]:04X}-"
        f"{values[3]:02X}{values[4]:02X}-"
        + "".join(f"{value:02X}" for value in values[5:])
    )


class ProductUserDataDirPatchTest(unittest.TestCase):
    def test_mac_profile_root_comes_from_browseros_product_gn_arg(self) -> None:
        build = _patch("chrome/BUILD.gn")
        plist = _patch("chrome/app/app-Info.plist")
        buildflags = _patch("chrome/browser/browseros/buildflags.gni")

        self.assertIn(
            '"BROWSEROS_PRODUCT_DIR_NAME=$browseros_product_dir_name"', build
        )
        self.assertRegex(
            plist,
            re.compile(
                r"\+\t<key>CrProductDirName</key>\n"
                r"\+\t<string>\$\{BROWSEROS_PRODUCT_DIR_NAME\}</string>"
            ),
        )
        self.assertRegex(
            buildflags,
            re.compile(
                r'\+if \(browseros_product_browserclaw\) \{\n'
                r'\+  browseros_product_dir_name = "BrowserClaw"\n'
                r'\+\} else \{\n'
                r'\+  browseros_product_dir_name = "BrowserOS"\n'
                r'\+\}'
            ),
        )

    def test_linux_profile_roots_are_product_specific(self) -> None:
        linux_paths = _patch("chrome/common/chrome_paths_linux.cc")

        self.assertRegex(
            linux_paths,
            re.compile(
                r"\+#elif BUILDFLAG\(BROWSEROS_PRODUCT_BROWSERCLAW\)\n"
                r'\+  std::string data_dir_basename = "browser-claw";\n'
                r" #else\n"
                r'-  std::string data_dir_basename = "chromium";\n'
                r'\+  std::string data_dir_basename = "browser-os";'
            ),
        )

    def test_windows_profile_roots_are_product_specific(self) -> None:
        install_modes = _patched_source(
            "chrome/install_static/chromium_install_modes.h"
        )
        browserclaw, browseros = _product_identity_branches(install_modes)

        self.assertEqual(
            install_modes.count("#if BUILDFLAG(BROWSEROS_PRODUCT_BROWSERCLAW)"),
            1,
        )
        self.assertIn(
            'inline constexpr wchar_t kProductPathName[] = L"BrowserClaw";',
            browserclaw,
        )
        self.assertIn(
            'inline constexpr wchar_t kProductPathName[] = L"BrowserOS";', browseros
        )
        self.assertNotIn("GetProduct", install_modes)
        self.assertNotIn("browseros_product.h", install_modes)

    def test_windows_install_identity_branches_are_complete(self) -> None:
        install_modes = _patched_source(
            "chrome/install_static/chromium_install_modes.h"
        )
        browserclaw, browseros = _product_identity_branches(install_modes)
        identity_struct = re.search(
            r"struct ProductInstallIdentity \{(?P<body>.*?)\n\};",
            install_modes,
            re.DOTALL,
        )

        self.assertIsNotNone(identity_struct)
        assert identity_struct is not None
        self.assertNotIn("app_guid", identity_struct.group("body"))
        self.assertNotIn("elevator_iid", identity_struct.group("body"))
        self.assertNotIn("tracing_service_iid", identity_struct.group("body"))
        self.assertEqual(
            browserclaw.count(
                "inline constexpr ProductInstallIdentity kProductInstallIdentity"
            ),
            1,
        )
        self.assertEqual(
            browseros.count(
                "inline constexpr ProductInstallIdentity kProductInstallIdentity"
            ),
            1,
        )

        expected_fields = {
            "base_app_name": ('L"BrowserClaw"', 'L"BrowserOS"'),
            "base_app_id": ('L"BrowserClaw"', 'L"BrowserOS"'),
            "browser_prog_id_prefix": ('L"BClawHTML"', 'L"BOSHTML"'),
            "browser_prog_id_description": (
                'L"BrowserClaw HTML Document"',
                'L"BrowserOS HTML Document"',
            ),
            "direct_launch_url_scheme": ('"browserclaw"', '"browseros"'),
            "pdf_prog_id_prefix": ('L"BClawPDF"', 'L"BOSPDF"'),
            "pdf_prog_id_description": (
                'L"BrowserClaw PDF Document"',
                'L"BrowserOS PDF Document"',
            ),
            "active_setup_guid": (
                'L"{E9E65674-914E-4A29-83A9-A98D407446EC}"',
                'L"{0EF5669B-7FD7-4138-A91F-E466631ADE97}"',
            ),
            "legacy_command_execute_clsid": (
                'L""',
                'L"{AFDDB293-0724-49E5-A4EC-1096BF6C84AF}"',
            ),
        }
        for field, (browserclaw_value, browseros_value) in expected_fields.items():
            with self.subTest(field=field, product="browserclaw"):
                self.assertEqual(
                    _field_initializer(browserclaw, field), browserclaw_value
                )
            with self.subTest(field=field, product="browseros"):
                self.assertEqual(_field_initializer(browseros, field), browseros_value)

        self.assertIn('L"924012147-"', browserclaw)
        self.assertIn('L"924012148-"', browseros)

        for field in (
            "base_app_name",
            "base_app_id",
            "browser_prog_id_prefix",
            "browser_prog_id_description",
            "direct_launch_url_scheme",
            "pdf_prog_id_prefix",
            "pdf_prog_id_description",
            "active_setup_guid",
            "legacy_command_execute_clsid",
            "toast_activator_clsid",
            "elevator_clsid",
            "tracing_service_clsid",
            "sandbox_sid_prefix",
        ):
            self.assertIn(f"kProductInstallIdentity.{field}", install_modes)

    def test_windows_install_clsids_are_product_specific(self) -> None:
        install_modes = _patched_source(
            "chrome/install_static/chromium_install_modes.h"
        )
        browserclaw, browseros = _product_identity_branches(install_modes)
        guid_fields = (
            "active_setup_guid",
            "toast_activator_clsid",
            "elevator_clsid",
            "tracing_service_clsid",
        )
        browserclaw_guids = {
            _guid(_field_initializer(browserclaw, field)) for field in guid_fields
        }
        browseros_guids = {
            _guid(_field_initializer(browseros, field)) for field in guid_fields
        }

        self.assertTrue(browserclaw_guids.isdisjoint(browseros_guids))
        expected_clsids = {
            "toast_activator_clsid": (
                "D0A19C03-EE25-463B-B38F-08516D2B1A79",
                "E76CCE76-27A7-46D3-9EED-CC8C5ED7BE72",
            ),
            "elevator_clsid": (
                "0AC4EA74-A61A-4807-AFE5-03701D2B97DD",
                "29ED629C-1F0E-47D1-A684-9397ACDB71AB",
            ),
            "tracing_service_clsid": (
                "9F3CA910-142B-4C2C-A61E-B2335E2E67FD",
                "C39C8575-9F42-4599-96F1-19DB7AEB51AF",
            ),
        }
        for field, (
            browserclaw_clsid,
            browseros_clsid,
        ) in expected_clsids.items():
            with self.subTest(field=field, product="browserclaw"):
                self.assertEqual(
                    _guid(_field_initializer(browserclaw, field)),
                    browserclaw_clsid,
                )
            with self.subTest(field=field, product="browseros"):
                self.assertEqual(
                    _guid(_field_initializer(browseros, field)), browseros_clsid
                )

    def test_windows_install_shared_identity_matches_implemented_interfaces(
        self,
    ) -> None:
        install_modes = _patched_source(
            "chrome/install_static/chromium_install_modes.h"
        )
        browserclaw, browseros = _product_identity_branches(install_modes)

        self.assertEqual(install_modes.count('.app_guid = L"",'), 1)
        self.assertNotRegex(install_modes, r'\.app_guid\s*=\s*L"\{')
        self.assertNotIn("elevator_iid", browserclaw)
        self.assertNotIn("elevator_iid", browseros)
        self.assertNotIn("tracing_service_iid", browserclaw)
        self.assertNotIn("tracing_service_iid", browseros)

        expected_iids = {
            "elevator_iid": "BB19A0E5-00C6-4966-94B2-5AFEC6FED93A",
            "tracing_service_iid": "A3FD580A-FFD4-4075-9174-75D0B199D3CB",
        }
        for field, expected_iid in expected_iids.items():
            with self.subTest(field=field):
                self.assertEqual(install_modes.count(f".{field} ="), 1)
                self.assertEqual(
                    _guid(_field_initializer(install_modes, field)), expected_iid
                )


if __name__ == "__main__":
    unittest.main()
