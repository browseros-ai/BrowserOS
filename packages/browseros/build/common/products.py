#!/usr/bin/env python3
"""Product descriptors for BrowserOS Chromium builds."""

from dataclasses import dataclass


BROWSEROS_AGENT_EXTENSION_ID = "bflpfmnmnokmjhmgnolecpppdbdophmk"
BROWSEROS_BUG_REPORTER_EXTENSION_ID = "adlpneommgkgeanpaekgoaolcpncohkf"
BROWSERCLAW_EXTENSION_ID = "pjimfkbpehlcllblajnpfamdfjhhlgkc"


@dataclass(frozen=True)
class MacProductIdentity:
    bundle_id: str
    dev_bundle_id: str
    signing_identifier: str
    dev_signing_identifier: str
    framework_name: str
    dev_framework_name: str
    dmg_volume_name: str


@dataclass(frozen=True)
class LinuxProductIdentity:
    package_name: str
    launcher_name: str
    desktop_id: str
    icon_name: str
    lib_dir: str
    appimage_dir: str
    apparmor_profile_name: str
    metainfo_id: str


@dataclass(frozen=True)
class WindowsProductIdentity:
    app_user_model_id: str
    installer_app_id: str


@dataclass(frozen=True)
class ProductDescriptor:
    id: str
    gn_product: str
    display_name: str
    dev_display_name: str
    company_full_name: str
    company_short_name: str
    installer_full_name: str
    dev_installer_full_name: str
    app_base_name: str
    artifact_prefix: str
    release_prefix: str
    homepage_url: str
    support_url: str
    bugtracker_url: str
    summary: str
    description: str
    string_replacements: tuple[tuple[str, str], ...]
    required_extension_ids: tuple[tuple[str, str], ...]
    server_bundle_ids: tuple[str, ...]
    mac: MacProductIdentity
    linux: LinuxProductIdentity
    windows: WindowsProductIdentity

    def app_name(self, build_type: str) -> str:
        """Return the display app name for a release or debug build."""
        return self.dev_display_name if build_type == "debug" else self.display_name

    def installer_name(self, build_type: str) -> str:
        """Return the installer display name for a release or debug build."""
        return (
            self.dev_installer_full_name
            if build_type == "debug"
            else self.installer_full_name
        )

    def mac_bundle_id(self, build_type: str) -> str:
        """Return the macOS bundle identifier for a release or debug build."""
        return self.mac.dev_bundle_id if build_type == "debug" else self.mac.bundle_id

    def mac_signing_identifier(self, build_type: str) -> str:
        """Return the codesign base identifier for a release or debug build."""
        if build_type == "debug":
            return self.mac.dev_signing_identifier
        return self.mac.signing_identifier

    def mac_framework_name(self, build_type: str) -> str:
        """Return the main Chromium framework name for this product."""
        if build_type == "debug":
            return self.mac.dev_framework_name
        return self.mac.framework_name


def _replacements(product_name: str) -> tuple[tuple[str, str], ...]:
    return (
        (
            r"The Chromium Authors. All rights reserved.",
            f"The {product_name} Authors. All rights reserved.",
        ),
        (
            r"Google LLC. All rights reserved.",
            f"The {product_name} Authors. All rights reserved.",
        ),
        (r"The Chromium Authors", f"{product_name} Software Inc"),
        (r"Google Chrome", product_name),
        (r"(Google)(?! Play)", product_name),
        (r"Chromium", product_name),
        (r"Chrome", product_name),
    )


BROWSEROS_PRODUCT = ProductDescriptor(
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
        (BROWSEROS_BUG_REPORTER_EXTENSION_ID, "BrowserOS bug reporter"),
        (BROWSEROS_AGENT_EXTENSION_ID, "BrowserOS agent"),
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


BROWSERCLAW_PRODUCT = ProductDescriptor(
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
    required_extension_ids=((BROWSERCLAW_EXTENSION_ID, "BrowserClaw app"),),
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


PRODUCTS = {
    BROWSEROS_PRODUCT.id: BROWSEROS_PRODUCT,
    BROWSERCLAW_PRODUCT.id: BROWSERCLAW_PRODUCT,
}


def get_product_descriptor(product_id: str | None) -> ProductDescriptor:
    """Resolve a product id to a committed product descriptor."""
    resolved_id = product_id or BROWSEROS_PRODUCT.id
    try:
        return PRODUCTS[resolved_id]
    except KeyError as exc:
        valid = ", ".join(sorted(PRODUCTS))
        raise ValueError(f"Unknown build.product '{resolved_id}'. Valid: {valid}") from exc


def default_product_descriptor() -> ProductDescriptor:
    """Return the default product used outside config mode."""
    return BROWSEROS_PRODUCT
