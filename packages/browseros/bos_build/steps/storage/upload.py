#!/usr/bin/env python3
"""Upload module for BrowserOS build artifacts to Cloudflare R2"""

import json
import hashlib
import os
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple, cast

from ...core.step import Step, ValidationError, step
from ...core.context import Context
from ...lib.utils import (
    log_info,
    log_error,
    log_success,
    log_warning,
    IS_WINDOWS,
    IS_MACOS,
)

from ...lib.r2 import (
    BOTO3_AVAILABLE,
    get_r2_client,
    get_release_json,
    upload_file_to_r2,
)
from ...release.prepared_resources import (
    PreparedResourcesManifest,
    load_prepared_resources,
)
from ...release.components import normalize_component_version
from ...products.resource_sources import source_resources_for_product
from ..package.linux_packaging import require_linux_artifacts


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def _source_manifest(ctx: Context) -> PreparedResourcesManifest:
    manifest = ctx.artifact_registry.get("prepared_resources")
    if not isinstance(manifest, PreparedResourcesManifest):
        prepared = getattr(ctx, "prepared_resources", None)
        if not isinstance(prepared, Path):
            raise ValueError("Source release metadata requires prepared resources")
        manifest = load_prepared_resources(prepared)
    if manifest.product != ctx.product.id:
        raise ValueError("Prepared-resource product does not match release metadata")
    if manifest.source_sha != ctx.source_sha:
        raise ValueError("Prepared-resource source SHA does not match release metadata")
    if manifest.browser_version != ctx.get_semantic_version():
        raise ValueError(
            "Prepared-resource browser version does not match release metadata"
        )
    return manifest


def _release_provenance(ctx: Context) -> dict[str, object]:
    provenance: dict[str, object] = {
        "source_sha": os.environ.get(
            "BROWSEROS_BUILD_SOURCE_SHA", os.environ.get("GITHUB_SHA", "")
        ),
    }
    if getattr(ctx, "resource_mode", "published") == "published":
        source = source_resources_for_product(ctx.product.id)
        server_version = (
            ctx.env.browseros_server_resource_version
            if ctx.product.id == "browseros"
            else ctx.env.browserclaw_server_resource_version
        )
        component_versions = {
            source.server_component: server_version,
            source.extension_component: ctx.env.bundled_product_extension_version,
            source.onboarding_component: ctx.env.onboarding_resource_version,
        }
        if all(component_versions.values()):
            provenance["component_versions"] = component_versions
    if getattr(ctx, "resource_mode", "published") == "source":
        manifest = _source_manifest(ctx)
        provenance = {
            "source_sha": manifest.source_sha,
            "parent_sha": manifest.parent_sha,
            "component_versions": dict(manifest.component_versions),
            "common_manifest_digest": manifest.digest(),
        }
    provenance.update(
        {
            "reservation_sha": os.environ.get("BROWSEROS_BUILD_RESERVATION_SHA", ""),
            "workflow_run_id": os.environ.get("GITHUB_RUN_ID", ""),
            "workflow_run_attempt": os.environ.get("GITHUB_RUN_ATTEMPT", ""),
        }
    )
    return {key: value for key, value in provenance.items() if value}


def _get_platform() -> str:
    """Get platform name for R2 path"""
    if IS_MACOS():
        return "macos"
    elif IS_WINDOWS():
        return "win"
    else:
        return "linux"


