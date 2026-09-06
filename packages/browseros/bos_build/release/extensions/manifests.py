#!/usr/bin/env python3
"""Extensions module - regenerate the extension update manifests coherently.

One command owns update-manifest(.alpha).xml + extensions(.alpha).json +
bundled-manifest.xml so the three can never drift (the live alpha manifest
fell behind prod under hand-editing). CRX building is out of scope — the
crx objects must already exist in R2 (HEAD-checked before any write).
"""

from pathlib import Path
from typing import Dict, List

from ...core.context import Context
from ...core.step import Step, ValidationError
from ...lib.r2 import BOTO3_AVAILABLE
from ...lib.utils import log_info
from ..feeds.publisher import FeedPublisher
from ..feeds.render import (
    extract_manifest_versions,
    parse_dotted_version,
    render_extensions_json,
    render_update_manifest,
    strict_extension_manifest_versions,
)
from ..feeds.spec import (
    EXTENSIONS,
    bundled_manifest_feed,
    extension_by_name,
    extensions_json_feed,
    update_manifest_feed,
)

CHANNELS = ("alpha", "prod")


def parse_set_options(entries: List[str]) -> Dict[str, str]:
    """Parse repeatable --set name=version options into a dict."""
    versions: Dict[str, str] = {}
    for entry in entries:
        name, sep, version = entry.partition("=")
        if not sep or not name or not version:
            raise ValueError(f"--set expects name=version, got '{entry}'")
        versions[name] = version
    return versions


