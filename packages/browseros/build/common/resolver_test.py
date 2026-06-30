#!/usr/bin/env python3
"""Tests for config/pipeline resolution against a mock chromium checkout."""

import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from .resolver import resolve_config, resolve_pipeline
from .testing import MockChromium


class ResolveConfigConfigModeTest(unittest.TestCase):
    def test_missing_chromium_src_raises(self):
        with self.assertRaises(ValueError) as err:
            resolve_config(cli_args={}, yaml_config={"build": {}})
        self.assertIn("chromium_src required", str(err.exception))

    def test_nonexistent_chromium_src_raises(self):
        yaml_config = {"build": {"chromium_src": "/nonexistent/chromium/src"}}
        with self.assertRaises(ValueError) as err:
            resolve_config(cli_args={}, yaml_config=yaml_config)
        self.assertIn("does not exist", str(err.exception))

    def test_arch_list_yields_one_context_per_arch(self):
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp))
            yaml_config = {
                "build": {
                    "chromium_src": str(m.src),
                    "architecture": ["x64", "arm64"],
                    "type": "release",
                }
            }
            contexts = resolve_config(cli_args={}, yaml_config=yaml_config)
            self.assertEqual([c.architecture for c in contexts], ["x64", "arm64"])
            self.assertEqual({c.build_type for c in contexts}, {"release"})
            self.assertEqual({c.chromium_src for c in contexts}, {m.src})

    def test_invalid_arch_raises(self):
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp))
            yaml_config = {
                "build": {"chromium_src": str(m.src), "architecture": "mips"}
            }
            with self.assertRaises(ValueError) as err:
                resolve_config(cli_args={}, yaml_config=yaml_config)
            self.assertIn("invalid architecture", str(err.exception))

    def test_cli_overrides_yaml(self):
        with (
            tempfile.TemporaryDirectory() as yaml_tmp,
            tempfile.TemporaryDirectory() as cli_tmp,
        ):
            yaml_checkout = MockChromium(Path(yaml_tmp))
            cli_checkout = MockChromium(Path(cli_tmp))
            yaml_config = {
                "build": {
                    "chromium_src": str(yaml_checkout.src),
                    "architecture": "x64",
                    "type": "debug",
                }
            }
            cli_args = {
                "chromium_src": str(cli_checkout.src),
                "arch": "arm64",
                "build_type": "release",
            }
            contexts = resolve_config(cli_args=cli_args, yaml_config=yaml_config)
            self.assertEqual(len(contexts), 1)
            self.assertEqual(contexts[0].chromium_src, cli_checkout.src)
            self.assertEqual(contexts[0].architecture, "arm64")
            self.assertEqual(contexts[0].build_type, "release")

    def test_build_type_defaults_to_debug(self):
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp))
            yaml_config = {
                "build": {"chromium_src": str(m.src), "architecture": "x64"}
            }
            contexts = resolve_config(cli_args={}, yaml_config=yaml_config)
            self.assertEqual(contexts[0].build_type, "debug")

    def test_product_defaults_to_browseros(self):
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp))
            yaml_config = {
                "build": {"chromium_src": str(m.src), "architecture": "x64"}
            }
            contexts = resolve_config(cli_args={}, yaml_config=yaml_config)
            self.assertEqual(contexts[0].product.id, "browseros")

    def test_browserclaw_product_resolves_descriptor(self):
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp))
            yaml_config = {
                "build": {
                    "chromium_src": str(m.src),
                    "architecture": "arm64",
                    "product": "browserclaw",
                }
            }
            contexts = resolve_config(cli_args={}, yaml_config=yaml_config)
            self.assertEqual(contexts[0].product.id, "browserclaw")
            self.assertEqual(contexts[0].BROWSEROS_APP_BASE_NAME, "BrowserClaw")

    def test_unknown_product_raises(self):
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp))
            yaml_config = {
                "build": {
                    "chromium_src": str(m.src),
                    "architecture": "x64",
                    "product": "netscape",
                }
            }
            with self.assertRaisesRegex(ValueError, "Unknown build.product"):
                resolve_config(cli_args={}, yaml_config=yaml_config)


