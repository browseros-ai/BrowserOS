#!/usr/bin/env python3
"""Server OTA module for BrowserOS Server binary updates"""

import hashlib
import re
import shutil
import tempfile
from pathlib import Path
from typing import List, Mapping, Optional

from ...core.step import Step, ValidationError
from ...core.context import Context
from ...lib.utils import (
    log_info,
    log_success,
    log_warning,
    IS_MACOS,
    IS_WINDOWS,
)

from .common import (
    SERVER_PLATFORMS,
    SignedArtifact,
    sparkle_sign_file,
    create_server_bundle_zip,
    get_appcast_path,
    find_server_resources_dir,
    merge_base_appcast,
)
from ..feeds.publisher import FeedPublisher
from .sign_binary import (
    notarize_macos_zip,
    sign_server_bundle_macos,
    sign_server_bundle_windows,
)
from ..feeds.render import parse_server_appcast_content, render_server_appcast
from ..feeds.spec import CDN_BASE_URL, server_feed
from ...products.server_binaries import ServerBundle, server_ota_bundles_for_product
from ...lib.r2 import get_r2_client, download_file_from_r2
from ...steps.storage.download import extract_artifact_zip
from ..resource_pins import (
    COMPONENT_RESOURCE_FAMILY,
    ResourcePin,
    verify_prepared_resource_pin,
)


_SOURCE_SHA_RE = re.compile(r"[0-9a-f]{40}")
_PAYLOAD_BINDING_SCHEMA = "browseros-server-ota-v1"


def _error_response(error: Exception) -> tuple[str, Optional[int]]:
    response = getattr(error, "response", {})
    code = str(response.get("Error", {}).get("Code", ""))
    status = response.get("ResponseMetadata", {}).get("HTTPStatusCode")
    return code, status


def _is_precondition_failure(error: Exception) -> bool:
    code, status = _error_response(error)
    return code in {
        "409",
        "412",
        "ConditionalRequestConflict",
        "PreconditionFailed",
    } or status in {409, 412}


