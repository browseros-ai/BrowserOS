#!/usr/bin/env python3
"""Prepare browser resources from the active source checkout."""

import re
import shutil
from pathlib import Path

from ...core.context import Context
from ...core.step import Step, ValidationError, step
from ...lib.utils import get_platform, log_info, log_success
from ...products.resource_sources import source_resources_for_product
from ...release.components import read_component_version
from ...release.extensions.crx import find_chrome_binary
from ...release.extensions.specs import spec_by_name
from ...release.extensions.workspace import require_env
from ...release.prepared_resources import (
    LocalPreparationOperations,
    PreparationRequest,
    PreparedResourcesManifest,
    load_prepared_resources,
    prepare_common_resources,
    validate_prepared_resources,
)
from ...release.server_resources import (
    ServerResourceBuilder,
    target_ids_for_lane,
)


def _repo_root(ctx: Context) -> Path:
    return ctx.root_dir.parent.parent.resolve()


def _require_source_context(ctx: Context) -> None:
    if ctx.resource_mode != "source":
        raise ValidationError("source resource steps require resource_mode=source")
    if not re.fullmatch(r"[0-9a-fA-F]{40}", ctx.source_sha):
        raise ValidationError("source resource steps require a full source SHA")


def _default_common_dir(ctx: Context) -> Path:
    return (
        ctx.root_dir
        / "resources/binaries/prepared_common"
        / ctx.product.id
        / ctx.source_sha
        / ctx.semantic_version
    )


def _request_from_manifest(
    ctx: Context,
    root: Path,
    manifest: PreparedResourcesManifest,
) -> PreparationRequest:
    if manifest.product != ctx.product.id:
        raise ValueError("Prepared-resource product does not match the build")
    if manifest.source_sha != ctx.source_sha:
        raise ValueError("Prepared-resource source SHA does not match the build")
    if manifest.browser_version != ctx.semantic_version:
        raise ValueError("Prepared-resource browser version does not match the build")
    return PreparationRequest(
        product=manifest.product,
        parent_sha=manifest.parent_sha,
        source_sha=manifest.source_sha,
        browser_version=manifest.browser_version,
        component_versions=manifest.component_versions,
        output_dir=root,
        manifest_url=ctx.get_extensions_manifest_url(),
    )


def validated_common_resources(ctx: Context) -> PreparedResourcesManifest:
    """Load and validate the common directory bound to this build."""
    _require_source_context(ctx)
    root = ctx.prepared_resources or _default_common_dir(ctx)
    manifest = load_prepared_resources(root)
    request = _request_from_manifest(ctx, root, manifest)
    validated = validate_prepared_resources(root, request)
    ctx.prepared_resources = root.resolve()
    ctx.artifact_registry.add("prepared_resources", validated)
    ctx.artifact_registry.add("prepared_resources_dir", root.resolve())
    return validated


@step("prepare_common_resources", phase="prep", optional=True)
class PrepareCommonResourcesModule(Step):
    """Build or validate platform-independent source resources."""

    produces = ["prepared_resources"]
    description = "Prepare product CRX, bug reporter CRX, and onboarding"

    def preflight(self, ctx: Context) -> None:
        _require_source_context(ctx)
        output = ctx.prepared_resources or _default_common_dir(ctx)
        if output.exists():
            validated_common_resources(ctx)
            return
        if ctx.prepared_resources_supplied:
            raise ValidationError(f"Prepared-resource directory not found: {output}")
        if shutil.which("bun") is None:
            raise ValidationError("Source common resources require Bun")
        source = source_resources_for_product(ctx.product.id)
        spec = spec_by_name(source.extension_name)
        missing = []
        for name in (spec.signing_key_env, *spec.required_env):
            try:
                require_env(name)
            except EnvironmentError:
                missing.append(name)
        if missing:
            raise ValidationError(
                "Source common resources require: " + ", ".join(missing)
            )
        try:
            find_chrome_binary()
        except RuntimeError as exc:
            raise ValidationError(str(exc)) from exc

    def validate(self, ctx: Context) -> None:
        _require_source_context(ctx)

    def execute(self, ctx: Context) -> None:
        self.validate(ctx)
        output = (ctx.prepared_resources or _default_common_dir(ctx)).resolve()
        if output.exists():
            validated_common_resources(ctx)
            log_success(f"Validated common resources: {output}")
            return
        if ctx.prepared_resources_supplied:
            raise FileNotFoundError(f"Prepared-resource directory not found: {output}")

        source = source_resources_for_product(ctx.product.id)
        versions = {
            component: read_component_version(_repo_root(ctx), component)
            for component in (
                source.server_component,
                source.extension_component,
                source.onboarding_component,
            )
        }
        request = PreparationRequest(
            product=ctx.product.id,
            parent_sha="",
            source_sha=ctx.source_sha,
            browser_version=ctx.semantic_version,
            component_versions=versions,
            output_dir=output,
            manifest_url=ctx.get_extensions_manifest_url(),
        )
        log_info(f"Preparing common resources from {ctx.source_sha}")
        manifest = prepare_common_resources(
            request,
            LocalPreparationOperations(_repo_root(ctx)),
        )
        ctx.prepared_resources = output
        ctx.artifact_registry.add("prepared_resources", manifest)
        ctx.artifact_registry.add("prepared_resources_dir", output)
        log_success(f"Prepared common resources: {output}")


@step("prepare_server_resources", phase="prep", optional=True)
class PrepareServerResourcesModule(Step):
    """Build the active lane's server resource directory."""

    produces = ["server_resources"]
    description = "Build target server resources from the active checkout"

    def preflight(self, ctx: Context) -> None:
        _require_source_context(ctx)
        builder = ServerResourceBuilder(_repo_root(ctx))
        for target in target_ids_for_lane(get_platform(), ctx.architecture):
            builder.preflight(product=ctx.product.id, target=target)

    def validate(self, ctx: Context) -> None:
        _require_source_context(ctx)
        root = ctx.prepared_resources or _default_common_dir(ctx)
        if not root.is_dir():
            raise ValidationError(f"Prepared-resource directory not found: {root}")

    def execute(self, ctx: Context) -> None:
        self.validate(ctx)
        manifest = validated_common_resources(ctx)
        source = source_resources_for_product(ctx.product.id)
        try:
            version = manifest.component_versions[source.server_component]
        except KeyError as exc:
            raise ValueError(
                f"Prepared resources omit {source.server_component} version"
            ) from exc
        builder = ServerResourceBuilder(_repo_root(ctx))
        results = {}
        for target in target_ids_for_lane(get_platform(), ctx.architecture):
            results[target] = builder.prepare(
                product=ctx.product.id,
                target=target,
                version=version,
                source_sha=ctx.source_sha,
            )
        ctx.artifact_registry.add("server_resources", results)
        log_success(f"Prepared server resources: {', '.join(results)}")
