#!/usr/bin/env python3
"""Application signing and notarization module for BrowserOS (macOS)"""

import fnmatch
import os
import plistlib
import re
import shutil
import subprocess
import tempfile
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, Iterator, List, Optional, Tuple

from ...core.context import Context
from ...core.step import Step, ValidationError, step
from ...lib.env import EnvConfig
from ...lib.notarization import notarytool_wait_args
from ...products.server_binaries import (
    ServerBundle,
    all_server_bundles,
    macos_sign_spec_for,
    server_bundles_for_product,
)
from ...lib.utils import (
    IS_MACOS,
    log_error,
    log_info,
    log_success,
    log_warning,
    join_paths,
    run_command as utils_run_command,
)


def get_browseros_server_binary_info(component_path: Path) -> Optional[Dict[str, str]]:
    """Return metadata for known BrowserOS Server binaries, if applicable."""
    spec = macos_sign_spec_for(component_path)
    if spec is None:
        return None
    info: Dict[str, str] = {
        "identifier_suffix": spec.identifier_suffix,
        "options": spec.options,
    }
    if spec.entitlements:
        info["entitlements"] = spec.entitlements
    return info


SERVER_RESOURCES_SOURCE_REL = all_server_bundles()[0].chromium_resources_root
SERVER_RESOURCES_BUNDLE_REL = all_server_bundles()[0].macos_bundle_resources_root
# Finder droppings in the staged tree must not fail the nightly sign.
SERVER_RESOURCES_JUNK_FILES = {".DS_Store"}

BROWSER_PASSKEY_ENTITLEMENTS_NAME = "app-entitlements-browseros.plist"
BROWSER_PASSKEY_ENTITLEMENT = (
    "com.apple.developer.web-browser.public-key-credential"
)
BROWSER_KEYCHAIN_GROUP_SUFFIXES = (
    "devicetrust",
    "secure-payment-confirmation",
    "unexportable-keys",
    "webauthn",
    "webauthn-uvk",
)


@dataclass(frozen=True)
class BrowserPasskeyProductConfig:
    """Bind a release product to its app-specific Apple authorization profile.

    BrowserOS and BrowserOS neo use the same certificate and team, but Apple
    grants the managed capability to an exact App ID. Keeping the environment
    mapping product-owned prevents either profile from being used for the
    other bundle.
    """

    profile_env: str
    env_attr: str


BROWSER_PASSKEY_PRODUCTS = {
    "browseros": BrowserPasskeyProductConfig(
        profile_env="PROD_MACOS_BROWSEROS_PASSKEY_PROFILE_PATH",
        env_attr="macos_browseros_passkey_profile_path",
    ),
    "browserclaw": BrowserPasskeyProductConfig(
        profile_env="PROD_MACOS_BROWSERCLAW_PASSKEY_PROFILE_PATH",
        env_attr="macos_browserclaw_passkey_profile_path",
    ),
}


@dataclass(frozen=True)
class BrowserPasskeySigningInputs:
    """Validated inputs crossing Chromium's compile and Apple's signing boundary.

    Chromium bakes the keychain group into the framework, while codesign and the
    embedded provisioning profile authorize it later. Keeping the resolved
    values together prevents either side from silently drifting.
    """

    team_id: str
    bundle_id: str
    profile_path: Path
    entitlements_template: Path


def verify_server_resources_bundle(
    app_path: Path,
    chromium_src: Path,
    product_id: Optional[str] = None,
) -> List[str]:
    """Check bundled server resources match what the build staged."""
    problems: List[str] = []
    bundles = (
        server_bundles_for_product(product_id)
        if product_id
        else all_server_bundles()
    )
    for bundle in bundles:
        problems.extend(_verify_server_resource_bundle(bundle, app_path, chromium_src))
    return problems


def _verify_server_resource_bundle(
    bundle: ServerBundle, app_path: Path, chromium_src: Path
) -> List[str]:
    source_root = chromium_src / bundle.chromium_resources_root
    bundle_root = app_path / bundle.macos_bundle_resources_root
    if not source_root.is_dir():
        log_warning(
            f"Staged {bundle.name} resources not found at {source_root} - "
            "skipping bundle verification"
        )
        return []

    problems: List[str] = []
    bundle_label = bundle.macos_bundle_resources_root.as_posix()
    if not bundle_root.is_dir() and not bundle.required_in_chromium_output:
        log_warning(
            f"{bundle.name} bundle resources not found at {bundle_root} - "
            "skipping optional bundle verification"
        )
        return []

    staged = set()
    for source_file in sorted(source_root.rglob("*")):
        if not source_file.is_file() or source_file.name in SERVER_RESOURCES_JUNK_FILES:
            continue
        rel = source_file.relative_to(source_root)
        staged.add(rel)
        bundle_file = bundle_root / rel
        if not bundle_file.is_file():
            problems.append(
                f"{bundle_label}: missing from app bundle: {rel.as_posix()}"
            )
            continue
        if os.access(source_file, os.X_OK) and not os.access(bundle_file, os.X_OK):
            problems.append(
                f"{bundle_label}: lost executable bit in app bundle: {rel.as_posix()}"
            )

    if bundle_root.is_dir():
        for bundle_file in sorted(bundle_root.rglob("*")):
            if (
                not bundle_file.is_file()
                or bundle_file.name in SERVER_RESOURCES_JUNK_FILES
            ):
                continue
            rel = bundle_file.relative_to(bundle_root)
            if rel not in staged:
                log_warning(
                    f"App bundle has {bundle.name} file not in staged resources "
                    f"(stale?): {rel.as_posix()}"
                )

    return problems


def run_command(
    cmd: List[str],
    cwd: Optional[Path] = None,
    check: bool = True,
) -> subprocess.CompletedProcess:
    """Run a command and handle errors"""
    return utils_run_command(cmd, cwd=cwd, check=check)


def get_macos_keychain_path(env: Optional[EnvConfig] = None) -> Optional[Path]:
    """Return the explicitly configured macOS signing keychain."""
    value = env.macos_keychain_path if env else os.environ.get("MACOS_KEYCHAIN_PATH")
    if not value:
        return None
    return Path(value).expanduser()


def requires_browser_passkey_signing(ctx: Optional[Context]) -> bool:
    """Whether this release artifact may opt into the managed passkey capability."""
    return bool(
        ctx
        and ctx.product.id in BROWSER_PASSKEY_PRODUCTS
        and ctx.build_type == "release"
    )


def browser_passkey_groups(team_id: str, bundle_id: str) -> tuple[str, ...]:
    """Return every keychain group Chromium compiles for browser features."""
    prefix = f"{team_id}.{bundle_id}"
    return tuple(f"{prefix}.{suffix}" for suffix in BROWSER_KEYCHAIN_GROUP_SUFFIXES)


def get_browser_passkey_profile_path(
    env: Optional[EnvConfig], product_id: str
) -> Optional[Path]:
    """Resolve the uncommitted, app-specific profile used by release signing."""
    config = BROWSER_PASSKEY_PRODUCTS[product_id]
    value = getattr(env, config.env_attr) if env else os.environ.get(config.profile_env)
    return Path(value).expanduser() if value else None


