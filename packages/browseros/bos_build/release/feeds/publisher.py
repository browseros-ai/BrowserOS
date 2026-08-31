#!/usr/bin/env python3
"""Publish update feeds through validation, backups, and downgrade guards."""

import difflib
import hashlib
import json
import re
import xml.etree.ElementTree as ET
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Callable, List, Optional, Sequence

from ...lib.env import EnvConfig
from ...lib.paths import get_package_root
from ...lib.r2 import get_r2_client
from ...lib.utils import log_error, log_info, log_success, log_warning
from .render import (
    SPARKLE_NS,
    extract_appcast_item_count,
    extract_appcast_version,
    extract_channel_metadata,
    extract_enclosure_urls,
    extract_manifest_versions,
    is_canonical_extensions_json,
    parse_dotted_version,
    strict_extension_manifest_versions,
)
from .spec import (
    EXTENSIONS,
    FeedSpec,
    all_feeds,
    feed_by_key,
    update_manifest_feed,
)

BACKUP_PREFIX = "feeds-history"
_TIMESTAMP_FORMAT = "%Y%m%dT%H%M%SZ"
_SPARKLE_VERSION_RE = re.compile(
    r"<sparkle:version\s*>\s*([0-9]+(?:\.[0-9]+)*)\s*</sparkle:version\s*>"
)
_APP_BLOCK_RE = re.compile(r"<app\b([^>]*)>(.*?)</app\s*>", re.DOTALL)
_APP_ID_RE = re.compile(r'\bappid\s*=\s*["\']([^"\']+)["\']')
_VERSION_ATTR_RE = re.compile(r'\bversion\s*=\s*["\']([0-9]+(?:\.[0-9]+)*)["\']')


def _xml_local_name(tag: str) -> str:
    return tag.rsplit("}", 1)[-1]


def _strict_dotted_version(value: Optional[str]) -> Optional[tuple[int, ...]]:
    """Parse only a nonempty sequence of numeric dotted components."""
    if value is None:
        return None
    stripped = value.strip()
    parts = stripped.split(".")
    if not stripped or any(not part.isdigit() for part in parts):
        return None
    return tuple(int(part) for part in parts)


def _content_sha256(content: str) -> str:
    return hashlib.sha256(content.encode("utf-8")).hexdigest()


def _recover_appcast_versions(content: str) -> List[str]:
    return [match.group(1) for match in _SPARKLE_VERSION_RE.finditer(content)]


def _recover_manifest_versions(content: str) -> dict[str, str]:
    versions: dict[str, str] = {}
    for block in _APP_BLOCK_RE.finditer(content):
        app_id = _APP_ID_RE.search(block.group(1))
        version = _VERSION_ATTR_RE.search(block.group(2))
        if app_id is None or version is None:
            continue
        previous = versions.get(app_id.group(1))
        if previous is not None and previous != version.group(1):
            return {}
        versions[app_id.group(1)] = version.group(1)
    return versions


def _has_single_direct_channel(root: ET.Element) -> bool:
    channels = [
        element for element in root.iter() if _xml_local_name(element.tag) == "channel"
    ]
    return (
        len(channels) == 1
        and channels[0] in list(root)
        and _xml_local_name(channels[0].tag) == "channel"
    )


def _strict_appcast_versions(content: str) -> Optional[List[str]]:
    """Return every direct item's strict Sparkle version, or None if unsafe."""
    try:
        root = ET.fromstring(content)
    except ET.ParseError:
        return None
    if root.tag != "rss" or not _has_single_direct_channel(root):
        return None

    channel = next(
        element for element in root if _xml_local_name(element.tag) == "channel"
    )
    direct_items = [
        element for element in channel if _xml_local_name(element.tag) == "item"
    ]
    all_items = [
        element for element in root.iter() if _xml_local_name(element.tag) == "item"
    ]
    if len(all_items) != len(direct_items) or any(
        item not in direct_items for item in all_items
    ):
        return None

    versions: List[str] = []
    version_tag = f"{{{SPARKLE_NS}}}version"
    for item in direct_items:
        version_elements = [element for element in item if element.tag == version_tag]
        if len(version_elements) != 1:
            return None
        value = version_elements[0].text
        if value is None or _strict_dotted_version(value) is None:
            return None
        versions.append(value.strip())
    return versions