class ServerOTAModule(Step):
    """Create signed server OTA payloads from immutable release resources."""

    produces = ["server_ota_artifacts", "server_appcast"]
    requires = []
    description = "Create and upload BrowserOS Server OTA update"

    def __init__(
        self,
        version: str = "",
        channel: str = "alpha",
        platform_filter: Optional[str] = None,
        product_id: str = "browseros",
        release_sha: str = "",
    ):
        self.version = version
        self.channel = channel
        self.platform_filter = platform_filter
        self.product_id = product_id
        self.release_sha = release_sha
        self._download_dir: Optional[Path] = None

    @property
    def bundle(self) -> ServerBundle:
        bundles = server_ota_bundles_for_product(self.product_id)
        if not bundles:
            raise RuntimeError(f"Product '{self.product_id}' has no server bundle")
        return bundles[0]

    def artifact_key(self, target: str) -> str:
        """R2 source key of the unsigned server resources zip for a target."""
        return self.bundle.unsigned_artifact_key(target, version=self.version)

    def zip_filename(self, platform_name: str) -> str:
        """Sparkle payload zip name (also the enclosure URL basename)."""
        prefix = self.bundle.id.replace("-", "_")
        return f"{prefix}_{self.version}_{platform_name}.zip"

    def validate(self, context: Context) -> None:
        if not self.version:
            raise ValidationError("Version is required")

        if self.channel not in ["alpha", "prod"]:
            raise ValidationError("Channel must be 'alpha' or 'prod'")

        if not server_ota_bundles_for_product(self.product_id):
            raise ValidationError(f"Product '{self.product_id}' has no server bundle")

        if _SOURCE_SHA_RE.fullmatch(self.release_sha) is None:
            raise ValidationError("A full lowercase release SHA is required")

        if IS_MACOS():
            if not context.env.macos_certificate_name:
                raise ValidationError("MACOS_CERTIFICATE_NAME required for signing")
        elif IS_WINDOWS():
            if not context.env.code_sign_tool_path:
                raise ValidationError("CODE_SIGN_TOOL_PATH required for signing")

        if not context.env.has_r2_config():
            raise ValidationError(
                "R2 configuration not set. Required env vars: "
                "R2_ACCOUNT_ID, R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY"
            )

    def _get_platforms(self) -> List[dict]:
        """Get platforms to process based on filter (supports comma-separated)"""
        if self.platform_filter:
            requested = [p.strip() for p in self.platform_filter.split(",")]
            return [p for p in SERVER_PLATFORMS if p["name"] in requested]
        return SERVER_PLATFORMS

    def _download_artifacts(
        self,
        ctx: Context,
        download_dir: Path,
        r2_client,
        source_pin: ResourcePin,
    ) -> None:
        """Download checksum-verified immutable server resources."""
        bucket = ctx.env.r2_bucket
        platforms = self._get_platforms()
        pinned = {item.target: item for item in source_pin.objects}

        log_info("📥 Downloading server artifacts from R2...")

        for platform in platforms:
            target = platform["target"]
            resource = pinned.get(target)
            if resource is None:
                raise RuntimeError(f"Immutable source pin is missing {target}")
            r2_key = resource.key
            zip_path = download_dir / f"{target}.zip"
            extract_dir = download_dir / target

            log_info(f"  Downloading {target}...")
            if not download_file_from_r2(r2_client, r2_key, zip_path, bucket):
                raise RuntimeError(f"Failed to download artifact: {r2_key}")

            digest = hashlib.sha256()
            with zip_path.open("rb") as artifact_file:
                for chunk in iter(lambda: artifact_file.read(1024 * 1024), b""):
                    digest.update(chunk)
            if zip_path.stat().st_size != resource.size:
                raise RuntimeError(f"Immutable source size mismatch: {r2_key}")
            if digest.hexdigest() != resource.sha256:
                raise RuntimeError(f"Immutable source checksum mismatch: {r2_key}")

            extract_artifact_zip(zip_path, extract_dir)
            zip_path.unlink()

        log_success(f"Downloaded {len(platforms)} artifact(s)")

    def execute(self, context: Context) -> None:
        ctx = context
        log_info(f"\n🚀 BrowserOS Server OTA v{self.version} ({self.channel})")
        log_info("=" * 70)

        r2_client = get_r2_client(ctx.env)
        if not r2_client:
            raise RuntimeError("Failed to create R2 client")
        family_name = COMPONENT_RESOURCE_FAMILY[self.bundle.id]
        source_pin = verify_prepared_resource_pin(
            r2_client,
            ctx.env.r2_bucket,
            family_name,
            self.version,
            self.release_sha,
        )
        if self._reuse_live_release(ctx):
            return

        with (
            tempfile.TemporaryDirectory(prefix="ota_artifacts_") as dl,
            tempfile.TemporaryDirectory(prefix="ota_staging_") as st,
        ):
            binaries_dir = Path(dl)
            temp_dir = Path(st)
            log_info(f"Temp directory: {temp_dir}")

            self._download_artifacts(ctx, binaries_dir, r2_client, source_pin)
            signed_artifacts = self._build_platform_artifacts(
                ctx, binaries_dir, temp_dir
            )
            self._finalize_release(ctx, signed_artifacts)

    def _reuse_live_release(self, ctx: Context) -> bool:
        """Stage an already-live same-version release without replacing payloads."""
        spec = server_feed(self.bundle.id, self.channel)
        live = FeedPublisher(env=ctx.env).fetch_live(spec.key)
        if live is None:
            return False
        existing = parse_server_appcast_content(live)
        if existing is None or existing.version != self.version:
            return False

        requested = {platform["name"] for platform in self._get_platforms()}
        missing = sorted(requested.difference(existing.artifacts))
        if missing:
            raise RuntimeError(
                f"Live {spec.key} is missing same-version payloads: "
                f"{', '.join(missing)}"
            )

        appcast_path = get_appcast_path(self.channel, self.bundle.id)
        appcast_path.parent.mkdir(parents=True, exist_ok=True)
        appcast_path.write_text(live)
        artifacts = [existing.artifacts[name] for name in sorted(requested)]
        ctx.artifact_registry.add("server_ota_artifacts", artifacts)
        ctx.artifact_registry.add("server_appcast", appcast_path)
        log_success(
            f"Reused live server OTA v{self.version} without replacing payloads"
        )
        return True

    def _build_platform_artifacts(
        self, ctx: Context, binaries_dir: Path, temp_dir: Path
    ) -> List[SignedArtifact]:
        """Sign, archive, and Sparkle-sign each selected platform."""
        signed_artifacts: List[SignedArtifact] = []

        for platform in self._get_platforms():
            log_info(f"\n📦 Processing {platform['name']}...")

            source_resources = find_server_resources_dir(binaries_dir, platform)
            if not source_resources:
                raise RuntimeError(f"Resources dir not found for {platform['name']}")

            staging_resources = temp_dir / platform["name"] / "resources"
            shutil.copytree(source_resources, staging_resources)

            if not self._sign_bundle(staging_resources, platform, ctx):
                raise RuntimeError(f"Signing failed for {platform['name']}")

            zip_name = self.zip_filename(platform["name"])
            zip_path = temp_dir / zip_name

            if not create_server_bundle_zip(staging_resources, zip_path):
                raise RuntimeError(f"Failed to create bundle for {platform['name']}")

            if platform["os"] == "macos" and IS_MACOS():
                if not notarize_macos_zip(zip_path, ctx.env):
                    raise RuntimeError(f"Notarization failed for {platform['name']}")

            log_info(f"Signing {zip_name} with Sparkle...")
            signature, length = sparkle_sign_file(zip_path, ctx.env)
            if not signature:
                raise RuntimeError(f"Sparkle signing failed for {platform['name']}")

            log_success(f"  {platform['name']}: {length} bytes")
            signed_artifacts.append(
                SignedArtifact(
                    platform=platform["name"],
                    zip_path=zip_path,
                    signature=signature,
                    length=length,
                    os=platform["os"],
                    arch=platform["arch"],
                )
            )

        if not signed_artifacts:
            raise RuntimeError("OTA failed - no artifacts processed")
        return signed_artifacts

    def _finalize_release(
        self, ctx: Context, signed_artifacts: List[SignedArtifact]
    ) -> None:
        """Write the appcast, upload every signed zip to R2, and surface URLs."""
        log_info("\n📤 Uploading artifacts to R2...")
        r2_client = get_r2_client(ctx.env)
        if not r2_client:
            raise RuntimeError("Failed to create R2 client")
        canonical_artifacts = [
            self._upload_bound_payload(ctx, r2_client, artifact)
            for artifact in signed_artifacts
        ]

        log_info("\n📝 Generating appcast...")
        spec = server_feed(self.bundle.id, self.channel)
        appcast_path = get_appcast_path(self.channel, self.bundle.id)
        existing_appcast = merge_base_appcast(
            FeedPublisher(env=ctx.env), spec, appcast_path
        )

        appcast_content = render_server_appcast(
            spec,
            self.version,
            canonical_artifacts,
            existing=existing_appcast,
        )
        appcast_path.parent.mkdir(parents=True, exist_ok=True)
        appcast_path.write_text(appcast_content)
        log_success(f"Appcast saved to: {appcast_path}")

        ctx.artifact_registry.add("server_ota_artifacts", canonical_artifacts)
        ctx.artifact_registry.add("server_appcast", appcast_path)

        log_info("\n" + "=" * 70)
        log_success(f"✅ Server OTA v{self.version} ({self.channel}) artifacts ready!")
        log_info("=" * 70)

        log_info("\nArtifact URLs:")
        for artifact in canonical_artifacts:
            log_info(f"  {CDN_BASE_URL}/server/{artifact.zip_path.name}")

        log_info(f"\nAppcast saved to: {appcast_path}")
        log_info(
            "\n📋 Next step: Run 'browseros ota server release-appcast "
            f"--channel {self.channel} --publish' to make the release live"
        )

    def _upload_bound_payload(
        self,
        ctx: Context,
        r2_client,
        artifact: SignedArtifact,
    ) -> SignedArtifact:
        """Create one immutable signed payload or reuse its canonical object."""
        data = artifact.zip_path.read_bytes()
        key = f"server/{artifact.zip_path.name}"
        if artifact.length != len(data):
            raise RuntimeError(f"Signed payload length mismatch: {key}")
        metadata = {
            "binding-schema": _PAYLOAD_BINDING_SCHEMA,
            "bundle-id": self.bundle.id,
            "version": self.version,
            "release-sha": self.release_sha,
            "platform": artifact.platform,
            "os": artifact.os,
            "arch": artifact.arch,
            "sha256": hashlib.sha256(data).hexdigest(),
            "sparkle-signature": artifact.signature,
            "length": str(len(data)),
        }
        try:
            r2_client.put_object(
                Bucket=ctx.env.r2_bucket,
                Key=key,
                Body=data,
                ContentType="application/zip",
                Metadata=metadata,
                IfNoneMatch="*",
            )
        except Exception as error:
            if not _is_precondition_failure(error):
                raise RuntimeError(
                    f"Failed to create source-bound server payload {key}: {error}"
                ) from error
            return self._read_bound_payload(
                ctx,
                r2_client,
                key,
                artifact.platform,
                artifact.os,
                artifact.arch,
            )
        log_success(f"Uploaded source-bound server payload: {key}")
        return artifact

    def _read_bound_payload(
        self,
        ctx: Context,
        r2_client,
        key: str,
        platform: str,
        os_type: str,
        arch: str,
    ) -> SignedArtifact:
        """Read and validate a canonical signed payload after a write race."""
        try:
            response = r2_client.get_object(Bucket=ctx.env.r2_bucket, Key=key)
        except Exception as error:
            raise RuntimeError(
                f"Failed to read canonical server payload {key}: {error}"
            ) from error

        body = response.get("Body")
        if body is None or not hasattr(body, "read"):
            raise RuntimeError(f"Canonical server payload {key} has no readable body")
        try:
            data = body.read()
        finally:
            close = getattr(body, "close", None)
            if close:
                close()

        metadata = response.get("Metadata")
        if not isinstance(metadata, Mapping):
            metadata = {}
        actual_sha256 = hashlib.sha256(data).hexdigest()
        expected = {
            "binding-schema": _PAYLOAD_BINDING_SCHEMA,
            "bundle-id": self.bundle.id,
            "version": self.version,
            "release-sha": self.release_sha,
            "platform": platform,
            "os": os_type,
            "arch": arch,
            "sha256": actual_sha256,
            "length": str(len(data)),
        }
        mismatches = {
            name: {"expected": value, "actual": metadata.get(name)}
            for name, value in expected.items()
            if metadata.get(name) != value
        }
        signature = metadata.get("sparkle-signature")
        if not isinstance(signature, str) or not signature:
            mismatches["sparkle-signature"] = {
                "expected": "non-empty",
                "actual": signature,
            }
        if mismatches:
            raise RuntimeError(
                f"Canonical server payload binding mismatch for {key}: {mismatches}"
            )
        assert isinstance(signature, str)

        log_success(f"Reused source-bound server payload: {key}")
        return SignedArtifact(
            platform=platform,
            zip_path=Path(key.rsplit("/", 1)[-1]),
            signature=signature,
            length=len(data),
            os=os_type,
            arch=arch,
        )

    def _sign_bundle(
        self, staging_resources: Path, platform: dict, ctx: Context
    ) -> bool:
        """Codesign every binary in the staged resources tree."""
        os_type = platform["os"]

        if os_type == "macos":
            if not IS_MACOS():
                log_warning(
                    f"macOS signing requires macOS - leaving {platform['name']} unsigned"
                )
                return True
            return sign_server_bundle_macos(
                staging_resources, ctx.env, ctx.get_entitlements_dir()
            )

        if os_type == "windows":
            return sign_server_bundle_windows(staging_resources, ctx.env, self.bundle)

        log_info("No code signing for Linux binaries")
        return True
