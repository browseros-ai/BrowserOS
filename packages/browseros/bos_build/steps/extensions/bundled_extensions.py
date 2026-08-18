#!/usr/bin/env python3
"""Stage bundled extension CRXs from published or prepared resources."""

import json
import shutil
import sys
import xml.etree.ElementTree as ET
from pathlib import Path
from typing import Dict, List, NamedTuple

import requests

from ...core.context import Context
from ...core.step import Step, ValidationError, step
from ...lib.utils import log_info, log_success
from ...products.resource_sources import source_resources_for_product
from ...release.components import normalize_component_version
from ...release.feeds.spec import extension_by_name
from ...release.prepared_resources import PreparedResourcesManifest
from ..resources.source import validated_common_resources


class ExtensionInfo(NamedTuple):
    """One extension selected for browser staging."""

    id: str
    version: str
    codebase: str


@step("bundled_extensions", phase="prep")
class BundledExtensionsModule(Step):
    """Stage product-required CRXs and their Chromium manifest."""

    produces = ["bundled_extensions"]
    requires = []
    description = "Stage bundled extension CRXs"

    def validate(self, ctx: Context) -> None:
        if not ctx.chromium_src or not ctx.chromium_src.exists():
            raise ValidationError(
                f"Chromium source directory not found: {ctx.chromium_src}"
            )

    def execute(self, ctx: Context) -> None:
        self.validate(ctx)
        if ctx.resource_mode == "source":
            manifest = validated_common_resources(ctx)
            staged = self._prepared_extensions(ctx, manifest)
            ctx.artifact_registry.add("common_manifest_digest", manifest.digest())
            source_label = "prepared common resources"
        else:
            extensions = self._fetch_and_parse_manifest(
                ctx.get_extensions_manifest_url()
            )
            if not extensions:
                raise RuntimeError("No extensions found in manifest")
            staged = self._select_product_extensions(extensions, ctx)
            source_label = "CDN manifest"

        output_dir = self._get_output_dir(ctx)
        output_dir.mkdir(parents=True, exist_ok=True)
        self._clear_generated_outputs(ctx, output_dir)
        log_info(f"\n📦 Bundling extensions from {source_label}...")
        log_info(f"  Output: {output_dir}")
        if ctx.resource_mode == "source":
            self._copy_prepared_extensions(staged, ctx, output_dir, manifest)
        else:
            for extension in staged:
                self._download_extension(extension, output_dir)
        self._generate_json(staged, output_dir)
        log_success(f"Bundled {len(staged)} extensions successfully")

    def _prepared_extensions(
        self, ctx: Context, manifest: PreparedResourcesManifest
    ) -> List[ExtensionInfo]:
        files = [manifest.files[role] for role in ("product_crx", "bug_reporter_crx")]
        required_ids = {extension_id for extension_id, _ in ctx.required_extension_ids}
        actual_ids = {prepared.extension_id for prepared in files}
        if actual_ids != required_ids:
            raise ValueError("Prepared-resource extensions do not match the product")
        return [
            ExtensionInfo(
                id=prepared.extension_id,
                version=prepared.version,
                codebase=f"prepared://{prepared.path}",
            )
            for prepared in files
        ]

    def _copy_prepared_extensions(
        self,
        extensions: List[ExtensionInfo],
        ctx: Context,
        output_dir: Path,
        manifest: PreparedResourcesManifest,
    ) -> None:
        root = ctx.prepared_resources
        if root is None:
            raise RuntimeError("Prepared resources were not registered")
        by_id = {
            prepared.extension_id: prepared
            for role, prepared in manifest.files.items()
            if role.endswith("crx")
        }
        for extension in extensions:
            prepared = by_id[extension.id]
            shutil.copy2(
                root / prepared.path,
                output_dir / f"{extension.id}.crx",
            )

    def _get_output_dir(self, ctx: Context) -> Path:
        return ctx.chromium_src / "chrome/browser/browseros/bundled_extensions"

    def _clear_generated_outputs(self, ctx: Context, output_dir: Path) -> None:
        for crx_path in output_dir.glob("*.crx"):
            crx_path.unlink()
        (output_dir / "bundled_extensions.json").unlink(missing_ok=True)
        generated = ctx.chromium_src / ctx.out_dir / "browseros_extensions"
        if generated.is_symlink() or generated.is_file():
            generated.unlink()
        elif generated.exists():
            shutil.rmtree(generated)

    def _fetch_and_parse_manifest(self, url: str) -> List[ExtensionInfo]:
        log_info(f"  Fetching manifest: {url}")
        if "://" not in url:
            try:
                return self._parse_manifest_xml(Path(url).read_text(encoding="utf-8"))
            except OSError as exc:
                raise RuntimeError(f"Failed to read extension manifest: {exc}") from exc
        try:
            response = requests.get(url, timeout=30)
            response.raise_for_status()
        except requests.RequestException as exc:
            raise RuntimeError(f"Failed to fetch manifest: {exc}") from exc
        return self._parse_manifest_xml(response.text)

    def _parse_manifest_xml(self, xml_content: str) -> List[ExtensionInfo]:
        try:
            root = ET.fromstring(xml_content)
        except ET.ParseError as exc:
            raise RuntimeError(f"Failed to parse manifest XML: {exc}") from exc
        namespace = {"gupdate": "http://www.google.com/update2/response"}
        apps = root.findall(".//gupdate:app", namespace) or root.findall(".//app")
        extensions = []
        for app in apps:
            app_id = app.get("appid")
            if not app_id:
                continue
            update = app.find("gupdate:updatecheck", namespace)
            if update is None:
                update = app.find("updatecheck")
            if update is None:
                continue
            version = update.get("version")
            codebase = update.get("codebase")
            if version and codebase:
                extensions.append(ExtensionInfo(app_id, version, codebase))
        return extensions

    def _select_product_extensions(
        self, extensions: List[ExtensionInfo], ctx: Context
    ) -> List[ExtensionInfo]:
        by_id = {extension.id: extension for extension in extensions}
        pinned_version = ctx.env.bundled_product_extension_version
        if pinned_version:
            source = source_resources_for_product(ctx.product.id)
            version = normalize_component_version(
                source.extension_component, pinned_version
            )
            spec = extension_by_name(source.extension_name)
            by_id[spec.extension_id] = ExtensionInfo(
                spec.extension_id,
                version,
                spec.crx_url(version),
            )
        missing = [
            f"{name} ({extension_id})"
            for extension_id, name in ctx.required_extension_ids
            if extension_id not in by_id
        ]
        if missing:
            raise RuntimeError(
                f"Bundled extension manifest for {ctx.product.display_name} "
                "missing required entries: " + ", ".join(missing)
            )
        return [by_id[extension_id] for extension_id, _ in ctx.required_extension_ids]

    def _download_extension(self, extension: ExtensionInfo, output_dir: Path) -> None:
        destination = output_dir / f"{extension.id}.crx"
        log_info(f"  Downloading {extension.id} v{extension.version}...")
        try:
            response = requests.get(extension.codebase, stream=True, timeout=60)
            response.raise_for_status()
            total = int(response.headers.get("content-length", 0))
            downloaded = 0
            with open(destination, "wb") as stream:
                for chunk in response.iter_content(chunk_size=65536):
                    stream.write(chunk)
                    downloaded += len(chunk)
                    if total:
                        sys.stdout.write(
                            f"\r    {destination.name}: {downloaded / total * 100:.0f}%  "
                        )
                        sys.stdout.flush()
            sys.stdout.write(f"\r    {destination.name}: done\n")
            sys.stdout.flush()
        except requests.RequestException as exc:
            raise RuntimeError(f"Failed to download {extension.id}: {exc}") from exc

    def _generate_json(self, extensions: List[ExtensionInfo], output_dir: Path) -> None:
        data: Dict[str, Dict[str, str]] = {
            extension.id: {
                "external_crx": f"{extension.id}.crx",
                "external_version": extension.version,
            }
            for extension in extensions
        }
        path = output_dir / "bundled_extensions.json"
        path.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
        log_info(f"  Generated {path.name}")