def _plist_from_command_output(output: str, source: str) -> Dict[str, Any]:
    """Extract an XML plist from command output that may contain diagnostics."""
    start = output.find("<?xml")
    end = output.rfind("</plist>")
    if start < 0 or end < 0:
        raise RuntimeError(f"{source} did not contain an XML property list")
    end += len("</plist>")
    try:
        payload = plistlib.loads(output[start:end].encode("utf-8"))
    except (plistlib.InvalidFileException, ValueError) as exc:
        raise RuntimeError(f"Could not parse {source} property list: {exc}") from exc
    if not isinstance(payload, dict):
        raise RuntimeError(f"{source} property list is not a dictionary")
    return payload


def decode_provisioning_profile(profile_path: Path) -> Dict[str, Any]:
    """Decode a macOS provisioning profile through Apple's CMS tooling."""
    result = run_command(
        ["security", "cms", "-D", "-i", str(profile_path)], check=False
    )
    if result.returncode != 0:
        raise RuntimeError(f"Could not decode provisioning profile: {profile_path}")
    return _plist_from_command_output(result.stdout, "provisioning profile")


def _profile_allows_value(patterns: Any, value: str) -> bool:
    if not isinstance(patterns, list):
        return False
    return any(
        isinstance(pattern, str) and fnmatch.fnmatchcase(value, pattern)
        for pattern in patterns
    )


def validate_browser_passkey_profile(
    profile: Dict[str, Any], team_id: str, bundle_id: str
) -> None:
    """Validate Apple's profile allowlist before it is sealed into the app."""
    entitlements = profile.get("Entitlements")
    if not isinstance(entitlements, dict):
        raise RuntimeError("Provisioning profile has no Entitlements dictionary")

    expected_app_id = f"{team_id}.{bundle_id}"
    app_id = entitlements.get("com.apple.application-identifier") or entitlements.get(
        "application-identifier"
    )
    if app_id != expected_app_id:
        raise RuntimeError(
            "Provisioning profile application identifier does not match "
            f"the browser: expected {expected_app_id}, got {app_id or '<missing>'}"
        )

    profile_team = entitlements.get("com.apple.developer.team-identifier")
    if profile_team != team_id:
        raise RuntimeError(
            "Provisioning profile team does not match signing team: "
            f"expected {team_id}, got {profile_team or '<missing>'}"
        )

    if entitlements.get(BROWSER_PASSKEY_ENTITLEMENT) is not True:
        raise RuntimeError(
            f"Provisioning profile does not authorize {BROWSER_PASSKEY_ENTITLEMENT}"
        )

    allowed_groups = entitlements.get("keychain-access-groups")
    missing_groups = [
        group
        for group in browser_passkey_groups(team_id, bundle_id)
        if not _profile_allows_value(allowed_groups, group)
    ]
    if missing_groups:
        raise RuntimeError(
            "Provisioning profile does not authorize browser keychain groups: "
            + ", ".join(missing_groups)
        )


def validate_browser_passkey_entitlements(
    entitlements: Dict[str, Any], team_id: str, bundle_id: str, source: str
) -> None:
    """Check the concrete claims that codesign will attach to a browser app."""
    expected_app_id = f"{team_id}.{bundle_id}"
    if entitlements.get("com.apple.application-identifier") != expected_app_id:
        raise RuntimeError(
            f"{source} application identifier does not match {expected_app_id}"
        )

    claimed_groups = entitlements.get("keychain-access-groups")
    expected_groups = set(browser_passkey_groups(team_id, bundle_id))
    if not isinstance(claimed_groups, list) or not expected_groups.issubset(
        set(claimed_groups)
    ):
        raise RuntimeError(f"{source} is missing browser keychain access groups")

    if entitlements.get(BROWSER_PASSKEY_ENTITLEMENT) is not True:
        raise RuntimeError(f"{source} is missing {BROWSER_PASSKEY_ENTITLEMENT}")


def render_browser_passkey_entitlements(
    template_path: Path, output_path: Path, team_id: str, bundle_id: str
) -> None:
    """Resolve Chromium's signing placeholders into a codesign-ready plist."""
    template = template_path.read_text(encoding="utf-8")
    required_placeholders = ("${CHROMIUM_TEAM_ID}", "${CHROMIUM_BUNDLE_ID}")
    missing = [item for item in required_placeholders if item not in template]
    if missing:
        raise RuntimeError(
            f"Passkey entitlement template is missing placeholders: {', '.join(missing)}"
        )

    rendered = template.replace("${CHROMIUM_TEAM_ID}", team_id).replace(
        "${CHROMIUM_BUNDLE_ID}", bundle_id
    )
    if "${CHROMIUM_" in rendered:
        raise RuntimeError("Passkey entitlement template has unresolved placeholders")

    try:
        payload = plistlib.loads(rendered.encode("utf-8"))
    except (plistlib.InvalidFileException, ValueError) as exc:
        raise RuntimeError(f"Rendered passkey entitlements are invalid: {exc}") from exc
    if not isinstance(payload, dict):
        raise RuntimeError("Rendered passkey entitlements are not a dictionary")
    validate_browser_passkey_entitlements(
        payload, team_id, bundle_id, "Rendered passkey entitlements"
    )
    output_path.write_text(rendered, encoding="utf-8")


def _file_contains(path: Path, value: bytes) -> bool:
    """Search a large framework without loading the whole Mach-O into memory."""
    overlap = b""
    with path.open("rb") as binary:
        while chunk := binary.read(1024 * 1024):
            window = overlap + chunk
            if value in window:
                return True
            overlap = window[-max(len(value) - 1, 0) :]
    return False


def verify_compiled_browser_passkey_identity(
    app_path: Path, ctx: Context, team_id: str
) -> None:
    """Prove the built framework expects the group the signer will authorize."""
    framework_name = ctx.product.mac_framework_name(ctx.build_type)
    binary_name = framework_name.removesuffix(".framework")
    framework = app_path / "Contents" / "Frameworks" / framework_name
    candidates = (
        framework / "Versions" / "Current" / binary_name,
        framework / binary_name,
    )
    framework_binary = next((path for path in candidates if path.is_file()), None)
    if framework_binary is None:
        raise RuntimeError(f"Browser framework binary not found under {framework}")

    expected_group = f"{team_id}.{ctx.product.mac_bundle_id(ctx.build_type)}.webauthn"
    if not _file_contains(framework_binary, expected_group.encode("utf-8")):
        raise RuntimeError(
            "Compiled browser framework does not contain the signing keychain "
            f"group {expected_group}; rebuild after applying release BRANDING"
        )


