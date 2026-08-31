#!/usr/bin/env python3
"""Prepared-resource CLI tests."""

import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from typer.testing import CliRunner

from bos_build.browseros import app
from bos_build.release.candidate_test import candidate_record
from bos_build.release.prepared_resources import PreparedResourcesManifest


runner = CliRunner()


class ReleaseResourcesCliTest(unittest.TestCase):
    @patch("bos_build.cli.release_resources.read_component_version")
    @patch("bos_build.cli.release_resources.LocalPreparationOperations")
    @patch("bos_build.cli.release_resources.prepare_common_resources")
    def test_prepare_accepts_candidate_record_and_emits_manifest_digest(
        self, prepare, operations, read_version
    ) -> None:
        manifest = PreparedResourcesManifest(
            product="browseros",
            parent_sha=candidate_record().parent_sha,
            source_sha=candidate_record().candidate_sha,
            browser_version="0.31.0",
            component_versions={
                "server": "0.0.128",
                "agent": "0.0.101.0",
                "app-onboard": "0.0.0",
            },
            files={},
        )
        prepare.return_value = manifest
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            candidate_path = root / "candidate.json"
            candidate_path.write_text(candidate_record().to_json())
            output = root / "prepared"
            github_output = root / "github-output"

            result = runner.invoke(
                app,
                [
                    "release",
                    "resources",
                    "prepare",
                    "--candidate",
                    str(candidate_path),
                    "--output",
                    str(output),
                    "--repo-root",
                    str(root),
                    "--github-output",
                    str(github_output),
                ],
            )

            self.assertEqual(result.exit_code, 0, result.output)
            values = dict(
                line.split("=", 1) for line in github_output.read_text().splitlines()
            )
            self.assertEqual(values["prepared_resources"], str(output.resolve()))
            self.assertEqual(values["manifest_sha256"], manifest.digest())
            operations.assert_called_once()
            request = prepare.call_args.args[0]
            self.assertEqual(request.component_versions, manifest.component_versions)
            read_version.assert_not_called()

    def test_explicit_prepare_fills_the_products_onboarding_version(self) -> None:
        cases = {
            "browseros": "app-onboard",
            "browserclaw": "claw-onboard",
        }
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            for product, onboarding_component in cases.items():
                with (
                    self.subTest(product=product),
                    patch(
                        "bos_build.cli.release_resources.read_component_version",
                        return_value="0.0.9",
                    ) as read_version,
                    patch(
                        "bos_build.cli.release_resources.LocalPreparationOperations"
                    ),
                    patch(
                        "bos_build.cli.release_resources.prepare_common_resources"
                    ) as prepare,
                ):
                    manifest = PreparedResourcesManifest(
                        product=product,
                        parent_sha="",
                        source_sha="1" * 40,
                        browser_version="0.31.0",
                        component_versions={onboarding_component: "0.0.9"},
                        files={},
                    )
                    prepare.return_value = manifest
                    result = runner.invoke(
                        app,
                        [
                            "release",
                            "resources",
                            "prepare",
                            "--product",
                            product,
                            "--source-sha",
                            "1" * 40,
                            "--browser-version",
                            "0.31.0",
                            "--output",
                            str(root / product),
                            "--repo-root",
                            str(root),
                        ],
                    )

                    self.assertEqual(result.exit_code, 0, result.output)
                    request = prepare.call_args.args[0]
                    self.assertEqual(
                        request.component_versions[onboarding_component], "0.0.9"
                    )
                    read_version.assert_called_once_with(
                        root.resolve(), onboarding_component
                    )


if __name__ == "__main__":
    unittest.main()
