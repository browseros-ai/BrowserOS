#!/usr/bin/env python3
"""Build, stamp, pack, and upload extension CRXs."""

import json
import os
import re
from pathlib import Path
from typing import List, Mapping, Optional, Tuple

from ...core.step import Step, ValidationError
from ...lib.paths import get_package_root
from ...lib.r2 import BOTO3_AVAILABLE, get_r2_client, upload_file_to_r2
from ...lib.utils import log_info, log_success, log_warning
from ..feeds.spec import CDN_BASE_URL, EXTENSIONS as FEED_EXTENSIONS
from .crx import find_chrome_binary, pack_crx
from .specs import (
    ExtensionSpec,
    ExternalRepoSource,
    InRepoSource,
    select_specs,
    spec_by_name,
)
from .workspace import (
    require_env,
    resolve_source,
    run_command,
    update_manifest_version,
    write_env_file,
)

# Chrome requires extension versions to be 1-4 dot-separated integers.
_VERSION_RE = re.compile(r"^\d+(\.\d+){0,3}$")
_UPDATE_FEED_EXTENSION_NAMES = frozenset(
    extension.name for extension in FEED_EXTENSIONS if extension.in_update_feed
)
_UPDATE_MANIFEST_URL = f"{CDN_BASE_URL}/extensions/update-manifest.xml"


def _validate_manifest_update_url(
    spec: ExtensionSpec, manifest: Mapping[str, object], dist_path: Path
) -> None:
    """Require update-feed extensions to declare the stable updater manifest."""
    if spec.name not in _UPDATE_FEED_EXTENSION_NAMES:
        return
    update_url = manifest.get("update_url")
    if update_url != _UPDATE_MANIFEST_URL:
        raise RuntimeError(
            f"Extension '{spec.name}' build at '{dist_path}' has update_url "
            f"{update_url!r}; expected '{_UPDATE_MANIFEST_URL}'"
        )


class ExtensionReleaseModule(Step):
    """Build, version-stamp, pack and upload the selected extension CRXs."""

    produces = []
    requires = []
    description = "Build and upload extension CRXs to the CDN"

    def __init__(
        self,
        version: str,
        names: Tuple[str, ...],
        branch_override: Optional[str] = None,
        chrome_binary: Optional[str] = None,
        monorepo_root: Optional[Path] = None,
        work_root: Optional[Path] = None,
        r2_client=None,
    ):
        self.version = version
        self.names = names
        self.branch_override = branch_override
        self.chrome_binary = chrome_binary
        self._monorepo_root = monorepo_root
        self._work_root = work_root
        self._r2_client = r2_client

    def _specs(self) -> List[ExtensionSpec]:
        return [spec_by_name(name) for name in self.names]

    def validate(self, ctx) -> None:
        try:
            specs = self._specs()
        except ValueError as e:
            raise ValidationError(str(e))

        for spec in specs:
            try:
                require_env(spec.signing_key_env)
            except EnvironmentError:
                raise ValidationError(
                    f"Signing key env var '{spec.signing_key_env}' for "
                    f"'{spec.name}' is missing or empty"
                )
            for name in spec.required_env:
                try:
                    require_env(name)
                except EnvironmentError:
                    raise ValidationError(
                        f"Required build env var '{name}' for "
                        f"'{spec.name}' is missing or empty"
                    )

        if not BOTO3_AVAILABLE:
            raise ValidationError("boto3 library not installed - run: pip install boto3")
        if not ctx.env.has_r2_config():
            raise ValidationError("R2 configuration not set")

        # Public repos clone fine without it — warn, don't fail.
        if not os.environ.get("GH_TOKEN") and any(
            isinstance(spec.source, ExternalRepoSource) for spec in specs
        ):
            log_warning(
                "GH_TOKEN is not set — cloning external extension repos "
                "will fail if any of them is private"
            )

        try:
            find_chrome_binary(self.chrome_binary)
        except RuntimeError as e:
            raise ValidationError(str(e))

    def execute(self, ctx) -> None:
        package_root = get_package_root()
        monorepo_root = self._monorepo_root or package_root.parent.parent
        work_root = self._work_root or package_root / "build" / "extensions"

        client = self._r2_client
        if client is None:
            client = get_r2_client(ctx.env)
            if client is None:
                raise RuntimeError("Failed to create R2 client")
        chrome = find_chrome_binary(self.chrome_binary)

        for spec in self._specs():
            log_info(f"\n=== Releasing extension: {spec.name} v{self.version} ===")
            source_root = resolve_source(
                spec,
                monorepo_root=monorepo_root,
                work_root=work_root,
                branch_override=self.branch_override,
            )
            update_manifest_version(
                source_root / spec.manifest_path, self.version
            )
            if isinstance(spec.source, InRepoSource):
                touched = spec.manifest_path
                if spec.env:
                    touched += f" and {spec.env_dir or '.'}/.env"
                log_warning(
                    f"stamped {touched} in the working tree — "
                    "revert them if this was a local test run"
                )
            if spec.env:
                env_dir = (
                    source_root / spec.env_dir if spec.env_dir else source_root
                )
                write_env_file(
                    env_dir,
                    spec.env,
                    required_names=spec.required_env,
                )
            if spec.pre_build:
                run_command(spec.pre_build, source_root)
            run_command(spec.build, source_root)

            dist_path = source_root / spec.dist_path
            manifest = json.loads(
                (dist_path / "manifest.json").read_text(encoding="utf-8")
            )
            _validate_manifest_update_url(spec, manifest, dist_path)
            crx_path = pack_crx(
                dist_path,
                require_env(spec.signing_key_env),
                chrome,
                work_root / "dist" / spec.crx_filename(self.version),
            )

            r2_key = spec.crx_key(self.version)
            if not upload_file_to_r2(client, crx_path, r2_key, ctx.env.r2_bucket):
                raise RuntimeError(f"Upload failed for {r2_key}")
            log_success(f"CRX live at {CDN_BASE_URL}/{r2_key}")


def build_pipeline(
    version: str,
    name: Optional[str],
    branch: Optional[str],
    chrome_binary: Optional[str],
) -> List[Step]:
    """Assemble the CRX release pipeline for one --name (or every spec)."""
    if not _VERSION_RE.fullmatch(version):
        raise ValueError(
            f"Invalid version '{version}' — Chrome extension versions are "
            "1-4 dot-separated integers (e.g. 0.0.118)"
        )
    # A '-'-prefixed value would be parsed as a git option, not a branch.
    if branch and branch.startswith("-"):
        raise ValueError(f"Invalid branch name '{branch}'")
    specs = select_specs(name)
    return [
        ExtensionReleaseModule(
            version=version,
            names=tuple(spec.name for spec in specs),
            branch_override=branch,
            chrome_binary=chrome_binary,
        )
    ]
