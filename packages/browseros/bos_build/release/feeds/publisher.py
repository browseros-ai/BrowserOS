#!/usr/bin/env python3
"""Rails-enforcing feed publisher — the only write path to live feed keys.

Every PUT is preceded by: well-formed check, spec title/link match (the
alpha→prod byte-copy killer), HEAD-200 on every referenced download, a
downgrade guard against the live object, and a feeds-history backup.
Dry-run is the default; callers must opt into writing with publish=True.
"""

import difflib
import json
import xml.etree.ElementTree as ET
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Callable, List, Optional

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
    parse_dotted_version,
)
from .spec import EXTENSIONS, FeedSpec, all_feeds, update_manifest_feed

BACKUP_PREFIX = "feeds-history"
_TIMESTAMP_FORMAT = "%Y%m%dT%H%M%SZ"


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


def _has_single_direct_channel(root: ET.Element) -> bool:
    channels = [
        element
        for element in root.iter()
        if _xml_local_name(element.tag) == "channel"
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
        element
        for element in root
        if _xml_local_name(element.tag) == "channel"
    )
    direct_items = [
        element
        for element in channel
        if _xml_local_name(element.tag) == "item"
    ]
    all_items = [
        element
        for element in root.iter()
        if _xml_local_name(element.tag) == "item"
    ]
    if len(all_items) != len(direct_items) or any(
        item not in direct_items for item in all_items
    ):
        return None

    versions: List[str] = []
    version_tag = f"{{{SPARKLE_NS}}}version"
    for item in direct_items:
        version_elements = [
            element for element in item if element.tag == version_tag
        ]
        if len(version_elements) != 1:
            return None
        value = version_elements[0].text
        if _strict_dotted_version(value) is None:
            return None
        versions.append(value.strip())
    return versions


def _is_empty_appcast_shell(content: str) -> bool:
    """Whether content is an unambiguously unpopulated RSS appcast.

    BrowserClaw's Windows feeds were provisioned before their first release
    with one whitespace-only ``<item>``.  Treat that shape, and the equivalent
    shell with no items, as first publication.  Any item attributes, text, or
    children make the live document meaningful and therefore unguardable.
    """
    try:
        root = ET.fromstring(content)
    except ET.ParseError:
        return False
    if root.tag != "rss":
        return False
    if not _has_single_direct_channel(root):
        return False
    channel = next(
        element
        for element in root
        if _xml_local_name(element.tag) == "channel"
    )
    direct_items = [
        element
        for element in channel
        if _xml_local_name(element.tag) == "item"
    ]
    all_items = [
        element
        for element in root.iter()
        if _xml_local_name(element.tag) == "item"
    ]
    if len(all_items) != len(direct_items) or any(
        item not in direct_items for item in all_items
    ):
        return False
    return all(
        not item.attrib
        and not list(item)
        and not (item.text or "").strip()
        for item in direct_items
    )


def _default_http_head(url: str) -> int:
    import requests

    try:
        return requests.head(url, timeout=30, allow_redirects=True).status_code
    except requests.RequestException as e:
        log_error(f"HEAD {url} failed: {e}")
        return 0


def _default_appcast_staging_dir() -> Path:
    return get_package_root() / "bos_build" / "config" / "appcast"