def validate_browser_passkey_signing_inputs(
    app_path: Path, ctx: Optional[Context], team_id: str
) -> Optional[BrowserPasskeySigningInputs]:
    """Resolve and validate every release-only passkey signing input."""
    if not requires_browser_passkey_signing(ctx):
        return None
    assert ctx is not None
    if not team_id:
        raise RuntimeError(
            f"{ctx.product.display_name} passkey signing requires a macOS team ID"
        )

    config = BROWSER_PASSKEY_PRODUCTS[ctx.product.id]
    profile_path = get_browser_passkey_profile_path(ctx.env, ctx.product.id)
    if profile_path is None:
        # Managed-capability approval can lag a release by weeks. Preserve a
        # normally signed, fully usable browser while the profile is absent;
        # only platform passkeys are unavailable in this fallback mode.
        log_warning(
            f"{config.profile_env} is not configured; signing "
            f"{ctx.product.display_name} without macOS platform passkeys"
        )
        return None
    if not profile_path.is_file():
        raise RuntimeError(
            f"{ctx.product.display_name} passkey profile not found: {profile_path}"
        )

    entitlements_template = (
        ctx.chromium_src / "chrome" / "app" / BROWSER_PASSKEY_ENTITLEMENTS_NAME
    )
    if not entitlements_template.is_file():
        raise RuntimeError(
            f"Browser passkey entitlement template not found: {entitlements_template}"
        )

    bundle_id = ctx.product.mac_bundle_id(ctx.build_type)
    validate_browser_passkey_profile(
        decode_provisioning_profile(profile_path), team_id, bundle_id
    )
    verify_compiled_browser_passkey_identity(app_path, ctx, team_id)
    return BrowserPasskeySigningInputs(
        team_id=team_id,
        bundle_id=bundle_id,
        profile_path=profile_path,
        entitlements_template=entitlements_template,
    )


def _find_generic_app_entitlements(
    app_path: Path, root_dir: Path, ctx: Optional[Context]
) -> Optional[Path]:
    """Find the standard entitlements used when managed passkeys are disabled."""
    entitlements_dirs = []
    if ctx:
        entitlements_dirs.extend(
            [ctx.get_entitlements_dir(), ctx.chromium_src / "chrome" / "app"]
        )
    else:
        entitlements_dirs.extend(
            [
                join_paths(root_dir, "resources", "entitlements"),
                join_paths(root_dir, "entitlements"),
                join_paths(app_path.parent.parent.parent, "chrome", "app"),
            ]
        )

    for entitlements_name in (
        "app-entitlements.plist",
        "app-entitlements-chrome.plist",
    ):
        for entitlements_dir in entitlements_dirs:
            entitlements = join_paths(entitlements_dir, entitlements_name)
            if entitlements.exists():
                return entitlements
    return None


@contextmanager
def resolved_app_entitlements(
    app_path: Path,
    root_dir: Path,
    ctx: Optional[Context],
    passkey_inputs: Optional[BrowserPasskeySigningInputs],
) -> Iterator[Optional[Path]]:
    """Yield the final plist and keep temporary rendered claims alive for codesign."""
    if passkey_inputs is None:
        if ctx and ctx.product.id in BROWSER_PASSKEY_PRODUCTS:
            # Persistent Chromium outputs can retain bundle contents from an
            # earlier signing run. A build that omits the profile must not
            # silently reseal that stale authorization file, including when a
            # developer switches the same output between release and debug.
            (app_path / "Contents" / "embedded.provisionprofile").unlink(
                missing_ok=True
            )
        yield _find_generic_app_entitlements(app_path, root_dir, ctx)
        return

    # The profile must be embedded before the outer bundle is signed because
    # codesign seals it as bundle content. Temporary entitlements live only for
    # the codesign handoff and never enter release artifacts or source control.
    embedded_profile = app_path / "Contents" / "embedded.provisionprofile"
    shutil.copy2(passkey_inputs.profile_path, embedded_profile)
    log_info(f"  Embedded browser passkey profile: {embedded_profile}")

    with tempfile.TemporaryDirectory(prefix="browser-passkey-entitlements-") as tmp:
        rendered_path = Path(tmp) / "app-entitlements-browseros.plist"
        render_browser_passkey_entitlements(
            passkey_inputs.entitlements_template,
            rendered_path,
            passkey_inputs.team_id,
            passkey_inputs.bundle_id,
        )
        yield rendered_path


def _product_release_team_id(ctx: Context) -> str:
    """Read the team Chromium will compile from the release branding overlay."""
    branding = (
        ctx.root_dir
        / "chromium_files"
        / "products"
        / ctx.product.id
        / "chrome"
        / "app"
        / "theme"
        / "chromium"
        / "BRANDING.release"
    )
    if not branding.is_file():
        raise RuntimeError(
            f"{ctx.product.display_name} release branding not found: {branding}"
        )
    for line in branding.read_text(encoding="utf-8").splitlines():
        key, separator, value = line.partition("=")
        if separator and key == "MAC_TEAM_ID":
            if not value:
                raise RuntimeError(
                    f"{ctx.product.display_name} release MAC_TEAM_ID is empty"
                )
            return value
    raise RuntimeError(
        f"{ctx.product.display_name} release branding has no MAC_TEAM_ID"
    )


def unlock_keychain(env: Optional[EnvConfig] = None) -> None:
    """Unlock the configured signing keychain."""
    configured_keychain = get_macos_keychain_path(env)
    keychain_path = (
        configured_keychain
        if configured_keychain
        else Path.home() / "Library" / "Keychains" / "login.keychain-db"
    )
    password = (
        env.macos_keychain_password
        if env
        else os.environ.get("MACOS_KEYCHAIN_PASSWORD")
    )

    if not password:
        if configured_keychain:
            raise RuntimeError(
                "MACOS_KEYCHAIN_PASSWORD is required when MACOS_KEYCHAIN_PATH is set"
            )
        log_warning("MACOS_KEYCHAIN_PASSWORD not set — keychain may be locked (will fail over SSH)")
        return

    if not keychain_path.exists():
        if configured_keychain:
            raise RuntimeError(f"Configured keychain not found at {keychain_path}")
        log_warning(f"Keychain not found at {keychain_path}")
        return

    log_info(f"🔓 Unlocking macOS signing keychain: {keychain_path}")
    unlock_result = run_command(
        ["security", "unlock-keychain", "-p", password, str(keychain_path)],
        check=False,
    )
    # Prevent auto-lock during long signing + notarization runs
    settings_result = run_command(
        ["security", "set-keychain-settings", "-t", "3600", str(keychain_path)],
        check=False,
    )
    if configured_keychain and unlock_result.returncode != 0:
        raise RuntimeError(f"Failed to unlock configured keychain: {keychain_path}")
    if configured_keychain and settings_result.returncode != 0:
        raise RuntimeError(
            f"Failed to update configured keychain settings: {keychain_path}"
        )


