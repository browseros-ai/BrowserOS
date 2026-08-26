#!/usr/bin/env python3
"""Tests for source-backed browser resources."""

import tempfile
import unittest
from pathlib import Path
from unittest import mock

from bos_build.core.context import Context
from bos_build.release.prepared_resources import PreparedResourcesManifest
from bos_build.release.server_resources import ServerResourceResult
from bos_build.steps.resources.source import (
    PrepareCommonResourcesModule,
    PrepareServerResourcesModule,
)


SOURCE_SHA = "2" * 40


class SourceResourcesStepTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.chromium = self.root / "chromium"
        self.chromium.mkdir()
        self.prepared = self.root / "prepared"
        self.manifest = PreparedResourcesManifest(
            product="browseros",
            parent_sha="1" * 40,
            source_sha=SOURCE_SHA,
            browser_version="0.31.0",
            component_versions={
                "server": "0.0.128",
                "agent": "0.0.101.0",
                "app-onboard": "0.0.12",
            },
            files={},
        )

    def _context(self, *, prepared: bool = True) -> Context:
        return Context(
            root_dir=self.root / "packages/browseros",
            chromium_src=self.chromium,
            architecture="x64",
            build_type="release",
            resource_mode="source",
            prepared_resources=self.prepared if prepared else None,
            source_sha=SOURCE_SHA,
            chromium_version="128.0.0.0",
            browseros_build_offset="1",
            browseros_version_parts=(128, 0, 0, 1),
            browseros_chromium_version="128.0.0.1",
            semantic_version="0.31.0",
        )

    @mock.patch("bos_build.steps.resources.source.validate_prepared_resources")
    @mock.patch("bos_build.steps.resources.source.load_prepared_resources")
    def test_supplied_common_directory_is_validated_without_rebuilding(
        self, load, validate
    ) -> None:
        self.prepared.mkdir()
        load.return_value = self.manifest
        validate.return_value = self.manifest
        ctx = self._context()

        with mock.patch(
            "bos_build.steps.resources.source.prepare_common_resources"
        ) as prepare:
            PrepareCommonResourcesModule().execute(ctx)

        prepare.assert_not_called()
        validate.assert_called_once()
        self.assertEqual(ctx.artifact_registry.get("prepared_resources"), self.manifest)

    @mock.patch("bos_build.steps.resources.source.prepare_common_resources")
    @mock.patch("bos_build.steps.resources.source.read_component_version")
    def test_omitted_directory_builds_common_resources_from_checkout(
        self, read_version, prepare
    ) -> None:
        versions = {
            "server": "0.0.128",
            "agent": "0.0.101.0",
            "app-onboard": "0.0.12",
        }
        read_version.side_effect = lambda _root, component: versions[component]
        prepare.return_value = self.manifest
        ctx = self._context(prepared=False)

        with mock.patch("bos_build.steps.resources.source.LocalPreparationOperations"):
            PrepareCommonResourcesModule().execute(ctx)

        request = prepare.call_args.args[0]
        self.assertEqual(request.product, "browseros")
        self.assertEqual(request.source_sha, SOURCE_SHA)
        self.assertEqual(dict(request.component_versions), versions)
        self.assertEqual(
            request.output_dir,
            (
                self.root
                / "packages/browseros/resources/binaries/prepared_common/browseros"
                / SOURCE_SHA
                / "0.31.0"
            ).resolve(),
        )
        self.assertIsNotNone(ctx.prepared_resources)

    @mock.patch("bos_build.steps.resources.source.load_prepared_resources")
    def test_supplied_directory_must_match_product_and_source(self, load) -> None:
        self.prepared.mkdir()
        load.return_value = PreparedResourcesManifest(
            **{**self.manifest.__dict__, "product": "browserclaw"}
        )

        with self.assertRaisesRegex(ValueError, "product"):
            PrepareCommonResourcesModule().execute(self._context())

        load.return_value = PreparedResourcesManifest(
            **{**self.manifest.__dict__, "source_sha": "9" * 40}
        )
        with self.assertRaisesRegex(ValueError, "source"):
            PrepareCommonResourcesModule().execute(self._context())

    @mock.patch("bos_build.steps.resources.source.validate_prepared_resources")
    @mock.patch("bos_build.steps.resources.source.load_prepared_resources")
    @mock.patch("bos_build.steps.resources.source.ServerResourceBuilder")
    @mock.patch("bos_build.steps.resources.source.get_platform", return_value="linux")
    def test_server_step_builds_current_lane_from_common_identity(
        self, _platform, builder_class, load, validate
    ) -> None:
        self.prepared.mkdir()
        load.return_value = self.manifest
        validate.return_value = self.manifest
        destination = self.root / "server"
        builder_class.return_value.prepare.return_value = ServerResourceResult(
            product="browseros",
            target="linux-x64",
            version="0.0.128",
            source_sha=SOURCE_SHA,
            destination=destination,
            manifest_sha256="a" * 64,
        )
        ctx = self._context()

        PrepareServerResourcesModule().execute(ctx)

        builder_class.return_value.prepare.assert_called_once_with(
            product="browseros",
            target="linux-x64",
            version="0.0.128",
            source_sha=SOURCE_SHA,
        )
        self.assertEqual(
            ctx.artifact_registry.get("server_resources")["linux-x64"].destination,
            destination,
        )


if __name__ == "__main__":
    unittest.main()