class ResolveConfigDirectModeTest(unittest.TestCase):
    def test_missing_chromium_src_everywhere_raises(self):
        env = {k: v for k, v in os.environ.items() if k not in ("CHROMIUM_SRC", "ARCH")}
        with mock.patch.dict(os.environ, env, clear=True):
            with self.assertRaises(ValueError) as err:
                resolve_config(cli_args={}, yaml_config=None)
        self.assertIn("chromium_src required", str(err.exception))

    def test_cli_chromium_src_and_arch_resolve(self):
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp))
            cli_args = {
                "chromium_src": str(m.src),
                "arch": "arm64",
                "build_type": "release",
            }
            contexts = resolve_config(cli_args=cli_args, yaml_config=None)
            self.assertEqual(len(contexts), 1)
            self.assertEqual(contexts[0].chromium_src, m.src)
            self.assertEqual(contexts[0].architecture, "arm64")
            self.assertEqual(contexts[0].build_type, "release")
            self.assertEqual(contexts[0].product.id, "browseros")

    def test_env_chromium_src_used_when_no_cli(self):
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp))
            with mock.patch.dict(
                os.environ, {"CHROMIUM_SRC": str(m.src), "ARCH": "x64"}
            ):
                contexts = resolve_config(cli_args={}, yaml_config=None)
            self.assertEqual(contexts[0].chromium_src, m.src)
            self.assertEqual(contexts[0].architecture, "x64")
            self.assertEqual(contexts[0].build_type, "debug")

    def test_invalid_arch_raises(self):
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp))
            cli_args = {"chromium_src": str(m.src), "arch": "sparc"}
            with self.assertRaises(ValueError) as err:
                resolve_config(cli_args=cli_args, yaml_config=None)
            self.assertIn("invalid architecture", str(err.exception))


class ResolvePipelineTest(unittest.TestCase):
    def test_config_mode_requires_modules(self):
        with self.assertRaises(ValueError) as err:
            resolve_pipeline(cli_args={}, yaml_config={"build": {}})
        self.assertIn("modules required", str(err.exception))

    def test_config_mode_returns_yaml_modules(self):
        yaml_config = {"modules": ["clean", "configure", "compile"]}
        pipeline = resolve_pipeline(cli_args={}, yaml_config=yaml_config)
        self.assertEqual(pipeline, ["clean", "configure", "compile"])

    def test_direct_mode_requires_modules_or_flags(self):
        with self.assertRaises(ValueError) as err:
            resolve_pipeline(cli_args={}, yaml_config=None)
        self.assertIn("No pipeline specified", str(err.exception))

    def test_direct_mode_rejects_modules_and_flags_together(self):
        cli_args = {"modules": "clean", "build": True}
        with self.assertRaises(ValueError) as err:
            resolve_pipeline(cli_args=cli_args, yaml_config=None)
        self.assertIn("Cannot use both", str(err.exception))

    def test_direct_mode_parses_modules_string(self):
        pipeline = resolve_pipeline(
            cli_args={"modules": "clean, compile ,sign_macos"}, yaml_config=None
        )
        self.assertEqual(pipeline, ["clean", "compile", "sign_macos"])

    def test_direct_mode_expands_phase_flags_in_execution_order(self):
        execution_order = [
            ("setup", ["clean", "git_setup"]),
            ("prep", ["patches"]),
            ("build", ["configure", "compile"]),
        ]
        cli_args = {"setup": True, "build": True}
        pipeline = resolve_pipeline(
            cli_args=cli_args, yaml_config=None, execution_order=execution_order
        )
        self.assertEqual(pipeline, ["clean", "git_setup", "configure", "compile"])

    def test_direct_mode_phase_flags_require_execution_order(self):
        with self.assertRaises(ValueError) as err:
            resolve_pipeline(cli_args={"setup": True}, yaml_config=None)
        self.assertIn("execution_order required", str(err.exception))


if __name__ == "__main__":
    unittest.main()