@step(
    "sign_macos",
    phase="sign",
    platforms=("macos",),
    env=(
        "MACOS_CERTIFICATE_NAME",
        "PROD_MACOS_NOTARIZATION_APPLE_ID",
        "PROD_MACOS_NOTARIZATION_TEAM_ID",
        "PROD_MACOS_NOTARIZATION_PWD",
    ),
)
class MacOSSignModule(Step):
    produces = ["signed_app"]
    requires = ["built_app"]
    description = "Sign and notarize macOS application"

    def preflight(self, ctx: Context) -> None:
        """Reject an unusable managed-capability profile before the long build."""
        if not requires_browser_passkey_signing(ctx):
            return

        config = BROWSER_PASSKEY_PRODUCTS[ctx.product.id]
        profile_path = get_browser_passkey_profile_path(ctx.env, ctx.product.id)
        if profile_path is None:
            log_warning(
                f"{config.profile_env} is not configured; "
                f"{ctx.product.display_name} will be released without macOS "
                "platform passkeys"
            )
            return
        if not profile_path.is_file():
            raise ValidationError(
                f"{ctx.product.display_name} passkey profile not found: {profile_path}"
            )

        try:
            compiled_team_id = _product_release_team_id(ctx)
            configured_team_id = ctx.env.macos_notarization_team_id or ""
            if configured_team_id != compiled_team_id:
                raise RuntimeError(
                    "Notarization team does not match product release branding: "
                    f"expected {compiled_team_id}, got "
                    f"{configured_team_id or '<missing>'}"
                )
            validate_browser_passkey_profile(
                decode_provisioning_profile(profile_path),
                compiled_team_id,
                ctx.product.mac_bundle_id(ctx.build_type),
            )
        except RuntimeError as exc:
            raise ValidationError(str(exc)) from exc

    def validate(self, ctx: Context) -> None:
        # Platform + env vars are declared in @step metadata and checked
        # at plan time; the app is a mid-run artifact, so it stays here.
        app_path = ctx.get_app_path()
        if not app_path.exists():
            raise ValidationError(f"App not found at: {app_path}")

    def execute(self, ctx: Context) -> None:
        log_info("=" * 70)
        log_info(f"🚀 Starting signing process for {ctx.product.display_name}...")
        log_info("=" * 70)

        app_path = ctx.get_app_path()
        env_ok, env_vars = check_environment(ctx.env)
        if not env_ok:
            raise RuntimeError("Signing environment not configured")
        unlock_keychain(ctx.env)

        self._verify_server_resources(app_path, ctx)
        self._stamp_update_versions(app_path, ctx)
        self._clear_extended_attributes(app_path)
        self._sign_all_components(
            app_path,
            env_vars["certificate_name"],
            ctx,
            env_vars["keychain_path"],
            env_vars["team_id"],
        )
        self._verify_signature(app_path, ctx, env_vars["team_id"])
        self._notarize(app_path, env_vars, ctx)

        ctx.artifact_registry.add("signed_app", app_path)
        log_success("Application signed and notarized successfully")

    def _stamp_update_versions(self, app_path: Path, ctx: Context) -> None:
        """Bake the update identity into the outer bundle before signing.

        Sparkle compares the appcast's sparkle:version against
        CFBundleVersion, so it must carry the epoch-prefixed BrowserOS
        version (Context.get_sparkle_version) — the chromium build stamps
        BUILD.PATCH there, which belongs to the retired offset scheme.
        CFBundleShortVersionString is what Finder and Sparkle show users.
        """
        info_plist = app_path / "Contents" / "Info.plist"
        feed_version = ctx.get_sparkle_version()
        display_version = ctx.get_semantic_version()

        run_command(
            ["plutil", "-replace", "CFBundleVersion", "-string",
             feed_version, str(info_plist)]
        )
        run_command(
            ["plutil", "-replace", "CFBundleShortVersionString", "-string",
             display_version, str(info_plist)]
        )
        log_info(
            f"🏷️  Stamped CFBundleVersion={feed_version}, "
            f"CFBundleShortVersionString={display_version}"
        )

    def _verify_server_resources(self, app_path: Path, ctx: Context) -> None:
        problems = verify_server_resources_bundle(
            app_path,
            ctx.chromium_src,
            ctx.product.id,
        )
        if problems:
            raise RuntimeError(
                "App bundle does not match staged server resources "
                "(signing a stale build?):\n  " + "\n  ".join(problems)
            )

    def _clear_extended_attributes(self, app_path: Path) -> None:
        log_info("🧹 Clearing extended attributes...")
        run_command(["xattr", "-cs", str(app_path)])

    def _sign_all_components(
        self,
        app_path: Path,
        certificate_name: str,
        ctx: Context,
        keychain_path: str = "",
        team_id: str = "",
    ) -> None:
        if not sign_all_components(
            app_path,
            certificate_name,
            ctx.root_dir,
            ctx,
            Path(keychain_path) if keychain_path else None,
            team_id,
        ):
            raise RuntimeError("Failed to sign all components")

    def _verify_signature(
        self, app_path: Path, ctx: Optional[Context] = None, team_id: str = ""
    ) -> None:
        if not verify_signature(app_path, ctx, team_id):
            raise RuntimeError("Signature verification failed")

    def _notarize(self, app_path: Path, env_vars: Dict[str, str], ctx: Context) -> None:
        keychain_path = env_vars.get("keychain_path", "")
        if not notarize_app(
            app_path,
            ctx.root_dir,
            env_vars,
            ctx,
            Path(keychain_path) if keychain_path else None,
        ):
            raise RuntimeError("Notarization failed")


def check_signing_environment(env: Optional[EnvConfig] = None) -> bool:
    """Check if all required environment variables are set for signing (early check)

    Args:
        env: Optional EnvConfig instance. If not provided, creates a new one.
    """
    # Only check on macOS
    if not IS_MACOS():
        return True

    if env is None:
        env = EnvConfig()

    missing = []

    if not env.macos_certificate_name:
        missing.append("MACOS_CERTIFICATE_NAME")
    if not env.macos_notarization_apple_id:
        missing.append("PROD_MACOS_NOTARIZATION_APPLE_ID")
    if not env.macos_notarization_team_id:
        missing.append("PROD_MACOS_NOTARIZATION_TEAM_ID")
    if not env.macos_notarization_password:
        missing.append("PROD_MACOS_NOTARIZATION_PWD")

    if missing:
        log_error("❌ Signing requires macOS environment variables!")
        log_error(f"Missing environment variables: {', '.join(missing)}")
        log_error("Please set all required environment variables before signing.")
        return False

    return True


def check_environment(env: Optional[EnvConfig] = None) -> Tuple[bool, Dict[str, str]]:
    """Check if all required environment variables are set

    Args:
        env: Optional EnvConfig instance. If not provided, creates a new one.
    """
    if env is None:
        env = EnvConfig()

    env_vars = {
        "certificate_name": env.macos_certificate_name or "",
        "apple_id": env.macos_notarization_apple_id or "",
        "team_id": env.macos_notarization_team_id or "",
        "notarization_pwd": env.macos_notarization_password or "",
        "keychain_path": str(get_macos_keychain_path(env) or ""),
        "keychain_profile": "notarytool-profile",
    }

    missing = []
    for key, value in env_vars.items():
        if key in {"keychain_path", "keychain_profile"}:
            continue
        if not value:
            env_name = {
                "certificate_name": "MACOS_CERTIFICATE_NAME",
                "apple_id": "PROD_MACOS_NOTARIZATION_APPLE_ID",
                "team_id": "PROD_MACOS_NOTARIZATION_TEAM_ID",
                "notarization_pwd": "PROD_MACOS_NOTARIZATION_PWD",
            }[key]
            missing.append(env_name)

    if missing:
        log_error(f"Required environment variables not set: {', '.join(missing)}")
        return False, env_vars

    return True, env_vars