@step("upload", phase="upload")
class UploadModule(Step):
    """Upload build artifacts to Cloudflare R2"""

    produces = []
    requires = []
    description = "Upload build artifacts to Cloudflare R2"

    def validate(self, ctx: Context) -> None:
        # Family nightlies persist the complete receipt in Actions first and
        # publish it only after both signed builds and the state merge pass.
        if os.environ.get("BROWSEROS_DEFER_R2_UPLOAD") == "1":
            return
        if not BOTO3_AVAILABLE:
            raise ValidationError(
                "boto3 library not installed - run: pip install boto3"
            )

        if not ctx.env.has_r2_config():
            raise ValidationError(
                "R2 configuration not set. Required env vars: "
                "R2_ACCOUNT_ID, R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY"
            )

    def execute(self, ctx: Context) -> None:
        log_info("\nUploading package artifacts to R2...")

        extra_metadata = {}
        sparkle_signatures = cast(
            Optional[dict[str, tuple[str, int]]],
            ctx.artifact_registry.get("sparkle_signatures"),
        )
        if sparkle_signatures:
            for filename, (sig, length) in sparkle_signatures.items():
                extra_metadata[filename] = {
                    "sparkle_signature": sig,
                    "sparkle_length": length,
                }

        success, release_json = upload_release_artifacts(ctx, extra_metadata)
        if not success:
            raise RuntimeError("Failed to upload artifacts to R2")


def generate_release_json(
    ctx: Context,
    artifacts: List[Dict],
    platform: str,
) -> Dict:
    """Generate release metadata for one platform."""
    env = ctx.env

    release_data = {
        "product": ctx.product.id,
        "product_name": ctx.product.display_name,
        "platform": platform,
        "version": ctx.get_semantic_version(),
        "chromium_version": ctx.chromium_version,
        "browseros_chromium_version": ctx.browseros_chromium_version,
        "build_date": datetime.now(timezone.utc).isoformat(),
        "artifacts": {},
    }
    release_data.update(_release_provenance(ctx))

    # Sparkle (macOS) and WinSparkle (Windows) both compare against this
    # epoch-prefixed BrowserOS version in the appcast (Context.get_sparkle_version).
    if platform in ("macos", "win"):
        release_data["sparkle_version"] = ctx.get_sparkle_version()

    base_url = f"{env.r2_cdn_base_url}/{ctx.get_release_path(platform)}"

    for artifact in artifacts:
        filename = artifact["filename"]
        artifact_key = artifact.get("release_key") or _get_artifact_key(
            filename,
            platform,
        )

        artifact_data = {
            "filename": filename,
            "url": f"{base_url}{filename}",
        }

        for key, value in artifact.items():
            if key not in ("filename", "release_key"):
                artifact_data[key] = value

        # Linux architectures publish serially into one receipt. Record each
        # writer's attempt so retrying arm64 cannot relabel an earlier x64 build.
        if platform == "linux" and release_data.get("workflow_run_attempt"):
            artifact_data["workflow_run_attempt"] = release_data["workflow_run_attempt"]

        release_data["artifacts"][artifact_key] = artifact_data

    return release_data


def _has_pinned_linux_identity(release: Dict) -> bool:
    """A named run alone cannot freeze mutable latest resource downloads."""
    if not release.get("workflow_run_id") or not release.get("source_sha"):
        return False
    versions = release.get("component_versions")
    if not isinstance(versions, dict):
        return False
    try:
        source = source_resources_for_product(release.get("product", ""))
        for name in (
            source.server_component,
            source.extension_component,
            source.onboarding_component,
        ):
            version = versions.get(name)
            if not isinstance(version, str):
                return False
            # Nonempty aliases such as `latest` still select mutable objects.
            # Use each resource's existing release-version rules, not truthiness.
            normalize_component_version(name, version)
    except ValueError:
        return False
    return True


