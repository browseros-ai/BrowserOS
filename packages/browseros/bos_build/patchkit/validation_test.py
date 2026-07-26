#!/usr/bin/env python3
"""Tests for feature name and description validation."""

import unittest

from .validation import validate_description, validate_feature_name


class ValidateFeatureNameTest(unittest.TestCase):
    def test_accepts_kebab_and_snake_names(self):
        for name in ("llm-chat", "a1_b", "feature2", "x"):
            valid, error = validate_feature_name(name)
            self.assertTrue(valid, f"{name!r} should be valid: {error}")
            self.assertEqual(error, "")

    def test_rejects_empty(self):
        valid, error = validate_feature_name("")
        self.assertFalse(valid)
        self.assertIn("empty", error)

    def test_rejects_spaces(self):
        valid, error = validate_feature_name("llm chat")
        self.assertFalse(valid)
        self.assertIn("spaces", error)

    def test_rejects_colon(self):
        valid, error = validate_feature_name("feat:llm")
        self.assertFalse(valid)
        self.assertIn(":", error)

    def test_rejects_uppercase(self):
        valid, error = validate_feature_name("LLM-Chat")
        self.assertFalse(valid)
        self.assertIn("lowercase", error)

    def test_rejects_leading_hyphen(self):
        valid, _ = validate_feature_name("-leading")
        self.assertFalse(valid)


class ValidateDescriptionTest(unittest.TestCase):
    def test_accepts_all_valid_prefixes(self):
        for prefix in ("feat:", "fix:", "build:", "chore:", "series:"):
            valid, error = validate_description(f"{prefix} something")
            self.assertTrue(valid, f"{prefix} should be valid: {error}")

    def test_rejects_empty(self):
        valid, error = validate_description("   ")
        self.assertFalse(valid)
        self.assertIn("empty", error)

    def test_rejects_missing_prefix(self):
        valid, error = validate_description("adds LLM chat")
        self.assertFalse(valid)
        self.assertIn("must start with", error)


if __name__ == "__main__":
    unittest.main()
