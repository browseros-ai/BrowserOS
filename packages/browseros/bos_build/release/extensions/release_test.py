#!/usr/bin/env python3
"""Tests for the ext release orchestration module and pipeline assembly."""

import unittest
from pathlib import Path
from types import SimpleNamespace
from typing import cast
from unittest.mock import MagicMock, patch

from ...core.context import Context
from ...core.step import ValidationError
from .release import ExtensionReleaseModule, build_pipeline

MODULE = "bos_build.release.extensions.release"

ALL_KEYS = {
    "BROWSEROS_AGENT_V2_KEY": "agent-pem",
    "BROWSEROS_CONTROLLER_KEY": "controller-pem",
    "BUGREPORTER_KEY": "bugreporter-pem",
    "BROWSERCLAW_KEY": "claw-pem",
}


def _ctx(has_r2=True, bucket="browseros"):
    return cast(
        Context,
        SimpleNamespace(
            env=SimpleNamespace(has_r2_config=lambda: has_r2, r2_bucket=bucket)
        ),
    )


class BuildPipelineTest(unittest.TestCase):
    def test_unknown_name_raises_with_valid_names(self):
        with self.assertRaisesRegex(ValueError, "bugreporter"):
            build_pipeline(
                version="1.0.0",
                name="agent-v2",
                branch=None,
                chrome_binary=None,
            )

    def test_named_extension_gets_one_release_step(self):
        steps = build_pipeline(
            version="1.2.3",
            name="agent",
            branch=None,
            chrome_binary=None,
        )
        self.assertEqual(len(steps), 1)
        release = steps[0]
        self.assertIsInstance(release, ExtensionReleaseModule)
        self.assertEqual(release.names, ("agent",))

    def test_all_extensions_get_one_release_step(self):
        steps = build_pipeline(
            version="2.0.0",
            name=None,
            branch=None,
            chrome_binary=None,
        )
        self.assertEqual(len(steps), 1)
        release = steps[0]
        self.assertIsInstance(release, ExtensionReleaseModule)
        self.assertEqual(
            release.names, ("agent", "controller", "bugreporter", "browserclaw")
        )

    def test_dash_prefixed_branch_rejected_at_assembly(self):
        with self.assertRaisesRegex(ValueError, "branch"):
            build_pipeline(
                version="1.0.0",
                name="controller",
                branch="--upload-pack=/bin/sh",
                chrome_binary=None,
            )

    def test_malformed_version_rejected_at_assembly(self):
        for version in ("1.0.0-beta", "abc", "", "1..2"):
            with self.assertRaisesRegex(ValueError, "version"):
                build_pipeline(
                    version=version,
                    name="agent",
                    branch=None,
                    chrome_binary=None,
                )


class ValidateTest(unittest.TestCase):
    def _module(self, names=("agent",), **kwargs):
        return ExtensionReleaseModule(version="1.0.0", names=names, **kwargs)

    def test_missing_signing_key_env_names_the_variable(self):
        with patch.dict("os.environ", {}, clear=False):
            import os

            os.environ.pop("BROWSEROS_AGENT_V2_KEY", None)
            with patch(f"{MODULE}.find_chrome_binary", return_value="chrome"):
                with self.assertRaisesRegex(
                    ValidationError, "BROWSEROS_AGENT_V2_KEY"
                ):
                    self._module().validate(_ctx())

    def test_missing_r2_config_fails(self):
        with patch.dict("os.environ", ALL_KEYS):
            with patch(f"{MODULE}.find_chrome_binary", return_value="chrome"):
                with self.assertRaisesRegex(ValidationError, "R2"):
                    self._module().validate(_ctx(has_r2=False))

    def test_chrome_resolution_failure_fails_validation(self):
        with patch.dict("os.environ", ALL_KEYS):
            with patch(
                f"{MODULE}.find_chrome_binary",
                side_effect=RuntimeError("no chrome anywhere"),
            ):
                with self.assertRaisesRegex(ValidationError, "no chrome"):
                    self._module().validate(_ctx())

    def test_unknown_name_fails_validation(self):
        with self.assertRaisesRegex(ValidationError, "Unknown extension"):
            self._module(names=("nope",)).validate(_ctx())


