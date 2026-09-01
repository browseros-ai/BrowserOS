"""Strict renderer for reviewable Linux packaging policy assets.

The renderer is intentionally not a general template language. Every asset has
an exact placeholder set and mode, while callers must validate or escape values
for the destination syntax before substitution.
"""

from __future__ import annotations

import re
import shutil
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING
from xml.sax.saxutils import escape

if TYPE_CHECKING:
    from ....core.context import Context


class PolicyAssetError(RuntimeError):
    """A committed policy asset or substitution value is unsafe or incomplete."""


@dataclass(frozen=True)
class AssetSpec:
    """Committed policy text plus its allowed substitutions and installed mode."""

    filename: str
    placeholders: frozenset[str]
    mode: int = 0o644


_PLACEHOLDER = re.compile(r"@@([A-Z][A-Z0-9_]*)@@")
_SAFE_LINUX_IDENTIFIER = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._+-]*$")
_SAFE_LINUX_PATH = re.compile(r"^/[A-Za-z0-9][A-Za-z0-9._+/-]*$")
_ASSET_ROOT = Path(__file__).with_name("assets")
_ICON_SIZES = (16, 22, 24, 32, 48, 64, 128, 256)

ASSETS = {
    "desktop": AssetSpec(
        "desktop.in",
        frozenset({"DISPLAY_NAME", "EXEC", "ICON_NAME"}),
    ),
    "apprun": AssetSpec(
        "AppRun.in",
        frozenset({"APP_DIR", "BROWSER_NAME"}),
        0o755,
    ),
    "control": AssetSpec(
        "debian/control.in",
        frozenset(
            {
                "PACKAGE_NAME",
                "VERSION",
                "DEB_ARCH",
                "HOMEPAGE_URL",
                "DISPLAY_NAME",
                "SUMMARY",
                "DESCRIPTION",
            }
        ),
    ),
    "launcher": AssetSpec(
        "debian/launcher.in",
        frozenset({"LIB_DIR", "BROWSER_NAME"}),
        0o755,
    ),
    "postinst": AssetSpec(
        "debian/postinst.in",
        frozenset({"LIB_DIR", "APPARMOR_PROFILE", "LAUNCHER_NAME"}),
        0o755,
    ),
    "prerm": AssetSpec(
        "debian/prerm.in",
        frozenset({"APPARMOR_PROFILE", "LAUNCHER_NAME"}),
        0o755,
    ),
    "apparmor": AssetSpec(
        "debian/apparmor.in",
        frozenset({"DISPLAY_NAME", "APPARMOR_PROFILE", "LIB_DIR", "BROWSER_NAME"}),
    ),
    "metainfo": AssetSpec(
        "debian/metainfo.xml.in",
        frozenset(
            {
                "METAINFO_ID",
                "DESKTOP_ID",
                "DISPLAY_NAME",
                "SUMMARY",
                "HOMEPAGE_URL",
                "BUGTRACKER_URL",
                "SUPPORT_URL",
                "DESCRIPTION",
                "VERSION",
            }
        ),
    ),
}


def write_asset(destination: Path, name: str, values: dict[str, str]) -> Path:
    """Render one named policy asset and install its declared mode."""
    spec = ASSETS[name]
    source_path = _ASSET_ROOT / spec.filename
    source = source_path.read_text(encoding="utf-8")
    found = frozenset(_PLACEHOLDER.findall(source))
    if found != spec.placeholders:
        raise PolicyAssetError(
            f"Policy asset {spec.filename} placeholders changed: "
            f"expected {sorted(spec.placeholders)}, found {sorted(found)}"
        )
    if frozenset(values) != spec.placeholders:
        missing = sorted(spec.placeholders - values.keys())
        unknown = sorted(values.keys() - spec.placeholders)
        raise PolicyAssetError(
            f"Policy asset {spec.filename} substitutions are invalid: "
            f"missing={missing or '-'}, unknown={unknown or '-'}"
        )

    rendered = _PLACEHOLDER.sub(lambda match: values[match.group(1)], source)
    if _PLACEHOLDER.search(rendered):
        raise PolicyAssetError(
            f"Policy asset {spec.filename} contains an unresolved placeholder"
        )
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(rendered, encoding="utf-8")
    destination.chmod(spec.mode)
    return destination


def single_line(value: object, field: str) -> str:
    """Validate a value embedded in a one-line desktop or Debian field."""
    rendered = str(value)
    if any(
        ord(character) < 0x20
        or 0x7F <= ord(character) <= 0x9F
        or character in ("\u2028", "\u2029")
        for character in rendered
    ):
        raise PolicyAssetError(f"{field} must be one line without control characters")
    return rendered


def linux_identifier(value: object, field: str) -> str:
    """Validate an unquoted package, desktop, executable, or profile name."""
    rendered = single_line(value, field)
    if not _SAFE_LINUX_IDENTIFIER.fullmatch(rendered):
        raise PolicyAssetError(f"{field} is not a safe Linux identifier: {rendered}")
    return rendered


def absolute_linux_path(value: object, field: str) -> str:
    """Validate an absolute package path embedded unquoted in shell policy."""
    rendered = single_line(value, field)
    if (
        not _SAFE_LINUX_PATH.fullmatch(rendered)
        or ".." in Path(rendered).parts
        or "//" in rendered
    ):
        raise PolicyAssetError(f"{field} is not a safe absolute Linux path: {rendered}")
    return rendered


def xml_text(value: object, field: str) -> str:
    """Escape one product-owned string for XML element or attribute text."""
    return escape(single_line(value, field), {'"': "&quot;", "'": "&apos;"})


def product_icon_source(ctx: Context) -> Path:
    """Return the required root icon used by both package formats."""
    source_root = Path(ctx.root_dir) / "resources" / ctx.product.id / "icons"
    for filename in ("product_logo_256.png", "product_logo.png"):
        candidate = source_root / filename
        if candidate.is_file():
            return candidate
    raise PolicyAssetError(
        "Required product icon is missing; expected product_logo_256.png or "
        f"product_logo.png under {source_root}"
    )


def copy_hicolor_icons(ctx: Context, destination_root: Path) -> None:
    """Install all committed product icon sizes with one deterministic fallback."""
    source_root = Path(ctx.root_dir) / "resources" / ctx.product.id / "icons"
    icon_name = linux_identifier(ctx.product.linux.icon_name, "icon name")
    copied = False
    for size in _ICON_SIZES:
        source = source_root / f"product_logo_{size}.png"
        if not source.is_file():
            continue
        destination = destination_root / f"{size}x{size}" / "apps" / f"{icon_name}.png"
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)
        copied = True
    if not copied:
        destination = destination_root / "256x256/apps" / f"{icon_name}.png"
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(product_icon_source(ctx), destination)