def find_components_to_sign(
    app_path: Path, ctx: Optional[Context] = None
) -> Dict[str, List[Path]]:
    """Dynamically find all components that need signing"""
    components = {
        "helpers": [],
        "xpc_services": [],
        "frameworks": [],
        "dylibs": [],
        "executables": [],
        "apps": [],
    }

    framework_path = join_paths(app_path, "Contents", "Frameworks")

    # Check both versioned and non-versioned paths for BrowserOS Framework
    # Handle both release and debug framework names
    if ctx:
        framework_names = [ctx.product.mac_framework_name(ctx.build_type)]
    else:
        framework_names = [
            "BrowserOS Framework.framework",
            "BrowserOS Dev Framework.framework",
            "BrowserOS neo Framework.framework",
            "BrowserOS neo Dev Framework.framework",
        ]
    nxtscape_framework_paths = []

    for fw_name in framework_names:
        fw_path = join_paths(framework_path, fw_name)
        if fw_path.exists():
            nxtscape_framework_paths.append(fw_path)

            # Add versioned path if context is available
            if ctx and ctx.browseros_chromium_version:
                versioned_path = join_paths(
                    fw_path, "Versions", ctx.browseros_chromium_version
                )
                if versioned_path.exists():
                    nxtscape_framework_paths.insert(
                        0, versioned_path
                    )  # Prioritize versioned path

    # Find all helper apps
    for nxtscape_fw_path in nxtscape_framework_paths:
        helpers_dir = join_paths(nxtscape_fw_path, "Helpers")
        if helpers_dir.exists():
            # Find all .app helpers
            components["helpers"].extend(helpers_dir.glob("*.app"))
            # Find all executable helpers (files without extension)
            for item in helpers_dir.iterdir():
                if item.is_file() and not item.suffix and os.access(item, os.X_OK):
                    components["executables"].append(item)
            break  # Use the first valid path found

    # Find all XPC services
    for xpc_path in framework_path.rglob("*.xpc"):
        components["xpc_services"].append(xpc_path)

    # Find all frameworks (with special handling for Sparkle)
    for fw_path in framework_path.rglob("*.framework"):
        components["frameworks"].append(fw_path)

        # Special handling for Sparkle framework versioned structure
        if "Sparkle.framework" in str(fw_path):
            # Look for Sparkle's versioned executables at Versions/B/
            sparkle_version_b = join_paths(fw_path, "Versions", "B")
            if sparkle_version_b.exists():
                # Add Autoupdate executable if it exists
                autoupdate = join_paths(sparkle_version_b, "Autoupdate")
                if autoupdate.exists() and autoupdate.is_file():
                    components["executables"].append(autoupdate)

    # Find all dylibs (check versioned path for BrowserOS Framework libraries)
    for nxtscape_fw_path in nxtscape_framework_paths:
        libraries_dir = join_paths(nxtscape_fw_path, "Libraries")
        if libraries_dir.exists():
            components["dylibs"].extend(libraries_dir.glob("*.dylib"))

    # Also find dylibs in other frameworks
    for dylib_path in framework_path.rglob("*.dylib"):
        if dylib_path not in components["dylibs"]:
            components["dylibs"].append(dylib_path)

    # Find all nested apps (like Updater.app in Sparkle)
    for nested_app in framework_path.rglob("*.app"):
        if nested_app not in components["helpers"]:
            components["apps"].append(nested_app)

    bundles = (
        server_bundles_for_product(ctx.product.id)
        if ctx
        else all_server_bundles()
    )
    for bundle in bundles:
        bundle_root = app_path / bundle.macos_bundle_resources_root
        if not bundle_root.exists():
            continue
        for item in bundle_root.rglob("*"):
            if (
                item.is_file()
                and not item.suffix
                and os.access(item, os.X_OK)
                and get_browseros_server_binary_info(item) is not None
            ):
                components["executables"].append(item)

    return components


def get_identifier_for_component(
    component_path: Path, base_identifier: str = "com.browseros"
) -> str:
    """Generate identifier for a component based on its path and name"""
    name = component_path.stem

    # Special cases for known components
    special_identifiers = {
        "Downloader": "org.sparkle-project.Downloader",
        "Installer": "org.sparkle-project.Installer",
        "Updater": "org.sparkle-project.Updater",
        "Autoupdate": "org.sparkle-project.Autoupdate",
        "Sparkle": "org.sparkle-project.Sparkle",
        "chrome_crashpad_handler": f"{base_identifier}.crashpad_handler",
        "app_mode_loader": f"{base_identifier}.app_mode_loader",
        "web_app_shortcut_copier": f"{base_identifier}.web_app_shortcut_copier",
    }

    # Check for special cases
    for key, identifier in special_identifiers.items():
        if key in str(component_path):
            return identifier

    # BrowserOS Server binaries share the same entitlements/options but need unique identifiers.
    browseros_server_info = get_browseros_server_binary_info(component_path)
    if browseros_server_info:
        suffix = browseros_server_info.get("identifier_suffix", component_path.stem)
        return f"{base_identifier}.{suffix}"

    # For helper apps
    if "Helper" in name:
        # Extract the helper type (GPU, Renderer, Plugin, Alerts)
        if "(" in name and ")" in name:
            helper_type = name[name.find("(") + 1 : name.find(")")].lower()
            return f"{base_identifier}.helper.{helper_type}"
        else:
            return f"{base_identifier}.helper"

    # For frameworks
    if component_path.suffix == ".framework":
        if name.endswith(" Framework") or name.endswith(" Dev Framework"):
            return f"{base_identifier}.framework"
        else:
            return f"{base_identifier}.{name.replace(' ', '_').lower()}"

    # For dylibs
    if component_path.suffix == ".dylib":
        return f"{base_identifier}.{name}"

    # Default
    return f"{base_identifier}.{name.replace(' ', '_').lower()}"


def get_signing_options(component_path: Path) -> str:
    """Determine signing options based on component type"""
    name = component_path.name

    # For Sparkle XPC services and apps - minimal restrictions
    if "sparkle" in str(component_path).lower():
        return "runtime"

    # For Chromium helper apps with specific sandboxing requirements
    if (
        "Helper (Renderer)" in name
        or "Helper (GPU)" in name
        or "Helper (Plugin)" in name
    ):
        return "restrict,kill,runtime"

    # Known BrowserOS Server binaries share the same relaxed options.
    browseros_server_info = get_browseros_server_binary_info(component_path)
    if browseros_server_info:
        return browseros_server_info.get("options", "runtime")

    # For dylibs - library flag ONLY for dynamic libraries
    if component_path.suffix == ".dylib":
        return "restrict,library,runtime,kill"

    # Default for other executables - no library flag
    return "runtime"


def _run_probe(cmd: List[str]) -> subprocess.CompletedProcess:
    """Run a read-only Mach-O inspection quietly (no build-log streaming)."""
    try:
        return subprocess.run(cmd, capture_output=True, text=True)
    except OSError as e:
        log_warning(f"Mach-O probe failed to run ({cmd[0]}): {e}")
        return subprocess.CompletedProcess(cmd, 1, stdout="", stderr="")


def get_macho_archs(path: Path) -> List[str]:
    """Architectures lipo reports for a file; empty when it is not Mach-O."""
    result = _run_probe(["lipo", "-archs", str(path)])
    if result.returncode != 0:
        return []
    return result.stdout.split()