class ExecuteTest(unittest.TestCase):
    def setUp(self):
        self.monorepo = Path("/mono")
        self.work = Path("/work")

        patches = {
            "resolve_source": MagicMock(
                side_effect=lambda spec, **kw: Path("/src") / spec.name
            ),
            "update_manifest_version": MagicMock(),
            "write_env_file": MagicMock(),
            "run_command": MagicMock(),
            "pack_crx": MagicMock(
                side_effect=lambda dist, key, chrome, out, **kw: out
            ),
            "upload_file_to_r2": MagicMock(return_value=True),
            "get_r2_client": MagicMock(return_value="r2-client"),
            "find_chrome_binary": MagicMock(return_value="chrome-bin"),
        }
        self.mocks = {}
        self.tracker = MagicMock()
        for name, mock in patches.items():
            patcher = patch(f"{MODULE}.{name}", mock)
            patcher.start()
            self.addCleanup(patcher.stop)
            self.mocks[name] = mock
            self.tracker.attach_mock(mock, name)

        env_patcher = patch.dict("os.environ", ALL_KEYS)
        env_patcher.start()
        self.addCleanup(env_patcher.stop)

    def _module(self, names, **kwargs):
        return ExtensionReleaseModule(
            version="1.0.0",
            names=names,
            monorepo_root=self.monorepo,
            work_root=self.work,
            **kwargs,
        )

    def test_agent_flow_order_and_arguments(self):
        self._module(("agent",)).execute(_ctx())

        called = [c[0] for c in self.tracker.mock_calls]
        self.assertEqual(
            called,
            [
                "get_r2_client",
                "find_chrome_binary",
                "resolve_source",
                "update_manifest_version",
                "write_env_file",
                "run_command",
                "run_command",
                "pack_crx",
                "upload_file_to_r2",
            ],
        )

        resolve_kwargs = self.mocks["resolve_source"].call_args.kwargs
        self.assertEqual(resolve_kwargs["monorepo_root"], self.monorepo)
        self.assertEqual(resolve_kwargs["work_root"], self.work)
        self.assertIsNone(resolve_kwargs["branch_override"])

        self.mocks["update_manifest_version"].assert_called_once_with(
            Path("/src/agent/apps/app/package.json"), "1.0.0"
        )
        self.mocks["write_env_file"].assert_called_once()
        env_args = self.mocks["write_env_file"].call_args.args
        self.assertEqual(env_args[0], Path("/src/agent/apps/app"))
        self.assertIn("VITE_PUBLIC_BROWSEROS_API", env_args[1])

        commands = [c.args for c in self.mocks["run_command"].call_args_list]
        self.assertEqual(
            commands,
            [
                ("bun ci", Path("/src/agent")),
                ("bun run build:agent", Path("/src/agent")),
            ],
        )

        pack_args = self.mocks["pack_crx"].call_args.args
        self.assertEqual(pack_args[0], Path("/src/agent/apps/app/dist/chrome-mv3"))
        self.assertEqual(pack_args[1], "agent-pem")
        self.assertEqual(pack_args[2], "chrome-bin")
        self.assertEqual(pack_args[3], self.work / "dist" / "agent-1.0.0.crx")

        self.mocks["upload_file_to_r2"].assert_called_once_with(
            "r2-client",
            self.work / "dist" / "agent-1.0.0.crx",
            "extensions/agent-1.0.0.crx",
            "browseros",
        )

    def test_env_file_lands_at_source_root_without_env_dir(self):
        self._module(("bugreporter",)).execute(_ctx())
        env_args = self.mocks["write_env_file"].call_args.args
        self.assertEqual(env_args[0], Path("/src/bugreporter"))

    def test_upload_failure_stops_later_extensions(self):
        self.mocks["upload_file_to_r2"].return_value = False
        with self.assertRaisesRegex(RuntimeError, "extensions/agent-1.0.0.crx"):
            self._module(("agent", "bugreporter")).execute(_ctx())

        resolved = [
            c.args[0].name for c in self.mocks["resolve_source"].call_args_list
        ]
        self.assertEqual(resolved, ["agent"])

    def test_branch_override_reaches_resolver(self):
        self._module(("bugreporter",), branch_override="canary").execute(_ctx())
        self.assertEqual(
            self.mocks["resolve_source"].call_args.kwargs["branch_override"],
            "canary",
        )

    def test_injected_r2_client_skips_factory(self):
        self._module(("agent",), r2_client="prebuilt").execute(_ctx())
        self.mocks["get_r2_client"].assert_not_called()
        self.assertEqual(
            self.mocks["upload_file_to_r2"].call_args.args[0], "prebuilt"
        )


if __name__ == "__main__":
    unittest.main()
