#!/usr/bin/env python3
"""Tests for DIRECT-mode config resolution."""

import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from bos_build.core.resolver import resolve_config, resolve_pipeline
from bos_build.lib.testing import MockChromium


class ResolveConfigDirectModeTest(unittest.TestCase):
    def test_missing_chromium_src_everywhere_raises(self):
        env = {k: v for k, v in os.environ.items() if k not in ("CHROMIUM_SRC", "ARCH")}
        with mock.patch.dict(os.environ, env, clear=True):
            with self.assertRaises(ValueError) as err:
                resolve_config(cli_args={})
        self.assertIn("chromium_src required", str(err.exception))

    def test_cli_chromium_src_and_arch_resolve(self):
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp))
            cli_args = {
                "chromium_src": str(m.src),
                "arch": "arm64",
                "build_type": "release",
            }
            contexts = resolve_config(cli_args=cli_args)
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
                contexts = resolve_config(cli_args={})
            self.assertEqual(contexts[0].chromium_src, m.src)
            self.assertEqual(contexts[0].architecture, "x64")
            self.assertEqual(contexts[0].build_type, "debug")

    def test_invalid_arch_raises(self):
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp))
            cli_args = {"chromium_src": str(m.src), "arch": "sparc"}
            with self.assertRaises(ValueError) as err:
                resolve_config(cli_args=cli_args)
            self.assertIn("invalid architecture", str(err.exception))

    def test_product_flag_resolves_descriptor(self):
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp))
            cli_args = {"chromium_src": str(m.src), "product": "browserclaw"}
            contexts = resolve_config(cli_args=cli_args)
            self.assertEqual(contexts[0].product.id, "browserclaw")
            self.assertEqual(contexts[0].product.app_base_name, "BrowserClaw")

    def test_unknown_product_raises(self):
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp))
            cli_args = {"chromium_src": str(m.src), "product": "netscape"}
            with self.assertRaises(ValueError) as err:
                resolve_config(cli_args=cli_args)
            self.assertIn("Unknown build.product", str(err.exception))

    def test_extra_gn_args_threaded_to_context(self):
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp))
            cli_args = {
                "chromium_src": str(m.src),
                "extra_gn_args": ("symbol_level=2", "dcheck_always_on=true"),
            }
            contexts = resolve_config(cli_args=cli_args)
            self.assertEqual(
                contexts[0].extra_gn_args, ("symbol_level=2", "dcheck_always_on=true")
            )

    def test_extra_gn_args_default_empty(self):
        with tempfile.TemporaryDirectory() as tmp:
            m = MockChromium(Path(tmp))
            contexts = resolve_config(cli_args={"chromium_src": str(m.src)})
            self.assertEqual(contexts[0].extra_gn_args, ())


class ResolvePipelineTest(unittest.TestCase):
    def test_direct_mode_requires_modules_or_flags(self):
        with self.assertRaises(ValueError) as err:
            resolve_pipeline(cli_args={})
        self.assertIn("No pipeline specified", str(err.exception))

    def test_direct_mode_rejects_modules_and_flags_together(self):
        cli_args = {"modules": "clean", "build": True}
        with self.assertRaises(ValueError) as err:
            resolve_pipeline(cli_args=cli_args)
        self.assertIn("Cannot use both", str(err.exception))

    def test_direct_mode_parses_modules_string(self):
        pipeline = resolve_pipeline(cli_args={"modules": "clean, compile ,sign_macos"})
        self.assertEqual(pipeline, ["clean", "compile", "sign_macos"])

    def test_direct_mode_expands_phase_flags_in_execution_order(self):
        execution_order = [
            ("setup", ["clean", "git_setup"]),
            ("prep", ["patches"]),
            ("build", ["configure", "compile"]),
        ]
        cli_args = {"setup": True, "build": True}
        pipeline = resolve_pipeline(cli_args=cli_args, execution_order=execution_order)
        self.assertEqual(pipeline, ["clean", "git_setup", "configure", "compile"])

    def test_direct_mode_phase_flags_require_execution_order(self):
        with self.assertRaises(ValueError) as err:
            resolve_pipeline(cli_args={"setup": True})
        self.assertIn("execution_order required", str(err.exception))


if __name__ == "__main__":
    unittest.main()