def slice_has_embedded_info_plist(path: Path, arch: str) -> bool:
    """True if the given slice carries a __TEXT,__info_plist section."""
    result = _run_probe(["otool", "-arch", arch, "-l", str(path)])
    return result.returncode == 0 and "sectname __info_plist" in result.stdout


def find_asymmetric_info_plist_archs(path: Path) -> List[str]:
    """Archs of a fat file whose slices disagree on an embedded Info.plist.

    codesign, signing a fat file, binds the file-level Info.plist into every
    slice's CodeDirectory — a slice without the section then never validates
    and Apple's notary service rejects it (the upstream claude binary ships
    the section on arm64 only). Empty result = thin, symmetric, or not Mach-O.
    """
    # Symlinks excluded (matches the Go port's Lstat): os.replace would
    # silently turn a bundle symlink into a regular file.
    if path.is_symlink() or not path.is_file():
        return []
    archs = get_macho_archs(path)
    if len(archs) < 2:
        return []
    with_plist = sum(1 for arch in archs if slice_has_embedded_info_plist(path, arch))
    if with_plist in (0, len(archs)):
        return []
    return archs


def _codesign_cmd(
    component_path: Path,
    certificate_name: str,
    identifier: Optional[str] = None,
    options: Optional[str] = None,
    entitlements: Optional[Path] = None,
    keychain_path: Optional[Path] = None,
) -> List[str]:
    cmd = ["codesign", "--sign", certificate_name, "--force", "--timestamp"]

    if keychain_path:
        cmd.extend(["--keychain", str(keychain_path)])

    if identifier:
        cmd.extend(["--identifier", identifier])

    if options:
        cmd.extend(["--options", options])

    if entitlements and entitlements.exists():
        cmd.extend(["--entitlements", str(entitlements)])

    cmd.append(str(component_path))
    return cmd


def sign_fat_component_per_slice(
    component_path: Path,
    certificate_name: str,
    archs: List[str],
    identifier: Optional[str] = None,
    options: Optional[str] = None,
    entitlements: Optional[Path] = None,
    keychain_path: Optional[Path] = None,
) -> bool:
    """Sign each slice as a thin file and lipo them back together."""
    try:
        with tempfile.TemporaryDirectory(dir=component_path.parent) as tmp:
            tmp_dir = Path(tmp)
            thin_paths = []
            for arch in archs:
                thin = tmp_dir / f"{component_path.name}.{arch}"
                run_command(
                    ["lipo", str(component_path), "-thin", arch, "-output", str(thin)]
                )
                run_command(
                    _codesign_cmd(
                        thin,
                        certificate_name,
                        identifier,
                        options,
                        entitlements,
                        keychain_path,
                    )
                )
                thin_paths.append(thin)

            fat = tmp_dir / f"{component_path.name}.fat"
            run_command(
                ["lipo", "-create", *[str(p) for p in thin_paths], "-output", str(fat)]
            )
            shutil.copymode(component_path, fat)
            os.replace(fat, component_path)
        return True
    except Exception as e:
        log_error(f"Failed to sign {component_path} per-slice: {e}")
        return False


def sign_component(
    component_path: Path,
    certificate_name: str,
    identifier: Optional[str] = None,
    options: Optional[str] = None,
    entitlements: Optional[Path] = None,
    keychain_path: Optional[Path] = None,
) -> bool:
    """Sign a single component"""
    asymmetric_archs = find_asymmetric_info_plist_archs(component_path)
    if asymmetric_archs:
        log_warning(
            f"{component_path.name}: slices disagree on embedded Info.plist "
            f"({', '.join(asymmetric_archs)}) — signing per-slice"
        )
        return sign_fat_component_per_slice(
            component_path,
            certificate_name,
            asymmetric_archs,
            identifier,
            options,
            entitlements,
            keychain_path,
        )

    try:
        run_command(
            _codesign_cmd(
                component_path,
                certificate_name,
                identifier,
                options,
                entitlements,
                keychain_path,
            )
        )
        return True
    except Exception as e:
        log_error(f"Failed to sign {component_path}: {e}")
        return False