class ExtensionsFeedModule(Step):
    """Regenerate extension manifests + config for one channel"""

    produces = []
    requires = []
    description = "Generate extension update manifests"

    def __init__(
        self,
        channel: str,
        set_versions: Dict[str, str],
        publish: bool = False,
        allow_downgrade: bool = False,
        publisher=None,
        baseline_root: Path | None = None,
    ):
        self.channel = channel
        self.set_versions = set_versions
        self.publish = publish
        self.allow_downgrade = allow_downgrade
        self._publisher = publisher
        self.baseline_root = baseline_root

    def validate(self, ctx: Context) -> None:
        if not BOTO3_AVAILABLE:
            raise ValidationError(
                "boto3 library not installed - run: pip install boto3"
            )

        if not ctx.env.has_r2_config():
            raise ValidationError("R2 configuration not set")

        if self.channel not in CHANNELS:
            raise ValidationError(
                f"channel must be one of {'/'.join(CHANNELS)}, got '{self.channel}'"
            )

        for name, version in self.set_versions.items():
            try:
                extension_by_name(name)
            except ValueError as e:
                raise ValidationError(str(e))
            if not version:
                raise ValidationError(f"--set {name}= is missing a version")

    def execute(self, ctx: Context) -> None:
        publisher = self._publisher or FeedPublisher(env=ctx.env)

        versions, previous_bundled = self._resolve_versions(publisher)
        bundled_versions = self._bundled_versions(versions, previous_bundled)
        log_info(
            "Extension versions: "
            + ", ".join(f"{n}={v}" for n, v in sorted(versions.items()))
        )

        crx_targets = sorted(set(versions.items()) | set(bundled_versions.items()))
        self._check_crx_objects(publisher, crx_targets)

        update_feed_versions = {
            ext.name: versions[ext.name]
            for ext in EXTENSIONS
            if ext.in_update_feed
        }

        outputs = (
            (update_manifest_feed(self.channel),
             render_update_manifest(update_feed_versions)),
            (extensions_json_feed(self.channel),
             render_extensions_json(self.channel)),
            (bundled_manifest_feed(), render_update_manifest(bundled_versions)),
        )

        # Preflight all three through the rails before the first write so a
        # guard refusal (e.g. a typo'd --set below the live version) cannot
        # leave the trio half-updated. In dry-run mode this preflight IS
        # the run.
        for spec, content in outputs:
            if not publisher.publish(
                spec,
                content,
                publish=False,
                allow_downgrade=self.allow_downgrade,
                verbose=not self.publish,
                stage=False,
            ):
                raise RuntimeError(
                    f"Feed refused in preflight: {spec.key} — nothing written"
                )

        if not self.publish:
            for spec, content in outputs:
                staging = publisher.stage(spec, content)
                log_info(f"DRY RUN — {spec.key} staged: {staging}")
            return

        for spec, content in outputs:
            if not publisher.publish(
                spec,
                content,
                publish=True,
                allow_downgrade=self.allow_downgrade,
            ):
                raise RuntimeError(
                    f"Feed refused: {spec.key} — earlier files in this run "
                    "were already written"
                )

    def _live_versions(self, publisher: FeedPublisher, key: str) -> Dict[str, str]:
        live = publisher.fetch_live(key)
        if live is None:
            return {}
        id_to_name = {ext.extension_id: ext.name for ext in EXTENSIONS}
        return {
            id_to_name[ext_id]: version
            for ext_id, version in extract_manifest_versions(live).items()
            if ext_id in id_to_name
        }

    def _resolve_versions(self, publisher: FeedPublisher):
        """Final pins: bundled fallback < channel manifest < explicit --set.

        Each feed retains its newest live or committed entry. A sibling may
        have merged its snapshot and then failed to publish R2; using only live
        bytes would silently erase that durable progress on the next merge.
        Callers supplying a baseline must hold the publication lock from this
        read through snapshot persistence and R2 publication.
        """
        live_bundled = self._live_versions(publisher, bundled_manifest_feed().key)
        live_channel = self._live_versions(
            publisher, update_manifest_feed(self.channel).key
        )
        previous_bundled = self._with_baseline(
            live_bundled, bundled_manifest_feed().key, bundled=True
        )
        previous_channel = self._with_baseline(
            live_channel, update_manifest_feed(self.channel).key, bundled=False
        )
        versions = {**previous_bundled, **previous_channel, **self.set_versions}

        missing = [ext.name for ext in EXTENSIONS if ext.name not in versions]
        if missing:
            raise RuntimeError(
                "No live version and no --set for: "
                + ", ".join(sorted(missing))
                + f" (channel {self.channel})"
            )
        return versions, previous_bundled

    def _with_baseline(
        self, versions: Dict[str, str], key: str, *, bundled: bool
    ) -> Dict[str, str]:
        """Carry newer committed pins forward without crossing channel policy."""
        if self.baseline_root is None:
            return versions
        path = self.baseline_root / key
        committed = strict_extension_manifest_versions(
            path.read_text(encoding="utf-8"), include_all_extensions=bundled
        )
        if committed is None:
            raise RuntimeError(f"Invalid committed extension snapshot: {path}")
        resolved = dict(versions)
        for extension in EXTENSIONS:
            version = committed.get(extension.extension_id)
            current = resolved.get(extension.name)
            if version is not None and (
                current is None
                or parse_dotted_version(version) > parse_dotted_version(current)
            ):
                resolved[extension.name] = version
        return resolved

    def _bundled_versions(
        self, versions: Dict[str, str], previous_bundled: Dict[str, str]
    ) -> Dict[str, str]:
        """Bundled keeps its live/committed version when newer than this run's.

        An alpha run legitimately pushes bundled past prod's versions; a
        later prod run must not need --allow-downgrade (which would disable
        the guard on the client-facing manifest too) just to get through.
        --allow-downgrade remains the explicit path to force bundled down.
        """
        if self.allow_downgrade:
            return dict(versions)

        merged = {}
        for name, version in versions.items():
            previous = previous_bundled.get(name)
            keep_previous = previous and (
                parse_dotted_version(previous) > parse_dotted_version(version)
            )
            merged[name] = previous if keep_previous else version
        return merged

    def _check_crx_objects(self, publisher: FeedPublisher, targets) -> None:
        """Every crx referenced by any output must already exist in R2."""
        for name, version in targets:
            url = extension_by_name(name).crx_url(version)
            status = publisher.http_head(url)
            if status != 200:
                raise RuntimeError(
                    f"crx not found in R2 (HTTP {status}): {url} — upload it "
                    "before regenerating manifests"
                )
