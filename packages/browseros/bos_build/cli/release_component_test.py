#!/usr/bin/env python3
"""CLI tests for standalone component release commands."""

import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from typer.testing import CliRunner

from bos_build.browseros import app
from bos_build.cli.release_component import _manifest_allocations
from bos_build.release.component_release import StandaloneReleaseRecord
from bos_build.release.extensions.manifests import render_update_manifest


runner = CliRunner()


class ComponentReleaseCliTest(unittest.TestCase):
    def test_resolve_writes_workflow_outputs_and_summary(self) -> None:
        record = StandaloneReleaseRecord(
            component="server",
            version="0.0.128",
            tag="agent-server/v0.0.128",
            release_sha="1" * 40,
            previous_tag="agent-server/v0.0.127",
            reservation="create",
        )
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            output = root / "output"
            summary = root / "summary"
            with (
                mock.patch(
                    "bos_build.cli.release_component.GitComponentReleaseOperations"
                ),
                mock.patch(
                    "bos_build.cli.release_component.resolve_standalone_release",
                    return_value=record,
                ),
            ):
                result = runner.invoke(
                    app,
                    [
                        "release",
                        "component",
                        "resolve",
                        "--component",
                        "server",
                        "--event-name",
                        "workflow_dispatch",
                        "--default-branch",
                        "main",
                        "--release-ref",
                        "HEAD",
                        "--repo",
                        "browseros-ai/BrowserOS",
                        "--github-output",
                        str(output),
                        "--github-summary",
                        str(summary),
                    ],
                )

            self.assertEqual(result.exit_code, 0, result.output)
            values = dict(
                line.split("=", 1) for line in output.read_text().splitlines()
            )
            self.assertEqual(values["version"], "0.0.128")
            self.assertEqual(values["tag"], "agent-server/v0.0.128")
            self.assertEqual(values["names"], "server")
            self.assertIn("BrowserOS server release", summary.read_text())

    def test_resolve_can_include_immutable_r2_allocations(self) -> None:
        record = StandaloneReleaseRecord(
            component="server",
            version="0.0.130",
            tag="agent-server/v0.0.130",
            release_sha="1" * 40,
            previous_tag="agent-server/v0.0.128",
            reservation="create",
        )
        env = mock.Mock(r2_bucket="browseros")
        client = object()
        with (
            mock.patch("bos_build.cli.release_component.EnvConfig", return_value=env),
            mock.patch(
                "bos_build.cli.release_component.get_r2_client", return_value=client
            ),
            mock.patch(
                "bos_build.cli.release_component.GitComponentReleaseOperations"
            ) as operations,
            mock.patch(
                "bos_build.cli.release_component.resolve_standalone_release",
                return_value=record,
            ),
        ):
            result = runner.invoke(
                app,
                [
                    "release",
                    "component",
                    "resolve",
                    "--component",
                    "server",
                    "--event-name",
                    "workflow_dispatch",
                    "--default-branch",
                    "main",
                    "--release-ref",
                    "HEAD",
                    "--repo",
                    "browseros-ai/BrowserOS",
                    "--r2-allocations",
                ],
            )

        self.assertEqual(result.exit_code, 0, result.output)
        operations.assert_called_once_with(
            mock.ANY,
            "browseros-ai/BrowserOS",
            "origin",
            r2_client=client,
            r2_bucket="browseros",
        )

    def test_manifest_versions_become_public_extension_allocations(self) -> None:
        content = render_update_manifest({"browserclaw": "0.1.9.0"})
        with mock.patch(
            "bos_build.cli.release_component._read_manifest", return_value=content
        ):
            allocations = _manifest_allocations(
                "browserclaw", ["https://cdn.browseros.com/extensions/update.xml"]
            )

        self.assertEqual(len(allocations), 1)
        self.assertEqual(allocations[0].version, "0.1.9.0")
        self.assertTrue(allocations[0].public)

    def test_stamp_updates_only_selected_component_files(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            manifest = root / "packages/browseros-agent/apps/server/package.json"
            lockfile = root / "packages/browseros-agent/bun.lock"
            manifest.parent.mkdir(parents=True)
            manifest.write_text(
                json.dumps({"name": "@browseros/server", "version": "0.0.127"})
                + "\n"
            )
            lockfile.parent.mkdir(parents=True, exist_ok=True)
            lockfile.write_text(
                "{\n"
                '  "workspaces": {\n'
                '    "apps/server": {\n'
                '      "name": "@browseros/server",\n'
                '      "version": "0.0.127",\n'
                "    },\n"
                "  },\n"
                "}\n"
            )
            result = runner.invoke(
                app,
                [
                    "release",
                    "component",
                    "stamp",
                    "--component",
                    "server",
                    "--version",
                    "0.0.128",
                    "--repo-root",
                    str(root),
                ],
            )

            self.assertEqual(result.exit_code, 0, result.output)
            self.assertEqual(json.loads(manifest.read_text())["version"], "0.0.128")
            self.assertIn('"version": "0.0.128"', lockfile.read_text())


if __name__ == "__main__":
    unittest.main()
