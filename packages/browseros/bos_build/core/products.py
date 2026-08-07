#!/usr/bin/env python3
"""Product descriptor type and the define() authoring factory.

The runtime type stays a fully explicit frozen dataclass (typed,
greppable). Authoring goes through ProductDescriptor.define(): ~8
irreducible inputs, everything else derived by convention, keyword
overrides for genuine deviations. Product instances live in
bos_build/products/<id>/product.py — one file answers "what is X".
"""

from dataclasses import dataclass, fields
from typing import Any, Dict, Optional, Tuple

BROWSEROS_AGENT_EXTENSION_ID = "bflpfmnmnokmjhmgnolecpppdbdophmk"
BROWSEROS_BUG_REPORTER_EXTENSION_ID = "adlpneommgkgeanpaekgoaolcpncohkf"
BROWSERCLAW_EXTENSION_ID = "pjimfkbpehlcllblajnpfamdfjhhlgkc"
# Packaged to the CDN but neither bundled nor in the update feeds.
BROWSEROS_CONTROLLER_EXTENSION_ID = "nlnihljpboknmfagkikhkdblbedophja"


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

    def artifact_filename(self, artifact_type: str, version: str, arch: str) -> str:
        """Standardized artifact filename, e.g. "BrowserOS_v0.31.0_arm64.dmg"."""
        base = self.artifact_prefix
        match artifact_type:
            case "dmg":
                return f"{base}_v{version}_{arch}.dmg"
            case "appimage":
                return f"{base}_v{version}_{arch}.AppImage"
            case "deb":
                deb_arch = {"x64": "amd64", "arm64": "arm64"}.get(arch, arch)
                return f"{base}_v{version}_{deb_arch}.deb"
            case "installer":
                return f"{base}_v{version}_{arch}_installer.exe"
            case "installer_zip":
                return f"{base}_v{version}_{arch}_installer.zip"
            case _:
                raise ValueError(f"Unknown artifact type: {artifact_type}")

    @classmethod
    def define(
        cls,
        *,
        id: str,
        display_name: str,
        windows_installer_guid: str,
        summary: str,
        description: str,
        mac_bundle_domain: str = "com.browseros",
        company: str = "BrowserOS",
        homepage_url: str = "https://www.browseros.com/",
        support_url: str = "https://docs.browseros.com/",
        bugtracker_url: str = "https://github.com/browseros-ai/BrowserOS/issues",
        required_extensions: Tuple[Tuple[str, str], ...] = (),
        **overrides,
    ) -> "ProductDescriptor":
        """Build a descriptor from irreducible inputs; derive the rest.

        Any top-level descriptor field can be overridden by keyword —
        a deviation from convention is then visible at the definition
        site. Unknown override names raise so typos fail loudly.
        """
        base = display_name
        derived: Dict[str, Any] = dict(
            id=id,
            gn_product=id,
            display_name=display_name,
            dev_display_name=f"{display_name} Dev",
            company_full_name=company,
            company_short_name=company,
            installer_full_name=f"{display_name} Installer",
            dev_installer_full_name=f"{display_name} Dev Installer",
            app_base_name=base,
            artifact_prefix=base,
            release_prefix=id,
            homepage_url=homepage_url,
            support_url=support_url,
            bugtracker_url=bugtracker_url,
            summary=summary,
            description=description,
            string_replacements=_replacements(display_name),
            required_extension_ids=required_extensions,
            server_bundle_ids=(f"{id}-server",),
            mac=MacProductIdentity(
                bundle_id=f"{mac_bundle_domain}.{base}",
                dev_bundle_id=f"{mac_bundle_domain}.dev.{base}",
                signing_identifier=f"{mac_bundle_domain}.{base}",
                dev_signing_identifier=f"{mac_bundle_domain}.dev.{base}",
                framework_name=f"{display_name} Framework.framework",
                dev_framework_name=f"{display_name} Dev Framework.framework",
                dmg_volume_name=display_name,
            ),
            linux=LinuxProductIdentity(
                package_name=id,
                launcher_name=id,
                desktop_id=f"{id}.desktop",
                icon_name=id,
                lib_dir=f"/usr/lib/{id}",
                appimage_dir=f"/opt/{id}",
                apparmor_profile_name=id,
                metainfo_id=f"{id}.desktop",
            ),
            windows=WindowsProductIdentity(
                app_user_model_id=f"{company}.{base}",
                installer_app_id=windows_installer_guid,
            ),
        )

        valid_fields = {f.name for f in fields(cls)}
        unknown = set(overrides) - valid_fields
        if unknown:
            raise TypeError(
                f"Unknown ProductDescriptor override(s) for '{id}': "
                f"{', '.join(sorted(unknown))}"
            )
        derived.update(overrides)
        return cls(**derived)


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


def get_product_descriptor(product_id: Optional[str]) -> ProductDescriptor:
    """Resolve a product id to a registered product descriptor.

    Deferred import: products/<id>/product.py files import this module
    for the descriptor type, so the registry loads lazily.
    """
    from ..products import DEFAULT_PRODUCT_ID, PRODUCTS

    resolved_id = product_id or DEFAULT_PRODUCT_ID
    try:
        return PRODUCTS[resolved_id]
    except KeyError as exc:
        valid = ", ".join(sorted(PRODUCTS))
        raise ValueError(
            f"Unknown build.product '{resolved_id}'. Valid: {valid}"
        ) from exc


def default_product_descriptor() -> ProductDescriptor:
    """Return the default product used outside config mode."""
    return get_product_descriptor(None)
