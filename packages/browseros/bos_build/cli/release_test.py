#!/usr/bin/env python3
"""CLI surface tests for the release subcommands."""

import re
import unittest
from unittest import mock

from typer.testing import CliRunner

from bos_build.browseros import app
from bos_build.cli import release as release_cli

runner = CliRunner()
ANSI_RE = re.compile(r"\x1b\[[0-9;]*m")


def invoke(*args: str):
    return runner.invoke(app, ["release", *args])


def combined(result) -> str:
    """stdout + stderr across click versions (8.2 split them)."""
    out = result.output
    try:
        out += result.stderr
    except (ValueError, AttributeError):
        pass
    return out


def plain_output(result) -> str:
    return ANSI_RE.sub("", combined(result))


class ReleaseHelpTest(unittest.TestCase):
    def test_lists_subcommands_without_flag_soup(self):
        result = invoke("--help")

        self.assertEqual(result.exit_code, 0, combined(result))
        for command in ("list", "appcast", "publish", "download", "github"):
            self.assertIn(command, result.output)
        for flag in ("--list", "--appcast", "--publish", "--download"):
            self.assertNotIn(flag, result.output)

    def test_bare_release_shows_help(self):
        result = runner.invoke(app, ["release"])

        self.assertEqual(result.exit_code, 0, combined(result))
        self.assertIn("Usage", result.output)

    def test_show_modules_lists_modules(self):
        result = invoke("--show-modules")

        self.assertEqual(result.exit_code, 0, combined(result))
        self.assertIn("appcast", combined(result))

    def test_list_help_shows_bounding_options(self):
        result = invoke("list", "--help")

        self.assertEqual(result.exit_code, 0, combined(result))
        help_text = plain_output(result)
        for token in ("--product", "--limit", "--all", "--version", "VERSION"):
            self.assertIn(token, help_text)


class ReleaseListInvocationTest(unittest.TestCase):
    def test_conflicting_positional_and_option_version_errors(self):
        result = invoke("list", "0.31.0", "--version", "0.32.0")

        self.assertNotEqual(result.exit_code, 0)
        self.assertIn("Conflicting", combined(result))

    def test_all_flag_removes_limit_and_spans_products(self):
        with mock.patch.object(release_cli, "execute_module") as em:
            result = invoke("list", "--all")

        self.assertEqual(result.exit_code, 0, combined(result))
        module = em.call_args[0][1]
        self.assertIsNone(module.limit)
        self.assertEqual(
            [product.id for product in module.products],
            ["browseros", "browserclaw"],
        )

    def test_limit_and_product_filter(self):
        with mock.patch.object(release_cli, "execute_module") as em:
            result = invoke("list", "--product", "browserclaw", "-n", "3")

        self.assertEqual(result.exit_code, 0, combined(result))
        module = em.call_args[0][1]
        self.assertEqual(module.limit, 3)
        self.assertEqual([product.id for product in module.products], ["browserclaw"])

    def test_version_detail_uses_product_context(self):
        with mock.patch.object(release_cli, "execute_module") as em:
            result = invoke("list", "0.31.0", "--product", "browserclaw")

        self.assertEqual(result.exit_code, 0, combined(result))
        ctx, module = em.call_args[0]
        self.assertEqual(ctx.release_version, "0.31.0")
        self.assertEqual(ctx.product.id, "browserclaw")
        self.assertIsNone(module.products)

    def test_matching_positional_and_option_version_is_not_a_conflict(self):
        with mock.patch.object(release_cli, "execute_module"):
            result = invoke("list", "0.31.0", "--version", "0.31.0")

        self.assertNotIn("Conflicting", combined(result))

    def test_unknown_product_names_valid_ids(self):
        result = invoke("list", "--product", "nosuch")

        self.assertNotEqual(result.exit_code, 0)
        self.assertIn("browserclaw", combined(result))


class ReleaseVersionRequiredTest(unittest.TestCase):
    def test_appcast_requires_version(self):
        self.assertNotEqual(invoke("appcast").exit_code, 0)

    def test_publish_requires_version(self):
        self.assertNotEqual(invoke("publish").exit_code, 0)

    def test_download_requires_version(self):
        self.assertNotEqual(invoke("download").exit_code, 0)


class CreateReleaseContextTest(unittest.TestCase):
    def test_sets_product_from_registry(self):
        ctx = release_cli.create_release_context("1.0.0", product="browserclaw")

        self.assertEqual(ctx.product.id, "browserclaw")
        self.assertEqual(ctx.release_version, "1.0.0")

    def test_defaults_to_browseros(self):
        ctx = release_cli.create_release_context("1.0.0")

        self.assertEqual(ctx.product.id, "browseros")


if __name__ == "__main__":
    unittest.main()