def sign_all_components(
    app_path: Path,
    certificate_name: str,
    root_dir: Path,
    ctx: Optional[Context] = None,
    keychain_path: Optional[Path] = None,
    team_id: str = "",
) -> bool:
    """Sign all components in the correct order (bottom-up)"""
    log_info("🔍 Discovering components to sign...")
    components = find_components_to_sign(app_path, ctx)
    base_identifier = (
        ctx.product.mac_signing_identifier(ctx.build_type) if ctx else "com.browseros"
    )
    main_identifier = (
        ctx.product.mac_signing_identifier(ctx.build_type)
        if ctx
        else "com.browseros.BrowserOS"
    )

    # Validate all compile/sign inputs before mutating nested signatures. A
    # stale debug build or wrong Apple profile would otherwise leave an app
    # half-resigned before failing at the outer bundle.
    try:
        passkey_inputs = validate_browser_passkey_signing_inputs(
            app_path, ctx, team_id
        )
    except RuntimeError as exc:
        log_error(f"Browser passkey signing validation failed: {exc}")
        return False

    # Print summary
    total_components = sum(len(items) for items in components.values())
    log_info(f"Found {total_components} components to sign:")
    for category, items in components.items():
        if items:
            log_info(f"  • {category}: {len(items)} items")

    # Sign in correct order (bottom-up)
    # 1. Sign XPC Services first
    log_info("\n🔏 Signing XPC Services...")
    for xpc in components["xpc_services"]:
        identifier = get_identifier_for_component(xpc, base_identifier)
        options = get_signing_options(xpc)
        if not sign_component(
            xpc, certificate_name, identifier, options, keychain_path=keychain_path
        ):
            return False

    # 2. Sign nested apps (like Sparkle's Updater.app)
    if components["apps"]:
        log_info("\n🔏 Signing nested applications...")
        for nested_app in components["apps"]:
            identifier = get_identifier_for_component(nested_app, base_identifier)
            options = get_signing_options(nested_app)
            if not sign_component(
                nested_app,
                certificate_name,
                identifier,
                options,
                keychain_path=keychain_path,
            ):
                return False

    # 3. Sign executables
    if components["executables"]:
        log_info("\n🔏 Signing executables...")
        # Get entitlements directory from context
        entitlements_dirs = []
        if ctx:
            entitlements_dirs.append(ctx.get_entitlements_dir())

        for exe in components["executables"]:
            identifier = get_identifier_for_component(exe, base_identifier)
            options = get_signing_options(exe)

            # Check for specific entitlements
            entitlements = None
            browseros_server_info = get_browseros_server_binary_info(exe)
            if browseros_server_info:
                entitlements_name = browseros_server_info.get("entitlements")
                if entitlements_name:
                    for ent_dir in entitlements_dirs:
                        ent_path = join_paths(ent_dir, entitlements_name)
                        if ent_path.exists():
                            entitlements = ent_path
                            break

            if not sign_component(
                exe,
                certificate_name,
                identifier,
                options,
                entitlements,
                keychain_path,
            ):
                return False

    # 4. Sign dylibs
    if components["dylibs"]:
        log_info("\n🔏 Signing dynamic libraries...")
        for dylib in components["dylibs"]:
            identifier = get_identifier_for_component(dylib, base_identifier)
            if not sign_component(
                dylib, certificate_name, identifier, keychain_path=keychain_path
            ):
                return False

    # 5. Sign helper apps
    if components["helpers"]:
        log_info("\n🔏 Signing helper applications...")
        # Get entitlements directory from context
        entitlements_dirs = []
        if ctx:
            entitlements_dirs.append(ctx.get_entitlements_dir())

        for helper in components["helpers"]:
            identifier = get_identifier_for_component(helper, base_identifier)
            options = get_signing_options(helper)

            # Check for specific entitlements
            entitlements = None
            entitlements_name = None

            if "Renderer" in helper.name:
                entitlements_name = "helper-renderer-entitlements.plist"
            elif "GPU" in helper.name:
                entitlements_name = "helper-gpu-entitlements.plist"
            elif "Plugin" in helper.name:
                entitlements_name = "helper-plugin-entitlements.plist"

            if entitlements_name:
                for ent_dir in entitlements_dirs:
                    ent_path = join_paths(ent_dir, entitlements_name)
                    if ent_path.exists():
                        entitlements = ent_path
                        break

            if not sign_component(
                helper,
                certificate_name,
                identifier,
                options,
                entitlements,
                keychain_path,
            ):
                return False

    # 6. Sign frameworks (except the main BrowserOS Framework)
    if components["frameworks"]:
        log_info("\n🔏 Signing frameworks...")
        # Sort to sign Sparkle.framework before BrowserOS Framework.framework
        frameworks_sorted = sorted(
            components["frameworks"], key=lambda x: 0 if "Sparkle" in x.name else 1
        )
        for framework in frameworks_sorted:
            identifier = get_identifier_for_component(framework, base_identifier)
            if not sign_component(
                framework, certificate_name, identifier, keychain_path=keychain_path
            ):
                return False

    # 7. Sign main executable
    log_info("\n🔏 Signing main executable...")
    # Handle both release and debug executable names
    main_exe_names = (
        [ctx.product.display_name, ctx.product.dev_display_name]
        if ctx
        else ["BrowserOS", "BrowserOS Dev"]
    )
    main_exe = None
    for exe_name in main_exe_names:
        exe_path = join_paths(app_path, "Contents", "MacOS", exe_name)
        if exe_path.exists():
            main_exe = exe_path
            break

    if not main_exe:
        log_error(
            f"Main executable not found in {join_paths(app_path, 'Contents', 'MacOS')}"
        )
        return False

    if not sign_component(
        main_exe, certificate_name, main_identifier, keychain_path=keychain_path
    ):
        return False

    # 8. Finally sign the app bundle
    log_info("\n🔏 Signing application bundle...")
    requirements = (
        f'=designated => identifier "{main_identifier}" and '
        "anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] /* exists */ and "
        "certificate leaf[field.1.2.840.113635.100.6.1.13] /* exists */"
    )

    try:
        with resolved_app_entitlements(
            app_path, root_dir, ctx, passkey_inputs
        ) as entitlements:
            cmd = [
                "codesign",
                "--sign",
                certificate_name,
                "--force",
                "--timestamp",
                "--identifier",
                main_identifier,
                "--options",
                "restrict,library,runtime,kill",
                "--requirements",
                requirements,
            ]

            if keychain_path:
                cmd.extend(["--keychain", str(keychain_path)])

            if entitlements:
                log_info(f"  Using entitlements: {entitlements}")
                cmd.extend(["--entitlements", str(entitlements)])
            else:
                log_warning("No app entitlements file found, signing without entitlements")

            cmd.append(str(app_path))
            run_command(cmd)
    except Exception as exc:
        log_error(f"Failed to sign application bundle: {exc}")
        return False

    return True


def verify_browser_passkey_signature(
    app_path: Path, ctx: Optional[Context], team_id: str
) -> None:
    """Verify the signed app, embedded profile, and compiled identity agree.

    A valid generic Developer ID signature is insufficient for platform
    passkeys. The OS also requires the managed entitlement and keychain groups
    to be authorized by the profile sealed into the outer app bundle.
    """
    if not requires_browser_passkey_signing(ctx):
        return
    assert ctx is not None
    if get_browser_passkey_profile_path(ctx.env, ctx.product.id) is None:
        return

    signature = run_command(
        ["codesign", "--display", "--verbose=4", str(app_path)], check=False
    )
    if signature.returncode != 0:
        raise RuntimeError("Could not inspect the browser code signature")
    team_match = re.search(r"(?:^|\n)TeamIdentifier=([^\n]+)", signature.stdout)
    signed_team_id = team_match.group(1).strip() if team_match else ""
    if signed_team_id != team_id:
        raise RuntimeError(
            "Signed app team does not match the configured team: "
            f"expected {team_id}, got {signed_team_id or '<missing>'}"
        )

    claims = run_command(
        [
            "codesign",
            "--display",
            "--entitlements",
            "-",
            "--xml",
            str(app_path),
        ],
        check=False,
    )
    if claims.returncode != 0:
        raise RuntimeError("Could not inspect the browser signed entitlements")
    validate_browser_passkey_entitlements(
        _plist_from_command_output(claims.stdout, "signed entitlements"),
        team_id,
        ctx.product.mac_bundle_id(ctx.build_type),
        f"Signed {ctx.product.display_name} entitlements",
    )

    embedded_profile = app_path / "Contents" / "embedded.provisionprofile"
    if not embedded_profile.is_file():
        raise RuntimeError("Signed browser app has no embedded provisioning profile")
    validate_browser_passkey_profile(
        decode_provisioning_profile(embedded_profile),
        team_id,
        ctx.product.mac_bundle_id(ctx.build_type),
    )


def verify_signature(
    app_path: Path, ctx: Optional[Context] = None, team_id: str = ""
) -> bool:
    """Verify application signature"""
    log_info("\n🔍 Verifying application signature integrity...")

    result = run_command(
        ["codesign", "--verify", "--deep", "--strict", "--verbose=2", str(app_path)],
        check=False,
    )

    if result.returncode != 0:
        log_error("Signature verification failed!")
        return False

    # --deep seals plain executables under Resources/ as files without
    # validating their own signatures (Apple's notary does, per slice) —
    # verify each file-type component directly so a bad slice fails here
    # instead of after a multi-minute notarization round-trip. Helpers,
    # frameworks, and XPC services are proper sub-bundles --deep already
    # recurses into.
    components = find_components_to_sign(app_path, ctx)
    for component in components["executables"] + components["dylibs"]:
        result = run_command(
            ["codesign", "--verify", "--verbose=2", str(component)],
            check=False,
        )
        if result.returncode != 0:
            log_error(f"Component signature verification failed: {component}")
            return False

    try:
        verify_browser_passkey_signature(app_path, ctx, team_id)
    except RuntimeError as exc:
        log_error(f"Browser passkey signature verification failed: {exc}")
        return False

    log_success("Signature verification passed")
    return True