def _default_extensions_staging_dir() -> Path:
    # <monorepo>/updates/extensions — the tracked home of the extension
    # manifests (bundled_extensions_test reads it), mirroring api-worker.
    return get_package_root().parent.parent / "updates" / "extensions"


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
        appcast_staging_dir: Optional[Path] = None,
        extensions_staging_dir: Optional[Path] = None,
        now: Optional[Callable[[], datetime]] = None,
    ):
        self.env = env or EnvConfig()
        self._client = r2_client
        self.http_head = http_head or _default_http_head
        self._appcast_staging_dir = (
            appcast_staging_dir or _default_appcast_staging_dir()
        )
        self._extensions_staging_dir = (
            extensions_staging_dir or _default_extensions_staging_dir()
        )
        self._now = now or (lambda: datetime.now(timezone.utc))

    @property
    def client(self):
        if self._client is None:
            self._client = get_r2_client(self.env)
            if self._client is None:
                raise RuntimeError("Failed to create R2 client")
        return self._client

    def fetch_live(self, key: str) -> Optional[str]:
        """Live object content from R2 (authoritative — CDN caches xml 60s).

        Decodes with errors="replace": a binary/corrupt live object must
        still read as "exists" so the guard fails closed and the backup runs,
        rather than being mistaken for a first publish.
        """
        try:
            response = self.client.get_object(Bucket=self.env.r2_bucket, Key=key)
            return response["Body"].read().decode("utf-8", errors="replace")
        except self.client.exceptions.NoSuchKey:
            return None

    def staging_path(self, spec: FeedSpec) -> Path:
        basename = spec.key.rsplit("/", 1)[-1]
        if spec.kind == "extensions":
            return self._extensions_staging_dir / basename
        return self._appcast_staging_dir / basename

    def publish(
        self,
        spec: FeedSpec,
        content: str,
        publish: bool = False,
        allow_downgrade: bool = False,
        verbose: bool = True,
        stage: bool = True,
    ) -> bool:
        """Run the rails for one feed; write only when publish=True.

        verbose=False keeps the rails (and their errors) but skips the
        content/diff dump — for preflight passes that precede a real write.
        stage=False lets multi-file publishers validate every feed before
        writing any local staging files.
        """
        log_info(f"\n── {spec.key} " + "─" * max(0, 50 - len(spec.key)))

        if not spec.publishable:
            message = (
                f"{spec.key} is not publishable yet: the chromium patch for "
                f"product-aware sparkle_glue update URLs has not landed, so "
                f"no shipped {spec.product} client polls this key."
            )
            if publish:
                log_error(message)
                return False
            log_warning(f"DRY-RUN ONLY — {message}")

        if not self._check_well_formed(spec, content):
            return False
        if not self._check_channel_metadata(spec, content):
            return False

        # Every appcast item must carry exactly one strict version, even on a
        # first publish to an absent key — the guard below only runs live.
        if spec.kind in ("browser", "server"):
            new_versions = _strict_appcast_versions(content)
            if not new_versions:
                self._log_appcast_version_error(spec, content)
                return False

        if not self._check_download_urls(spec, content):
            return False

        live = self.fetch_live(spec.key)
        if live is not None and not self._check_version_guard(
            spec, content, live, allow_downgrade
        ):
            return False

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
        """Rendered/provided content must carry this spec's channel identity.

        This is the rail that makes the historical failure — an alpha file
        byte-copied onto the prod key — impossible to publish.
        """
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
            expected = update_manifest_feed(spec.channel).url
            document = json.loads(content)
            extensions = (
                document.get("extensions") if isinstance(document, dict) else None
            )
            if not isinstance(extensions, dict):
                log_error(f"{spec.key}: 'extensions' is not an object")
                return False
            urls = {
                config.get("external_update_url")
                if isinstance(config, dict)
                else None
                for config in extensions.values()
            }
            if urls - {expected}:
                log_error(
                    f"{spec.key}: external_update_url mismatch — got "
                    f"{sorted(str(u) for u in urls - {expected})}, this "
                    f"channel requires {expected}. Refusing to publish."
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
                    f"{spec.key}: download URL check failed "
                    f"(HTTP {status}): {url}"
                )
                return False
        return True

    def _check_version_guard(
        self, spec: FeedSpec, content: str, live: str, allow_downgrade: bool
    ) -> bool:
        if spec.key.endswith(".json"):
            return True

        if spec.kind in ("browser", "server"):
            return self._guard_appcast_version(spec, content, live, allow_downgrade)
        return self._guard_manifest_versions(spec, content, live, allow_downgrade)

    def _guard_appcast_version(
        self, spec: FeedSpec, content: str, live: str, allow_downgrade: bool
    ) -> bool:
        new_versions = _strict_appcast_versions(content)
        if not new_versions:
            self._log_appcast_version_error(spec, content)
            return False
        new_version = max(
            new_versions,
            key=lambda value: tuple(int(part) for part in value.split(".")),
        )

        if not self._check_well_formed(spec, live):
            return False
        try:
            live_root = ET.fromstring(live)
        except ET.ParseError:
            return False
        if not _has_single_direct_channel(live_root):
            log_error(
                f"{spec.key}: live appcast must contain exactly one direct "
                "<channel>; refusing to publish."
            )
            return False
        if not self._check_channel_metadata(spec, live):
            return False

        live_versions = _strict_appcast_versions(live)
        if not live_versions:
            if _is_empty_appcast_shell(live):
                log_info(
                    f"{spec.key}: live appcast is an empty shell "
                    "(first population)"
                )
                return True
            return self._refuse_unguardable_live(spec, allow_downgrade)
        live_version = max(
            live_versions,
            key=lambda value: tuple(int(part) for part in value.split(".")),
        )

        return self._refuse_downgrade(
            spec, f"{spec.key}", new_version, live_version, allow_downgrade
        )

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
                f"{spec.key}: new content has <item> entries but no "
                "sparkle:version"
            )

    def _guard_manifest_versions(
        self, spec: FeedSpec, content: str, live: str, allow_downgrade: bool
    ) -> bool:
        live_versions = extract_manifest_versions(live)
        if not live_versions:
            return self._refuse_unguardable_live(spec, allow_downgrade)

        new_versions = extract_manifest_versions(content)

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

    def _refuse_unguardable_live(
        self, spec: FeedSpec, allow_downgrade: bool
    ) -> bool:
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
        return max(keys)[len(prefix):]

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