def merge_release_metadata(existing: Optional[Dict], new: Dict) -> Dict:
    """Merge serialized architecture uploads only within one release identity.

    Linux may retain a successful sibling across workflow attempts. Its artifact
    attempts stay explicit; a mixed receipt cannot claim a single platform attempt.
    The caller owns serialization of the remote read/merge/write operation.
    """
    if not existing:
        return new

    linux = existing.get("platform") == new.get("platform") == "linux"
    provenance_fields = (
        "source_sha",
        "reservation_sha",
        "parent_sha",
        "component_versions",
        "common_manifest_digest",
        "workflow_run_id",
    )
    if linux:
        provenance_fields += (
            "product",
            "version",
            "platform",
            "chromium_version",
            "browseros_chromium_version",
        )
    # Unpinned standalone builds may resolve different latest resources on a
    # retry. Retain another attempt only with complete resource identities; the
    # equality check below still requires those pins and the source/run to match.
    if not linux or not _has_pinned_linux_identity(new):
        provenance_fields += ("workflow_run_attempt",)
    if any(existing.get(field) != new.get(field) for field in provenance_fields):
        return new

    merged = dict(existing)
    merged.update({key: value for key, value in new.items() if key != "artifacts"})

    artifacts = {}
    for document in (existing, new):
        for key, artifact in document.get("artifacts", {}).items():
            fields = {**artifacts.get(key, {}), **artifact}
            if linux:
                # Older receipts recorded the attempt only at platform level.
                attempt = artifact.get(
                    "workflow_run_attempt", document.get("workflow_run_attempt")
                )
                if attempt:
                    fields["workflow_run_attempt"] = attempt
                else:
                    fields.pop("workflow_run_attempt", None)
            artifacts[key] = fields
    merged["artifacts"] = artifacts
    if linux:
        attempts = {
            artifact.get("workflow_run_attempt") for artifact in artifacts.values()
        }
        if len(attempts) == 1 and (attempt := next(iter(attempts))):
            merged["workflow_run_attempt"] = attempt
        else:
            merged.pop("workflow_run_attempt", None)
    return merged


def _get_linux_artifact_key(filename: str) -> Optional[str]:
    lower = filename.lower()

    if ".appimage" in lower:
        if "arm64" in lower or "aarch64" in lower:
            return "arm64_appimage"
        if "x64" in lower or "x86_64" in lower:
            return "x64_appimage"
    elif ".deb" in lower:
        if "arm64" in lower or "aarch64" in lower:
            return "arm64_deb"
        if "amd64" in lower or "x64" in lower or "x86_64" in lower:
            return "x64_deb"

    return None


def _get_artifact_key(filename: str, platform: str) -> str:
    """Derive a release artifact key from its filename."""
    lower = filename.lower()

    if platform == "macos":
        if "arm64" in lower:
            return "arm64"
        elif "x64" in lower or "x86_64" in lower:
            return "x64"
        elif "universal" in lower:
            return "universal"

    elif platform == "win":
        if "installer.exe" in lower:
            return "arm64_installer" if "arm64" in lower else "x64_installer"
        elif "installer.zip" in lower:
            return "arm64_zip" if "arm64" in lower else "x64_zip"

    elif platform == "linux":
        artifact_key = _get_linux_artifact_key(filename)
        if artifact_key:
            return artifact_key
        log_warning(f"Unrecognized Linux artifact name: {filename}; using stem key")

    return Path(filename).stem


def _product_artifact_filename_prefix(ctx: Context) -> str:
    """Return the exact filename prefix for this product/version's artifacts."""
    return f"{ctx.product.artifact_prefix}_v{ctx.get_semantic_version()}_"


def _filter_product_artifacts(ctx: Context, artifacts: List[Path]) -> List[Path]:
    expected_prefix = _product_artifact_filename_prefix(ctx)
    filtered = [
        artifact for artifact in artifacts if artifact.name.startswith(expected_prefix)
    ]
    skipped = [artifact.name for artifact in artifacts if artifact not in filtered]
    if skipped:
        log_warning(
            "Ignoring artifact(s) that do not belong to "
            f"{ctx.product.id} v{ctx.get_semantic_version()}: {', '.join(skipped)}"
        )
    return filtered


def detect_artifacts(ctx: Context) -> List[Path]:
    """Find the active product's artifacts for the current platform."""
    if not IS_MACOS() and not IS_WINDOWS():
        # Linux is one correlated release result. The resolver handles both
        # in-process registry handoff and deliberate exact-name disk recovery;
        # allowing a glob here would make partial packages publishable again.
        return list(require_linux_artifacts(ctx).paths)

    dist_dir = ctx.get_dist_dir()
    if not dist_dir.exists():
        return []

    artifacts = []

    if IS_MACOS():
        artifacts.extend(dist_dir.glob("*.dmg"))
    elif IS_WINDOWS():
        artifacts.extend(dist_dir.glob("*.exe"))
        artifacts.extend(dist_dir.glob("*.zip"))
    return sorted(_filter_product_artifacts(ctx, artifacts))