def _has_complete_appcast_download_urls(content: str) -> bool:
    try:
        root = ET.fromstring(content)
    except ET.ParseError:
        return False
    if root.tag != "rss" or not _has_single_direct_channel(root):
        return False

    channel = next(
        element for element in root if _xml_local_name(element.tag) == "channel"
    )
    items = [element for element in channel if _xml_local_name(element.tag) == "item"]
    if not items:
        return False
    for item in items:
        enclosures = [
            element for element in item if _xml_local_name(element.tag) == "enclosure"
        ]
        if not enclosures or any(
            not (enclosure.get("url") or "").strip() for enclosure in enclosures
        ):
            return False
    return True


def _is_empty_appcast_shell(content: str) -> bool:
    """Recognize only unambiguously unpopulated RSS appcasts."""
    try:
        root = ET.fromstring(content)
    except ET.ParseError:
        return False
    if (
        root.tag != "rss"
        or root.attrib != {"version": "2.0"}
        or (root.text or "").strip()
        or (root.tail or "").strip()
    ):
        return False

    root_children = list(root)
    if len(root_children) != 1 or root_children[0].tag != "channel":
        return False
    channel = root_children[0]
    if channel.attrib or (channel.text or "").strip() or (channel.tail or "").strip():
        return False

    metadata_counts: dict[str, int] = {}
    allowed_metadata = {"title", "link", "description", "language"}
    item_count = 0
    for child in channel:
        if (child.tail or "").strip():
            return False
        if child.tag == "item":
            item_count += 1
            if (
                item_count > 1
                or child.attrib
                or list(child)
                or (child.text or "").strip()
            ):
                return False
            continue
        # Versionless placeholders have no downgrade comparison. Treat only
        # the exact RSS shell as empty; extra structure or mixed content could
        # be malformed release data outside an <item>.
        if child.tag not in allowed_metadata or child.attrib or list(child):
            return False
        metadata_counts[child.tag] = metadata_counts.get(child.tag, 0) + 1
        if metadata_counts[child.tag] > 1:
            return False
    return True


def _default_http_head(url: str) -> int:
    import requests

    try:
        return requests.head(url, timeout=30, allow_redirects=True).status_code
    except requests.RequestException as e:
        log_error(f"HEAD {url} failed: {e}")
        return 0


def _default_staging_root() -> Path:
    return get_package_root().parent.parent / "updates"


@dataclass
class FeedStatus:
    spec: FeedSpec
    live_version: Optional[str]  # None = no live object; "-" = versionless kind
    last_published: Optional[str]  # newest feeds-history backup timestamp


