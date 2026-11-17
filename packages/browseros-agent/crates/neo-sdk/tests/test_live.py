"""The CI verdict must distinguish exercised tests from libtest's successful early returns."""

import importlib.util
from pathlib import Path
import sys
import unittest

spec = importlib.util.spec_from_file_location("sdk_live", Path(__file__).parents[1] / "live.py")
live = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = live
spec.loader.exec_module(live)


class LiveVerdictTests(unittest.TestCase):
    def result(self, text, code=0):
        return live.assess_result({"exitCode": code, "timedOut": False}, text)

    def test_an_entire_target_returning_early_is_not_coverage(self):
        text = "LIVE_TEST_EARLY_RETURN missing owned browser\n" * 5
        text += "test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s"
        result = self.result(text)
        self.assertEqual((result["nativePassed"], result["passed"], result["earlyReturns"]), (5, 0, 5))
        self.assertFalse(result["ok"])

    def test_zero_selected_tests_is_not_coverage(self):
        result = self.result("test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 0.00s")
        self.assertFalse(result["ok"])

    def test_exercised_rows_pass_with_zero_early_returns(self):
        result = self.result("test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.26s")
        self.assertEqual((result["passed"], result["failed"], result["earlyReturns"]), (5, 0, 0))
        self.assertTrue(result["ok"])

    def test_one_early_return_is_visible_and_fails_the_target(self):
        text = "test a_row ... LIVE_TEST_EARLY_RETURN hosted console\nok\n"
        text += "test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.0s"
        result = self.result(text)
        self.assertEqual((result["passed"], result["earlyReturns"]), (4, 1))
        self.assertFalse(result["ok"])

    def test_native_failures_keep_their_counts(self):
        result = self.result("test result: FAILED. 4 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.0s", 101)
        self.assertEqual((result["passed"], result["failed"]), (4, 1))
        self.assertFalse(result["ok"])

    def test_multiple_target_summaries_are_not_one_target_result(self):
        summary = "test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.0s\n"
        self.assertFalse(self.result(summary * 2)["ok"])


if __name__ == "__main__":
    unittest.main()