def upload_release_artifacts(
    ctx: Context,
    extra_metadata: Optional[Dict[str, Dict[str, Any]]] = None,
) -> Tuple[bool, Optional[Dict]]:
    """Upload release artifacts and their metadata to R2."""
    deferred = os.environ.get("BROWSEROS_DEFER_R2_UPLOAD") == "1"
    if not deferred and not BOTO3_AVAILABLE:
        log_warning("boto3 not installed. Skipping R2 upload.")
        log_info("Install with: pip install boto3")
        return True, None

    env = ctx.env

    if not deferred and not env.has_r2_config():
        log_warning("R2 configuration not set. Skipping upload.")
        return True, None

    platform = _get_platform()
    artifacts = detect_artifacts(ctx)
    if not artifacts:
        log_info("No artifacts found to upload")
        return True, None

    release_path = ctx.get_release_path(platform)

    if deferred:
        log_info("\nPreparing deferred immutable release receipt")
    else:
        log_info(f"\nUploading to R2: {env.r2_bucket}/{release_path}")
    log_info(f"Found {len(artifacts)} artifact(s):")
    for artifact in artifacts:
        log_info(f"  - {artifact.name}")

    artifact_metadata = []
    for index, artifact_path in enumerate(artifacts):
        metadata = {
            "filename": artifact_path.name,
            "size": artifact_path.stat().st_size,
            "sha256": _sha256(artifact_path),
        }

        if extra_metadata and artifact_path.name in extra_metadata:
            metadata.update(extra_metadata[artifact_path.name])

        if platform == "linux":
            # `detect_artifacts` returns LinuxArtifactPair.paths in this fixed
            # order, so release identity comes from the deep interface rather
            # than reparsing architecture tokens from filenames. Write this
            # after optional metadata so callers cannot redefine identity.
            format_name = ("appimage", "deb")[index]
            metadata["release_key"] = f"{ctx.architecture}_{format_name}"

        artifact_metadata.append(metadata)

    release_data = generate_release_json(ctx, artifact_metadata, platform)
    release_json_path = ctx.get_dist_dir() / "release.json"
    release_json_path.write_text(json.dumps(release_data, indent=2))
    if deferred:
        # The receipt and DMG travel together as one Actions artifact. Keeping
        # this step local prevents either product from becoming public before
        # its sibling build and the family state transaction have succeeded.
        ctx.artifact_registry.add("release_metadata", release_data)
        log_success("Prepared release receipt for deferred immutable publication")
        return True, release_data

    client = get_r2_client(env)
    if not client:
        log_error("Failed to create R2 client")
        return False, None

    for artifact_path in artifacts:
        r2_key = f"{release_path}{artifact_path.name}"
        if not upload_file_to_r2(client, artifact_path, r2_key, env.r2_bucket):
            return False, None

    existing_release_data = get_release_json(
        ctx.get_semantic_version(), platform, env, ctx.product.id
    )
    release_data = merge_release_metadata(existing_release_data, release_data)
    release_json_path.write_text(json.dumps(release_data, indent=2))

    r2_key = f"{release_path}release.json"
    if not upload_file_to_r2(client, release_json_path, r2_key, env.r2_bucket):
        return False, None

    log_success(f"\nSuccessfully uploaded {len(artifacts)} artifact(s) to R2")
    log_info("\nRelease metadata:")
    log_info(f"  Version: {release_data['version']}")
    if platform in ("macos", "win"):
        log_info(f"  Sparkle version: {release_data.get('sparkle_version', 'N/A')}")
    log_info(f"  Artifacts: {list(release_data['artifacts'].keys())}")

    release_links = [
        (artifact["filename"], artifact["url"])
        for artifact in release_data["artifacts"].values()
    ]
    ctx.artifact_registry.add("release_links", release_links)
    ctx.artifact_registry.add("release_metadata", release_data)

    return True, release_data
