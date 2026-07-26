#!/usr/bin/env python3
"""Bundled Extensions Module - stage bundled extension CRXs.

Default release builds download every required CRX from the CDN bundled
manifest. Nightly profiles can opt into `bundle_local_extensions: true`; in
that mode in-repo extension specs are built and packed from the local checkout,
while required external extensions continue to download from the CDN manifest.
"""

import json
import shutil
import sys
import xml.etree.ElementTree as ET
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, List, NamedTuple, Union

import requests

from ...core.context import Context
from ...core.step import Step, ValidationError, step
from ...lib.utils import log_info, log_success
from ...release.extensions.crx import find_chrome_binary, pack_crx
from ...release.extensions.specs import (
    EXTENSION_SPECS,
    ExtensionSpec,
    InRepoSource,
)
from ...release.extensions.workspace import (
    require_env,
    resolve_source,
    run_command,
    write_env_file,
)


class ExtensionInfo(NamedTuple):
    """Extension metadata parsed from update manifest"""

    id: str
    version: str
    codebase: str


@dataclass(frozen=True)
class ManifestBundle:
    """A required extension staged by downloading the CDN-pinned CRX."""

    name: str
    extension: ExtensionInfo


@dataclass(frozen=True)
class LocalBundle:
    """A required extension staged by building an in-repo extension spec."""

    name: str
    spec: ExtensionSpec


BundlePlan = Union[ManifestBundle, LocalBundle]