def notarize_app(
    app_path: Path,
    root_dir: Path,
    env_vars: Dict[str, str],
    ctx: Optional[Context] = None,
    keychain_path: Optional[Path] = None,
) -> bool:
    """Notarize the application"""
    log_info("\n📤 Preparing for notarization...")

    # Create zip for notarization
    notarize_zip = (
        ctx.get_notarization_zip() if ctx else join_paths(root_dir, "notarize.zip")
    )
    if notarize_zip.exists():
        notarize_zip.unlink()

    run_command(["ditto", "-c", "-k", "--keepParent", str(app_path), str(notarize_zip)])
    log_success("Archive created for notarization")

    # Store credentials
    log_info("🔑 Storing notarization credentials...")
    profile = env_vars.get("keychain_profile", "notarytool-profile")
    store_cmd = [
        "xcrun",
        "notarytool",
        "store-credentials",
        profile,
        "--apple-id",
        env_vars["apple_id"],
        "--team-id",
        env_vars["team_id"],
        "--password",
        env_vars["notarization_pwd"],
    ]
    if keychain_path:
        store_cmd.extend(["--keychain", str(keychain_path)])
    store_result = run_command(store_cmd, check=False)

    if keychain_path and store_result.returncode != 0:
        log_error("Failed to store notarization credentials in configured keychain")
        notarize_zip.unlink(missing_ok=True)
        return False

    # Submit for notarization — if store-credentials failed, pass creds
    # directly to avoid depending on the keychain profile.
    log_info("📤 Submitting application for notarization (this may take a while)...")
    use_keychain_profile = store_result.returncode == 0
    if use_keychain_profile:
        submit_cmd = [
            "xcrun",
            "notarytool",
            "submit",
            str(notarize_zip),
            "--keychain-profile",
            profile,
            *notarytool_wait_args(),
        ]
        if keychain_path:
            submit_cmd.extend(["--keychain", str(keychain_path)])
    else:
        log_warning("Keychain profile unavailable — passing credentials directly")
        submit_cmd = [
            "xcrun",
            "notarytool",
            "submit",
            str(notarize_zip),
            "--apple-id",
            env_vars["apple_id"],
            "--team-id",
            env_vars["team_id"],
            "--password",
            env_vars["notarization_pwd"],
            *notarytool_wait_args(),
        ]
    result = run_command(submit_cmd, check=False)

    log_info(result.stdout)
    if result.stderr:
        log_error(result.stderr)

    if result.returncode != 0:
        log_error("Notarization submission failed")
        return False

    # Check if accepted
    if "status: Accepted" not in result.stdout:
        log_error("App notarization failed - status was not 'Accepted'")
        # Try to extract submission ID for debugging
        for line in result.stdout.split("\n"):
            if "id:" in line:
                submission_id = line.split("id:")[1].strip().split()[0]
                log_info(
                    f'Get detailed logs with: xcrun notarytool log {submission_id} --keychain-profile "{profile}"'
                )
                break
        return False

    log_success("App notarization successful - status: Accepted")

    # Staple the ticket
    log_info("📎 Stapling notarization ticket to application...")
    result = run_command(["xcrun", "stapler", "staple", str(app_path)], check=False)

    if result.returncode != 0:
        log_error("Failed to staple notarization ticket!")
        return False

    log_success("Notarization ticket stapled successfully")

    # Clean up
    notarize_zip.unlink()

    # Verify notarization
    log_info("\n🔍 Verifying notarization status...")

    # Check Gatekeeper
    result = run_command(["spctl", "-a", "-vvv", str(app_path)], check=False)

    if result.returncode != 0:
        log_error("Gatekeeper check failed!")
        return False

    # Validate stapling
    result = run_command(["xcrun", "stapler", "validate", str(app_path)], check=False)

    if result.returncode != 0:
        log_error("Stapler validation failed!")
        return False

    log_success("Notarization and stapling verification passed")
    return True


def sign_app(ctx: Context, create_dmg: bool = True) -> bool:
    """Main signing function that uses BuildContext from bos_build.py"""
    log_info("=" * 70)
    log_info(f"🚀 Starting signing process for {ctx.product.display_name}...")
    log_info("=" * 70)

    # Error tracking similar to bash script
    error_count = 0
    error_messages = []

    def track_error(msg: str):
        nonlocal error_count
        error_count += 1
        error_messages.append(f"ERROR {error_count}: {msg}")
        log_error(msg)

    # Check environment
    env_ok, env_vars = check_environment(ctx.env if ctx else None)
    if not env_ok:
        return False
    unlock_keychain(ctx.env if ctx else None)
    keychain_path = (
        Path(env_vars["keychain_path"]) if env_vars["keychain_path"] else None
    )

    # Setup app path
    app_path = ctx.get_app_path()

    # Setup DMG path if needed
    dmg_path = None
    if create_dmg:
        dmg_dir = ctx.get_dist_dir()
        dmg_name = ctx.get_artifact_name("dmg")
        dmg_path = join_paths(dmg_dir, dmg_name)

    # Verify app exists
    if not app_path.exists():
        log_error(f"App not found at: {app_path}")
        return False

    problems = verify_server_resources_bundle(app_path, ctx.chromium_src, ctx.product.id)
    if problems:
        log_error(
            "App bundle does not match staged server resources "
            "(signing a stale build?):"
        )
        for problem in problems:
            log_error(f"  {problem}")
        return False

    try:
        # Clear extended attributes
        log_info("🧹 Clearing extended attributes...")
        run_command(["xattr", "-cs", str(app_path)])

        # Sign all components
        if not sign_all_components(
            app_path,
            env_vars["certificate_name"],
            ctx.root_dir,
            ctx,
            keychain_path,
            env_vars["team_id"],
        ):
            return False

        # Verify signature
        if not verify_signature(app_path, ctx, env_vars["team_id"]):
            return False

        # Notarize app
        if not notarize_app(app_path, ctx.root_dir, env_vars, ctx, keychain_path):
            return False

        # Create and notarize DMG if requested
        if create_dmg:
            print("\n" + "=" * 70)
            log_info("📦 Creating and notarizing DMG package")
            log_info("=" * 70)

            from ..package.macos import create_signed_notarized_dmg

            # Find pkg-dmg tool
            pkg_dmg_path = ctx.get_pkg_dmg_path()

            # Create, sign, and notarize DMG
            if dmg_path and not create_signed_notarized_dmg(
                app_path=app_path,
                dmg_path=dmg_path,
                certificate_name=env_vars["certificate_name"],
                volume_name=ctx.product.mac.dmg_volume_name,
                pkg_dmg_path=pkg_dmg_path,
                keychain_profile=env_vars["keychain_profile"],
                keychain_path=keychain_path,
                notarization_env=env_vars,
            ):
                log_error("DMG creation/notarization failed")
                return False

    except Exception as e:
        track_error(f"Unexpected error: {e}")
        import traceback

        traceback.print_exc()
        error_count += 1  # For the exception itself

    # Summary report (similar to bash script)
    log_info("=" * 70)
    if error_count > 0:
        log_error(f"Process completed with {error_count} errors:")
        for msg in error_messages:
            log_error(f"  {msg}")
        log_error("Review the errors above and address them before distribution.")
        if create_dmg:
            log_warning(f"Final DMG created at: {dmg_path} (may have issues)")
        return False
    else:
        log_success("Process completed successfully!")
        if create_dmg:
            log_info(f"Final DMG created at: {dmg_path}")
        log_info("The application is properly signed, notarized, and packaged.")
        log_info("=" * 70)
    return error_count == 0
