#!/usr/bin/env python3
"""Tests for Chromium checkout build locking."""

import json
import multiprocessing
import tempfile
import unittest
from pathlib import Path

from bos_build.core.checkout_lock import CheckoutLockError, ChromiumCheckoutLock


def _hold_checkout_lock(chromium_src: str, ready, release) -> None:
    with ChromiumCheckoutLock(
        Path(chromium_src),
        product="browserclaw",
        command=("test-holder",),
    ):
        ready.set()
        release.wait(15)


class ChromiumCheckoutLockTest(unittest.TestCase):
    def test_records_holder_metadata_and_releases(self):
        with tempfile.TemporaryDirectory() as tmp:
            chromium_src = Path(tmp) / "src"
            chromium_src.mkdir()

            with ChromiumCheckoutLock(
                chromium_src,
                product="browserclaw",
                command=("browseros", "build"),
            ) as lock:
                metadata = json.loads(lock.lock_path.read_text())
                self.assertEqual(metadata["product"], "browserclaw")
                self.assertEqual(metadata["chromium_src"], str(chromium_src.resolve()))
                self.assertEqual(metadata["command"], ["browseros", "build"])

            with ChromiumCheckoutLock(chromium_src, product="browseros"):
                pass

    def test_contention_fails_with_holder_details(self):
        with tempfile.TemporaryDirectory() as tmp:
            chromium_src = Path(tmp) / "src"
            chromium_src.mkdir()

            ctx = multiprocessing.get_context("spawn")
            ready = ctx.Event()
            release = ctx.Event()
            proc = ctx.Process(
                target=_hold_checkout_lock,
                args=(str(chromium_src), ready, release),
            )
            proc.start()
            self.addCleanup(proc.join, 5)
            self.addCleanup(release.set)

            self.assertTrue(ready.wait(5), f"lock holder exited: {proc.exitcode}")
            with self.assertRaises(CheckoutLockError) as raised:
                with ChromiumCheckoutLock(chromium_src, product="browseros"):
                    pass

            message = str(raised.exception)
            self.assertIn("Chromium checkout is already locked", message)
            self.assertIn("product=browserclaw", message)
            self.assertIn("started_at=", message)
            self.assertIn("--lock-wait", message)

            release.set()
            proc.join(5)
            self.assertEqual(proc.exitcode, 0)


if __name__ == "__main__":
    unittest.main()