@step("bundled_extensions", phase="prep")
class BundledExtensionsModule(Step):
    """Stage bundled CRXs and create bundled_extensions.json.

    Profile switch: `bundle_local_extensions: true` builds in-repo required
    extensions (agent/browserclaw) from the local checkout and packs them with
    their canonical signing key env vars. Required external extensions still
    come from the bundled CDN manifest.
    """

    produces = ["bundled_extensions"]
    requires = []
    description = "Stage bundled extension CRXs from CDN or local builds"

    def preflight(self, ctx: Context) -> None:
        if self._use_local_bundles(ctx):
            self._validate_local_bundle_requirements(ctx)

    def validate(self, ctx: Context) -> None:
        if not ctx.chromium_src or not ctx.chromium_src.exists():
            raise ValidationError(
                f"Chromium source directory not found: {ctx.chromium_src}"
            )
        if self._use_local_bundles(ctx):
            self._validate_local_bundle_requirements(ctx)

    def execute(self, ctx: Context) -> None:
        if self._use_local_bundles(ctx):
            log_info("\n📦 Bundling extensions from local builds + CDN manifest...")
        else:
            log_info("\n📦 Bundling extensions from CDN manifest...")

        manifest_url = ctx.get_extensions_manifest_url()
        output_dir = self._get_output_dir(ctx)

        output_dir.mkdir(parents=True, exist_ok=True)
        self._clear_generated_outputs(ctx, output_dir)
        log_info(f"  Output: {output_dir}")

        extensions = self._fetch_and_parse_manifest(manifest_url)
        if not extensions:
            raise RuntimeError("No extensions found in manifest")

        if self._use_local_bundles(ctx):
            planned = self._plan_hybrid_bundles(extensions, ctx)
            log_info(
                f"  Selected {len(planned)} extension(s) for {ctx.product.display_name}"
            )
            staged = self._stage_hybrid_bundles(planned, ctx, output_dir)
        else:
            staged = self._select_product_extensions(extensions, ctx)
            log_info(
                f"  Selected {len(staged)} extension(s) for {ctx.product.display_name}"
            )

            for ext in staged:
                self._download_extension(ext, output_dir)

        self._generate_json(staged, output_dir)

        log_success(f"Bundled {len(staged)} extensions successfully")

    def _get_output_dir(self, ctx: Context) -> Path:
        """Get the bundled extensions output directory in Chromium source"""
        return (
            ctx.chromium_src / "chrome" / "browser" / "browseros" / "bundled_extensions"
        )

    def _clear_generated_outputs(self, ctx: Context, output_dir: Path) -> None:
        """Remove generated extension payloads without touching source files."""
        for crx_path in output_dir.glob("*.crx"):
            crx_path.unlink()
        (output_dir / "bundled_extensions.json").unlink(missing_ok=True)

        generated_output_dir = ctx.chromium_src / ctx.out_dir / "browseros_extensions"
        if generated_output_dir.is_symlink() or generated_output_dir.is_file():
            generated_output_dir.unlink()
        elif generated_output_dir.exists():
            shutil.rmtree(generated_output_dir)

    def _fetch_and_parse_manifest(self, url: str) -> List[ExtensionInfo]:
        """Fetch XML manifest and parse extension information"""
        log_info(f"  Fetching manifest: {url}")

        try:
            response = requests.get(url, timeout=30)
            response.raise_for_status()
        except requests.RequestException as e:
            raise RuntimeError(f"Failed to fetch manifest: {e}")

        return self._parse_manifest_xml(response.text)

    def _parse_manifest_xml(self, xml_content: str) -> List[ExtensionInfo]:
        """Parse Google Update protocol XML manifest."""
        extensions = []

        try:
            root = ET.fromstring(xml_content)
        except ET.ParseError as e:
            raise RuntimeError(f"Failed to parse manifest XML: {e}")

        ns = {"gupdate": "http://www.google.com/update2/response"}

        # Try with namespace first, then without (for flexibility)
        apps = root.findall(".//gupdate:app", ns)
        if not apps:
            apps = root.findall(".//app")

        for app in apps:
            app_id = app.get("appid")
            if not app_id:
                continue

            updatecheck = app.find("gupdate:updatecheck", ns)
            if updatecheck is None:
                updatecheck = app.find("updatecheck")
            if updatecheck is None:
                continue

            version = updatecheck.get("version")
            codebase = updatecheck.get("codebase")

            if version and codebase:
                extensions.append(
                    ExtensionInfo(
                        id=app_id,
                        version=version,
                        codebase=codebase,
                    )
                )

        return extensions

    def _select_product_extensions(
        self, extensions: List[ExtensionInfo], ctx: Context
    ) -> List[ExtensionInfo]:
        """Return manifest entries required by the current build."""
        self._validate_required_extensions(extensions, ctx)
        required_ids = {extension_id for extension_id, _ in ctx.required_extension_ids}
        return [ext for ext in extensions if ext.id in required_ids]

    def _validate_required_extensions(
        self, extensions: List[ExtensionInfo], ctx: Context
    ) -> None:
        """Fail if the manifest omits a required bundled extension."""
        extension_ids = {ext.id for ext in extensions}
        missing = [
            f"{name} ({extension_id})"
            for extension_id, name in ctx.required_extension_ids
            if extension_id not in extension_ids
        ]
        if missing:
            raise RuntimeError(
                f"Bundled extension manifest for {ctx.product.display_name} "
                "missing required entries: " + ", ".join(missing)
            )

    def _use_local_bundles(self, ctx: Context) -> bool:
        return bool(getattr(ctx, "bundle_local_extensions", False))

    def _plan_hybrid_bundles(
        self, extensions: List[ExtensionInfo], ctx: Context
    ) -> List[BundlePlan]:
        """Choose local-build or manifest-download source per required ID."""
        manifest_by_id = {ext.id: ext for ext in extensions}
        local_specs_by_id = self._local_specs_by_id()
        planned: List[BundlePlan] = []
        missing: List[str] = []

        for extension_id, name in ctx.required_extension_ids:
            spec = local_specs_by_id.get(extension_id)
            if spec is not None:
                planned.append(LocalBundle(name=name, spec=spec))
                continue

            ext = manifest_by_id.get(extension_id)
            if ext is not None:
                planned.append(ManifestBundle(name=name, extension=ext))
                continue

            missing.append(f"{name} ({extension_id})")

        if missing:
            raise RuntimeError(
                f"Bundled extensions for {ctx.product.display_name} missing required "
                "local or manifest entries: " + ", ".join(missing)
            )
        return planned

    def _local_specs_by_id(self) -> Dict[str, ExtensionSpec]:
        """Specs buildable from this repository, keyed by canonical CRX ID."""
        return {
            spec.extension_id: spec
            for spec in EXTENSION_SPECS
            if isinstance(spec.source, InRepoSource)
        }

    def _stage_hybrid_bundles(
        self, planned: List[BundlePlan], ctx: Context, output_dir: Path
    ) -> List[ExtensionInfo]:
        chrome = (
            find_chrome_binary()
            if any(isinstance(item, LocalBundle) for item in planned)
            else ""
        )
        staged: List[ExtensionInfo] = []
        for item in planned:
            if isinstance(item, ManifestBundle):
                self._download_extension(item.extension, output_dir)
                staged.append(item.extension)
            else:
                staged.append(
                    self._build_and_pack_local_extension(
                        item.spec, ctx, output_dir, chrome
                    )
                )
        return staged

    def _build_and_pack_local_extension(
        self,
        spec: ExtensionSpec,
        ctx: Context,
        output_dir: Path,
        chrome_binary: str,
    ) -> ExtensionInfo:
        """Build an in-repo extension spec and pack it as <extension id>.crx."""
        package_root = ctx.root_dir
        monorepo_root = package_root.parent.parent
        work_root = package_root / "build" / "bundled_extensions"

        log_info(f"  Building local extension {spec.name}...")
        source_root = resolve_source(
            spec,
            monorepo_root=monorepo_root,
            work_root=work_root,
            branch_override=None,
        )
        version = self._read_manifest_version(source_root / spec.manifest_path)
        if spec.env:
            env_dir = source_root / spec.env_dir if spec.env_dir else source_root
            write_env_file(
                env_dir,
                spec.env,
                required_names=spec.required_env,
            )
        if spec.pre_build:
            run_command(spec.pre_build, source_root)
        run_command(spec.build, source_root)

        pack_crx(
            source_root / spec.dist_path,
            self._require_signing_key(spec),
            chrome_binary,
            output_dir / f"{spec.extension_id}.crx",
        )
        return ExtensionInfo(
            id=spec.extension_id,
            version=version,
            codebase=f"local://{spec.name}",
        )

    def _read_manifest_version(self, manifest_path: Path) -> str:
        if not manifest_path.exists():
            raise FileNotFoundError(f"Extension manifest not found: {manifest_path}")
        version = json.loads(manifest_path.read_text()).get("version")
        if not isinstance(version, str) or not version:
            raise RuntimeError(f"Extension manifest missing version: {manifest_path}")
        return version

    def _require_signing_key(self, spec: ExtensionSpec) -> str:
        return require_env(spec.signing_key_env)

    def _validate_local_bundle_requirements(self, ctx: Context) -> None:
        missing = []
        local_specs_by_id = self._local_specs_by_id()
        has_local_required = False
        for extension_id, name in ctx.required_extension_ids:
            spec = local_specs_by_id.get(extension_id)
            if spec is None:
                continue
            has_local_required = True
            for env_name in (spec.signing_key_env, *spec.required_env):
                try:
                    require_env(env_name)
                except EnvironmentError:
                    missing.append(f"{name}: {env_name}")
        if missing:
            raise ValidationError(
                "Local bundled extension env var(s) missing: "
                + ", ".join(missing)
            )
        if not has_local_required:
            return
        try:
            find_chrome_binary()
        except RuntimeError as e:
            raise ValidationError(str(e))

    def _download_extension(self, ext: ExtensionInfo, output_dir: Path) -> None:
        """Download a single extension .crx file"""
        dest_filename = f"{ext.id}.crx"
        dest_path = output_dir / dest_filename

        log_info(f"  Downloading {ext.id} v{ext.version}...")

        try:
            response = requests.get(ext.codebase, stream=True, timeout=60)
            response.raise_for_status()

            total_size = int(response.headers.get("content-length", 0))
            downloaded = 0

            with open(dest_path, "wb") as f:
                for chunk in response.iter_content(chunk_size=65536):
                    f.write(chunk)
                    downloaded += len(chunk)
                    if total_size:
                        percent = downloaded / total_size * 100
                        sys.stdout.write(f"\r    {dest_filename}: {percent:.0f}%  ")
                        sys.stdout.flush()

            if total_size:
                sys.stdout.write(
                    f"\r    {dest_filename}: done ({total_size / 1024:.0f} KB)\n"
                )
            else:
                sys.stdout.write(f"\r    {dest_filename}: done\n")
            sys.stdout.flush()

        except requests.RequestException as e:
            raise RuntimeError(f"Failed to download {ext.id}: {e}")

    def _generate_json(self, extensions: List[ExtensionInfo], output_dir: Path) -> None:
        """Generate bundled_extensions.json"""
        json_path = output_dir / "bundled_extensions.json"

        data: Dict[str, Dict[str, str]] = {}
        for ext in extensions:
            data[ext.id] = {
                "external_crx": f"{ext.id}.crx",
                "external_version": ext.version,
            }

        with open(json_path, "w") as f:
            json.dump(data, f, indent=2)
            f.write("\n")

        log_info(f"  Generated {json_path.name}")