class FeedPublisher:
    """Publishes FeedSpec content to R2 behind the safety rails."""

    def __init__(
        self,
        env: Optional[EnvConfig] = None,
        r2_client=None,
        http_head: Optional[Callable[[str], int]] = None,
        staging_root: Optional[Path] = None,
        appcast_staging_dir: Optional[Path] = None,
        extensions_staging_dir: Optional[Path] = None,
        now: Optional[Callable[[], datetime]] = None,
    ):
        self.env = env or EnvConfig()
        self._client = r2_client
        self.http_head = http_head or _default_http_head
        root = staging_root or _default_staging_root()
        self._staging_dirs = {
            "browser": appcast_staging_dir or root / "browser",
            "server": appcast_staging_dir or root / "server",
            "extensions": extensions_staging_dir or root / "extensions",
        }
        self._now = now or (lambda: datetime.now(timezone.utc))

    @property
    def client(self):
        if self._client is None:
            self._client = get_r2_client(self.env)
            if self._client is None:
                raise RuntimeError("Failed to create R2 client")
        return self._client

    def fetch_live(self, key: str) -> Optional[str]:
        """Read authoritative live content while preserving corrupt objects."""
        try:
            response = self.client.get_object(Bucket=self.env.r2_bucket, Key=key)
            return response["Body"].read().decode("utf-8", errors="replace")
        except self.client.exceptions.NoSuchKey:
            return None

    def staging_path(self, spec: FeedSpec) -> Path:
        basename = spec.key.rsplit("/", 1)[-1]
        try:
            directory = self._staging_dirs[spec.kind]
        except KeyError as e:
            raise ValueError(f"Unknown feed kind: {spec.kind}") from e
        return directory / basename

    def _prepare_staged_batch(
        self,
        staged: List[tuple[FeedSpec, str]],
        allow_downgrade: bool,
        repair_invalid_live: bool,
    ) -> Optional[List[tuple[FeedSpec, str]]]:
        selected = {spec.key: (spec, content) for spec, content in staged}
        json_manifests = {}
        for spec, content in staged:
            if spec.kind != "extensions" or not spec.key.endswith(".json"):
                continue
            if not is_canonical_extensions_json(content, spec.channel):
                log_error(
                    f"{spec.key}: extension config must exactly match the "
                    f"registered {spec.channel} extension set"
                )
                return None
            manifest_spec = update_manifest_feed(spec.channel)
            json_manifests[spec.key] = manifest_spec.key
            selected_manifest = selected.get(manifest_spec.key)
            if selected_manifest is not None:
                continue
            live_manifest = self.fetch_live(manifest_spec.key)
            if live_manifest is None:
                log_error(
                    f"{spec.key}: matching manifest {manifest_spec.key} is "
                    "neither selected nor live"
                )
                return None
            if not self.publish(
                manifest_spec,
                live_manifest,
                allow_downgrade=allow_downgrade,
                repair_invalid_live=repair_invalid_live,
                verbose=False,
                stage=False,
            ):
                log_error(
                    f"{spec.key}: matching live manifest {manifest_spec.key} "
                    "failed validation"
                )
                return None

        ordered = []
        added = set()
        for pair in staged:
            spec, _ = pair
            manifest_key = json_manifests.get(spec.key)
            manifest_pair = selected.get(manifest_key) if manifest_key else None
            if manifest_pair is not None and manifest_key not in added:
                ordered.append(manifest_pair)
                added.add(manifest_key)
            if spec.key not in added:
                ordered.append(pair)
                added.add(spec.key)
        return ordered

    def publish_staged(
        self,
        keys: Sequence[str],
        publish: bool = False,
        allow_downgrade: bool = False,
        repair_invalid_live: bool = False,
    ) -> bool:
        """Publish selected files from the tracked updates directory."""
        unique_keys = list(dict.fromkeys(keys))
        if not unique_keys:
            log_error("Select at least one feed key")
            return False

        try:
            specs = [feed_by_key(key) for key in unique_keys]
        except ValueError as e:
            log_error(str(e))
            return False

        staged = []
        for spec in specs:
            path = self.staging_path(spec)
            try:
                content = path.read_text()
            except (OSError, UnicodeError) as e:
                log_error(f"{spec.key}: cannot read local feed {path}: {e}")
                return False
            staged.append((spec, content))

        prepared = self._prepare_staged_batch(
            staged,
            allow_downgrade,
            repair_invalid_live,
        )
        if prepared is None:
            return False
        staged = prepared

        if publish:
            for spec, content in staged:
                if not self.publish(
                    spec,
                    content,
                    allow_downgrade=allow_downgrade,
                    repair_invalid_live=repair_invalid_live,
                    verbose=False,
                    stage=False,
                ):
                    return False

        for spec, content in staged:
            if not self.publish(
                spec,
                content,
                publish=publish,
                allow_downgrade=allow_downgrade,
                repair_invalid_live=repair_invalid_live,
            ):
                return False
        return True

    def publish(
        self,
        spec: FeedSpec,
        content: str,
        publish: bool = False,
        allow_downgrade: bool = False,
        repair_invalid_live: bool = False,
        verbose: bool = True,
        stage: bool = True,
    ) -> bool:
        """Validate one feed and write only when publish is true."""
        log_info(f"\n── {spec.key} " + "─" * max(0, 50 - len(spec.key)))

        if not spec.publishable:
            message = (
                f"{spec.key} is not publishable yet: the chromium patch for "
                f"product-aware sparkle_glue update URLs has not landed, so "
                f"no shipped {spec.product} client polls this key."
            )
            if publish or repair_invalid_live:
                log_error(message)
                return False
            log_warning(f"DRY-RUN ONLY — {message}")

        if not self._check_well_formed(spec, content):
            return False
        if not self._check_channel_metadata(spec, content):
            return False

        new_empty_appcast = False
        if spec.kind in ("browser", "server"):
            new_versions = _strict_appcast_versions(content)
            new_empty_appcast = _is_empty_appcast_shell(content)
            if not new_versions and not new_empty_appcast:
                self._log_appcast_version_error(spec, content)
                return False
            if new_versions and not _has_complete_appcast_download_urls(content):
                log_error(
                    f"{spec.key}: every appcast item must have an enclosure "
                    "and every enclosure must have a download URL"
                )
                return False
        elif spec.kind == "extensions" and spec.key.endswith(".xml"):
            versions = strict_extension_manifest_versions(
                content,
                include_all_extensions=not spec.channel,
            )
            if versions is None:
                log_error(
                    f"{spec.key}: manifest must contain the exact registered "
                    "extension set with canonical versioned CRX URLs"
                )
                return False

        if not self._check_download_urls(spec, content):
            return False

        live = self.fetch_live(spec.key)
        if live is None and new_empty_appcast:
            # A placeholder may migrate metadata on an existing empty object,
            # but must not establish a versionless feed at a new CDN key.
            self._log_appcast_version_error(spec, content)
            return False
        if live is not None and not self._check_version_guard(
            spec,
            content,
            live,
            allow_downgrade,
            repair_invalid_live,
        ):
            return False

        if publish and live == content:
            staging = self.stage(spec, content)
            log_success(f"Already published {spec.url} (staging: {staging})")
            return True

        if verbose:
            self._print_content_and_diff(spec, content, live)

        if not publish:
            if stage:
                staging = self.stage(spec, content)
                if verbose:
                    log_info(
                        f"DRY RUN — {spec.key} not written "
                        f"(pass --publish to write); staged: {staging}"
                    )
            elif verbose:
                log_info(
                    f"DRY RUN — {spec.key} not written "
                    "(pass --publish to write); staging deferred"
                )
            return True

        if live is not None and not self._backup_live(spec.key):
            return False

        content_type = (
            "application/json" if spec.key.endswith(".json") else "application/xml"
        )
        self.client.put_object(
            Bucket=self.env.r2_bucket,
            Key=spec.key,
            Body=content.encode("utf-8"),
            ContentType=content_type,
        )

        staging = self.stage(spec, content)
        log_success(f"Published {spec.url} (staging: {staging})")
        return True

    def stage(self, spec: FeedSpec, content: str) -> Path:
        return self._write_staging(spec, content)

    def _write_staging(self, spec: FeedSpec, content: str) -> Path:
        staging = self.staging_path(spec)
        staging.parent.mkdir(parents=True, exist_ok=True)
        staging.write_text(content)
        return staging

    def _check_well_formed(self, spec: FeedSpec, content: str) -> bool:
        try:
            if spec.key.endswith(".json"):
                json.loads(content)
            else:
                ET.fromstring(content)
        except (json.JSONDecodeError, ET.ParseError) as e:
            log_error(f"{spec.key}: content is not well-formed: {e}")
            return False
        return True

    def _check_channel_metadata(self, spec: FeedSpec, content: str) -> bool:
        """Require content to carry the selected channel identity."""
        if spec.kind in ("browser", "server"):
            title, link = extract_channel_metadata(content)
            if title != spec.title or link != spec.link:
                log_error(
                    f"{spec.key}: channel metadata mismatch — got "
                    f"title={title!r} link={link!r}, spec requires "
                    f"title={spec.title!r} link={spec.link!r}. "
                    "Refusing to publish."
                )
                return False
            return True

        if spec.kind == "extensions" and spec.key.endswith(".json"):
            if not is_canonical_extensions_json(content, spec.channel):
                log_error(
                    f"{spec.key}: extension config must exactly match the "
                    f"registered {spec.channel} extension set"
                )
                return False

        return True

    def _check_download_urls(self, spec: FeedSpec, content: str) -> bool:
        if spec.key.endswith(".json"):
            # extensions.json only references our co-published manifest.
            return True

        for url in extract_enclosure_urls(content):
            status = self.http_head(url)
            if status != 200:
                log_error(
                    f"{spec.key}: download URL check failed (HTTP {status}): {url}"
                )
                return False
        return True

    def _check_version_guard(
        self,
        spec: FeedSpec,
        content: str,
        live: str,
        allow_downgrade: bool,
        repair_invalid_live: bool = False,
    ) -> bool:
        if spec.key.endswith(".json"):
            return True

        if spec.kind in ("browser", "server"):
            return self._guard_appcast_version(
                spec,
                content,
                live,
                allow_downgrade,
                repair_invalid_live,
            )
        return self._guard_manifest_versions(
            spec,
            content,
            live,
            allow_downgrade,
            repair_invalid_live,
        )

    def _guard_appcast_version(
        self,
        spec: FeedSpec,
        content: str,
        live: str,
        allow_downgrade: bool,
        repair_invalid_live: bool = False,
    ) -> bool:
        new_versions = _strict_appcast_versions(content)
        if not new_versions:
            if _is_empty_appcast_shell(content) and _is_empty_appcast_shell(live):
                title, link = extract_channel_metadata(live)
                if title == spec.title and link == spec.link:
                    return True
                if title in spec.legacy_titles and link == spec.link:
                    # Empty appcasts have no version to compare. Restrict this
                    # path to a registered title migration on the same key so
                    # it cannot become an escape hatch for erasing releases.
                    log_warning(
                        f"{spec.key}: migrating live channel title {title!r} "
                        f"to {spec.title!r}"
                    )
                    return True
                self._check_channel_metadata(spec, live)
                return False
            self._log_appcast_version_error(spec, content)
            return False
        new_version = max(
            new_versions,
            key=lambda value: tuple(int(part) for part in value.split(".")),
        )

        try:
            live_root = ET.fromstring(live)
        except ET.ParseError:
            if repair_invalid_live:
                return self._repair_invalid_appcast(
                    spec,
                    content,
                    live,
                    new_version,
                    "malformed live appcast XML",
                    allow_downgrade,
                )
            self._check_well_formed(spec, live)
            return False
        if not _has_single_direct_channel(live_root):
            reason = "live appcast does not have one direct channel"
            if repair_invalid_live:
                return self._repair_invalid_appcast(
                    spec,
                    content,
                    live,
                    new_version,
                    reason,
                    allow_downgrade,
                )
            log_error(
                f"{spec.key}: live appcast must contain exactly one direct "
                "<channel>; refusing to publish."
            )
            return False
        title, link = extract_channel_metadata(live)
        if title != spec.title or link != spec.link:
            if title in spec.legacy_titles and link == spec.link:
                log_warning(
                    f"{spec.key}: migrating live channel title {title!r} "
                    f"to {spec.title!r}"
                )
            else:
                reason = (
                    f"live channel metadata mismatch "
                    f"(title={title!r}, link={link!r})"
                )
                if repair_invalid_live:
                    live_versions = _strict_appcast_versions(live)
                    if not live_versions:
                        log_error(
                            f"{spec.key}: wrong-channel live appcast has no "
                            "strict version; refusing repair."
                        )
                        return False
                    return self._repair_invalid_appcast(
                        spec,
                        content,
                        live,
                        new_version,
                        reason,
                        allow_downgrade,
                        require_live_version=True,
                        known_live_versions=live_versions,
                    )
                self._check_channel_metadata(spec, live)
                return False

        live_versions = _strict_appcast_versions(live)
        if not live_versions:
            if _is_empty_appcast_shell(live):
                log_info(
                    f"{spec.key}: live appcast is an empty shell (first population)"
                )
                return True
            if repair_invalid_live:
                return self._repair_invalid_appcast(
                    spec,
                    content,
                    live,
                    new_version,
                    "live appcast has no strict version",
                    allow_downgrade,
                )
            return self._refuse_unguardable_live(spec, allow_downgrade)
        live_version = max(
            live_versions,
            key=lambda value: tuple(int(part) for part in value.split(".")),
        )

        return self._refuse_downgrade(
            spec, f"{spec.key}", new_version, live_version, allow_downgrade
        )

    def _repair_invalid_appcast(
        self,
        spec: FeedSpec,
        content: str,
        live: str,
        new_version: str,
        reason: str,
        allow_downgrade: bool,
        require_live_version: bool = False,
        known_live_versions: Optional[Sequence[str]] = None,
    ) -> bool:
        if not _has_complete_appcast_download_urls(content):
            log_error(
                f"{spec.key}: repair appcast has missing download URLs; "
                "refusing to replace invalid live content."
            )
            return False

        live_versions = (
            list(known_live_versions)
            if known_live_versions is not None
            else _recover_appcast_versions(live)
        )
        if require_live_version and not live_versions:
            log_error(
                f"{spec.key}: wrong-channel live appcast has no recoverable "
                "strict version; refusing repair."
            )
            return False
        if live_versions:
            live_version = max(
                live_versions,
                key=lambda value: tuple(int(part) for part in value.split(".")),
            )
            if not self._refuse_downgrade(
                spec,
                spec.key,
                new_version,
                live_version,
                allow_downgrade,
            ):
                return False

        self._log_repair_warning(spec, reason, live, content)
        return True

    def _log_appcast_version_error(self, spec: FeedSpec, content: str) -> None:
        if extract_appcast_item_count(content) == 0:
            log_error(f"{spec.key}: new content has no <item> entries")
            return
        version = extract_appcast_version(content)
        if version is not None:
            if _strict_dotted_version(version) is None:
                log_error(
                    f"{spec.key}: new content has invalid sparkle:version "
                    f"{version!r}; expected numeric dotted components"
                )
                return
            log_error(
                f"{spec.key}: every direct <item> must contain exactly one "
                "numeric dotted sparkle:version"
            )
        else:
            log_error(
                f"{spec.key}: new content has <item> entries but no sparkle:version"
            )

    def _guard_manifest_versions(
        self,
        spec: FeedSpec,
        content: str,
        live: str,
        allow_downgrade: bool,
        repair_invalid_live: bool = False,
    ) -> bool:
        live_versions = extract_manifest_versions(live)
        if not live_versions:
            if repair_invalid_live:
                new_versions = strict_extension_manifest_versions(
                    content,
                    include_all_extensions=not spec.channel,
                )
                if not new_versions:
                    log_error(
                        f"{spec.key}: repair manifest must contain only "
                        "complete, strictly versioned extension entries."
                    )
                    return False
                recovered = _recover_manifest_versions(live)
                if recovered and not self._guard_manifest_version_maps(
                    spec,
                    new_versions,
                    recovered,
                    allow_downgrade,
                ):
                    return False
                try:
                    ET.fromstring(live)
                    reason = "live manifest has no parseable versions"
                except ET.ParseError:
                    reason = "malformed live manifest XML"
                self._log_repair_warning(spec, reason, live, content)
                return True
            return self._refuse_unguardable_live(spec, allow_downgrade)

        new_versions = extract_manifest_versions(content)
        return self._guard_manifest_version_maps(
            spec,
            new_versions,
            live_versions,
            allow_downgrade,
        )

    def _guard_manifest_version_maps(
        self,
        spec: FeedSpec,
        new_versions: dict[str, str],
        live_versions: dict[str, str],
        allow_downgrade: bool,
    ) -> bool:
        removed = sorted(set(live_versions) - set(new_versions))
        if removed:
            if not allow_downgrade:
                log_error(
                    f"{spec.key}: live entries would be removed: "
                    f"{', '.join(removed)}. Pass --allow-downgrade to drop "
                    "extensions from the live manifest."
                )
                return False
            log_warning(
                f"{spec.key}: dropping live entries {', '.join(removed)} "
                "(--allow-downgrade)"
            )

        for ext_id, new_version in new_versions.items():
            live_version = live_versions.get(ext_id)
            if live_version is None:
                continue
            if not self._refuse_downgrade(
                spec, ext_id, new_version, live_version, allow_downgrade
            ):
                return False
        return True

    def _log_repair_warning(
        self,
        spec: FeedSpec,
        reason: str,
        live: str,
        content: str,
    ) -> None:
        log_warning(
            f"REPAIR INVALID LIVE — {spec.key}: {reason}; "
            f"old_sha256={_content_sha256(live)} "
            f"new_sha256={_content_sha256(content)}"
        )

    def _refuse_unguardable_live(self, spec: FeedSpec, allow_downgrade: bool) -> bool:
        """A live object we cannot version-compare fails closed, not open."""
        if allow_downgrade:
            log_warning(
                f"{spec.key}: live feed has no parseable version(s) — "
                "replacing it (--allow-downgrade)"
            )
            return True
        log_error(
            f"{spec.key}: live feed exists but carries no parseable "
            "version(s), so the downgrade guard cannot run. Inspect it and "
            "pass --allow-downgrade to replace it."
        )
        return False

    def _refuse_downgrade(
        self,
        spec: FeedSpec,
        subject: str,
        new_version: str,
        live_version: str,
        allow_downgrade: bool,
    ) -> bool:
        if parse_dotted_version(new_version) >= parse_dotted_version(live_version):
            return True
        if allow_downgrade:
            log_warning(
                f"{spec.key}: downgrading {subject} {live_version} -> "
                f"{new_version} (--allow-downgrade)"
            )
            return True
        log_error(
            f"{spec.key}: version downgrade refused for {subject}: live is "
            f"{live_version}, new is {new_version}. Pass --allow-downgrade "
            "to override."
        )
        return False

    def _print_content_and_diff(
        self, spec: FeedSpec, content: str, live: Optional[str]
    ) -> None:
        print(content, end="" if content.endswith("\n") else "\n")

        if live is None:
            log_info(f"{spec.key}: no live object (first publish, no backup needed)")
            return

        diff = list(
            difflib.unified_diff(
                live.splitlines(),
                content.splitlines(),
                fromfile=f"live/{spec.key}",
                tofile=f"new/{spec.key}",
                lineterm="",
            )
        )
        if diff:
            log_info(f"Diff vs live {spec.key}:")
            print("\n".join(diff))
        else:
            log_info(f"{spec.key}: identical to live feed")

    def collect_status(self) -> List[FeedStatus]:
        """Live version + last feeds-history backup for every FeedSpec key."""
        return [
            FeedStatus(
                spec=spec,
                live_version=self._live_version_display(
                    spec, self.fetch_live(spec.key)
                ),
                last_published=self._last_backup_timestamp(spec.key),
            )
            for spec in all_feeds()
        ]

    def _live_version_display(
        self, spec: FeedSpec, live: Optional[str]
    ) -> Optional[str]:
        if live is None:
            return None
        if spec.key.endswith(".json"):
            return "-"
        if spec.kind in ("browser", "server"):
            return extract_appcast_version(live) or "?"

        versions = extract_manifest_versions(live)
        if not versions:
            return "?"
        id_to_name = {ext.extension_id: ext.name for ext in EXTENSIONS}
        named = sorted(
            (id_to_name.get(ext_id, ext_id[:8]), version)
            for ext_id, version in versions.items()
        )
        return ", ".join(f"{name}={version}" for name, version in named)

    def _last_backup_timestamp(self, key: str) -> Optional[str]:
        prefix = f"{BACKUP_PREFIX}/{key}."
        keys: List[str] = []
        token = None
        while True:
            kwargs = {"Bucket": self.env.r2_bucket, "Prefix": prefix}
            if token:
                kwargs["ContinuationToken"] = token
            response = self.client.list_objects_v2(**kwargs)
            keys.extend(obj["Key"] for obj in response.get("Contents", []))
            if not response.get("IsTruncated"):
                break
            token = response.get("NextContinuationToken")

        if not keys:
            return None
        # Timestamps are fixed-width UTC, so lexical max is newest.
        return max(keys)[len(prefix) :]

    def _backup_live(self, key: str) -> bool:
        timestamp = self._now().strftime(_TIMESTAMP_FORMAT)
        backup_key = f"{BACKUP_PREFIX}/{key}.{timestamp}"
        try:
            self.client.copy_object(
                Bucket=self.env.r2_bucket,
                CopySource={"Bucket": self.env.r2_bucket, "Key": key},
                Key=backup_key,
            )
        except Exception as e:
            log_error(f"Backup failed for {key} ({e}) — refusing to overwrite")
            return False
        log_info(f"Backed up live {key} -> {backup_key}")
        return True
