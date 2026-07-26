#!/usr/bin/env python3
"""Appcast module - Generate complete browser appcast feed files.

Replaces the old snippet printer: every browser feed key from the FeedSpec
table is rendered as a full single-item document and pushed through the
rails publisher (dry-run by default; --publish writes to R2 with backup).
"""

from ..core.context import Context
from ..core.step import Step, ValidationError
from ..lib.utils import log_error, log_info
from ..lib.r2 import BOTO3_AVAILABLE
from .common import fetch_all_release_metadata
from .feeds.publisher import FeedPublisher
from .feeds.render import render_browser_appcast
from .feeds.spec import browser_feeds_for_product


class AppcastModule(Step):
    """Generate full Sparkle/WinSparkle appcast files for a product"""

    produces = []
    requires = []
    description = "Generate full browser appcast feed files"

    def __init__(
        self,
        product_id: str = "browseros",
        publish: bool = False,
        allow_downgrade: bool = False,
        publisher=None,
        fetch_metadata=None,
    ):
        self.product_id = product_id
        self.publish = publish
        self.allow_downgrade = allow_downgrade
        self._publisher = publisher
        self._fetch_metadata = fetch_metadata or fetch_all_release_metadata

    def validate(self, ctx: Context) -> None:
        if not BOTO3_AVAILABLE:
            raise ValidationError(
                "boto3 library not installed - run: pip install boto3"
            )

        if not ctx.env.has_r2_config():
            raise ValidationError("R2 configuration not set")

        if not ctx.release_version:
            raise ValidationError("--version is required")

        feeds = browser_feeds_for_product(self.product_id)
        if not feeds:
            raise ValidationError(
                f"No browser feeds defined for product '{self.product_id}'"
            )

        if self.publish and not all(feed.publishable for feed in feeds):
            raise ValidationError(
                f"{self.product_id} browser feeds are not publishable yet: "
                "the chromium patch for product-aware sparkle_glue update "
                "URLs has not landed. Drop --publish to preview the XML."
            )

    def execute(self, ctx: Context) -> None:
        version = ctx.release_version
        metadata = self._fetch_metadata(version, ctx.env, self.product_id)
        if not metadata:
            raise RuntimeError(
                f"No release metadata found for version {version} "
                f"(product {self.product_id})"
            )

        publisher = self._publisher or FeedPublisher(env=ctx.env)
        rendered = 0
        failures = []

        for spec in browser_feeds_for_product(self.product_id):
            release = metadata.get(spec.platform)
            if release is None:
                log_info(f"{spec.key}: no {spec.platform} release metadata — skipped")
                continue

            artifacts = release.get("artifacts", {})
            artifact_key = next(
                (key for key in spec.artifact_keys if key in artifacts), None
            )
            if artifact_key is None:
                log_info(
                    f"{spec.key}: none of {'/'.join(spec.artifact_keys)} in "
                    f"{spec.platform} artifacts — skipped"
                )
                continue

            try:
                content = render_browser_appcast(
                    spec,
                    artifacts[artifact_key],
                    version,
                    release.get("sparkle_version", ""),
                    release.get("build_date", ""),
                )
            except ValueError as e:
                log_error(str(e))
                failures.append(spec.key)
                continue

            if publisher.publish(
                spec,
                content,
                publish=self.publish,
                allow_downgrade=self.allow_downgrade,
            ):
                rendered += 1
            else:
                failures.append(spec.key)

        if failures:
            raise RuntimeError(f"Feed(s) failed: {', '.join(failures)}")
        if rendered == 0:
            raise RuntimeError(
                "No feeds rendered — release metadata has no matching artifacts"
            )
