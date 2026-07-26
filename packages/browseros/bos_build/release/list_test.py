#!/usr/bin/env python3
"""Tests for release list composition."""

import unittest
from types import SimpleNamespace
from typing import cast
from unittest import mock

from ..core.context import Context
from ..core.products import get_product_descriptor
from ..lib.env import EnvConfig
from . import list as list_module
from .list import (
    ListModule,
    apply_list_limit,
    collect_product_rows,
    merge_product_versions,
)


def _env() -> EnvConfig:
    return cast(EnvConfig, SimpleNamespace())


class MergeProductVersionsTest(unittest.TestCase):
    def test_dedupes_preferring_productized_and_sorts_desc(self):
        rows = merge_product_versions(["0.31.0", "0.30.0"], ["0.29.0", "0.30.0"])

        self.assertEqual(
            rows, [("0.31.0", False), ("0.30.0", False), ("0.29.0", True)]
        )


class ApplyListLimitTest(unittest.TestCase):
    def test_slices_newest_and_counts_hidden(self):
        rows = [(f"0.{i}.0", False) for i in range(9, 0, -1)]

        visible, hidden = apply_list_limit(rows, 5)

        self.assertEqual(
            [version for version, _ in visible],
            ["0.9.0", "0.8.0", "0.7.0", "0.6.0", "0.5.0"],
        )
        self.assertEqual(hidden, 4)

    def test_none_limit_keeps_everything(self):
        rows = [("0.2.0", False), ("0.1.0", True)]

        self.assertEqual(apply_list_limit(rows, None), (rows, 0))


class CollectProductRowsTest(unittest.TestCase):
    def test_legacy_versions_attach_to_default_product_only(self):
        env = _env()
        with (
            mock.patch.object(
                list_module, "list_all_versions", return_value=["0.31.0"]
            ),
            mock.patch.object(
                list_module, "list_legacy_versions", return_value=["0.29.0"]
            ) as legacy,
        ):
            browseros_rows = collect_product_rows(
                get_product_descriptor("browseros"), env
            )
            claw_rows = collect_product_rows(
                get_product_descriptor("browserclaw"), env
            )

        self.assertIn(("0.29.0", True), browseros_rows)
        self.assertEqual(claw_rows, [("0.31.0", False)])
        legacy.assert_called_once()


class PrintProductSectionTest(unittest.TestCase):
    def _render(self, module: ListModule, product_id: str, rows) -> list[str]:
        lines: list[str] = []
        with mock.patch.object(list_module, "log_info", side_effect=lines.append):
            module._print_product_section(get_product_descriptor(product_id), rows)
        return lines

    def test_empty_section_still_renders(self):
        lines = self._render(ListModule(), "browserclaw", [])

        self.assertTrue(any("BrowserClaw" in line for line in lines))
        self.assertTrue(any("no releases" in line for line in lines))

    def test_truncation_hint_names_all_flag(self):
        rows = [(f"0.{i}.0", False) for i in range(9, 0, -1)]

        lines = self._render(ListModule(limit=5), "browseros", rows)

        self.assertTrue(any("4 more" in line and "--all" in line for line in lines))
        self.assertTrue(any(line.strip() == "0.5.0" for line in lines))
        self.assertFalse(any(line.strip() == "0.4.0" for line in lines))

    def test_legacy_rows_are_tagged(self):
        lines = self._render(ListModule(), "browseros", [("0.29.0", True)])

        self.assertTrue(any(line.strip() == "0.29.0 (legacy)" for line in lines))


class DetailViewTest(unittest.TestCase):
    def test_detail_fetch_uses_context_product(self):
        ctx = cast(
            Context,
            SimpleNamespace(
                release_version="0.31.0",
                env=_env(),
                product=get_product_descriptor("browserclaw"),
            ),
        )

        with mock.patch.object(
            list_module, "fetch_all_release_metadata", return_value={}
        ) as fetch:
            ListModule().execute(ctx)

        fetch.assert_called_once_with("0.31.0", ctx.env, "browserclaw")


if __name__ == "__main__":
    unittest.main()
