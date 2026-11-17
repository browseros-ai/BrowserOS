#!/usr/bin/env python3
"""Tests for release artifact downloads."""

import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from typing import cast
from unittest import mock

import requests

from ..core.context import Context
from ..core.products import get_product_descriptor
from . import download as download_module
from .download import DownloadModule

VERSION = "0.99.0"
METADATA = {
    "macos": {
        "product": "browseros",
        "version": VERSION,
        "platform": "macos",
        "artifacts": {
            "arm64": {
                "filename": "BrowserOS-arm64.dmg",
                "url": "https://cdn.invalid/BrowserOS-arm64.dmg",
                "size": 100,
            },
        },
    }
}


def _ctx() -> Context:
    return cast(
        Context,
        SimpleNamespace(
            release_version=VERSION,
            env=None,
            product=get_product_descriptor("browseros"),
        ),
    )


class FlakyResponse:
    """CDN response that streams one chunk then resets the connection."""

    headers = {"content-length": "100"}

    def raise_for_status(self):
        pass

    def iter_content(self, chunk_size=65536):
        yield b"partial-"
        raise requests.ConnectionError("simulated CDN reset mid-stream")


class FullResponse:
    headers = {"content-length": "16"}

    def raise_for_status(self):
        pass

    def iter_content(self, chunk_size=65536):
        yield b"0123456789abcdef"


class DownloadModuleTest(unittest.TestCase):
    def test_failed_download_raises_and_leaves_no_residue(self):
        with tempfile.TemporaryDirectory() as tmp:
            with (
                mock.patch.object(
                    download_module,
                    "fetch_all_release_metadata",
                    return_value=METADATA,
                ),
                mock.patch.object(
                    download_module.requests, "get", return_value=FlakyResponse()
                ),
                self.assertRaisesRegex(RuntimeError, "BrowserOS-arm64.dmg"),
            ):
                DownloadModule(os_filter="macos", output_dir=Path(tmp)).execute(
                    _ctx()
                )

            leftovers = [
                f.name
                for f in Path(tmp).rglob("*")
                if f.name.endswith((".dmg", ".partial"))
            ]
            self.assertEqual(leftovers, [])

    def test_empty_metadata_raises(self):
        with tempfile.TemporaryDirectory() as tmp:
            with (
                mock.patch.object(
                    download_module, "fetch_all_release_metadata", return_value={}
                ),
                self.assertRaisesRegex(RuntimeError, "No release metadata"),
            ):
                DownloadModule(os_filter="macos", output_dir=Path(tmp)).execute(
                    _ctx()
                )

    def test_successful_download_stages_then_renames(self):
        with tempfile.TemporaryDirectory() as tmp:
            with (
                mock.patch.object(
                    download_module,
                    "fetch_all_release_metadata",
                    return_value=METADATA,
                ),
                mock.patch.object(
                    download_module.requests, "get", return_value=FullResponse()
                ),
            ):
                DownloadModule(os_filter="macos", output_dir=Path(tmp)).execute(
                    _ctx()
                )

            dest = Path(tmp) / VERSION / "BrowserOS-arm64.dmg"
            self.assertEqual(dest.read_bytes(), b"0123456789abcdef")
            self.assertEqual(list(Path(tmp).rglob("*.partial")), [])


if __name__ == "__main__":
    unittest.main()
