import unittest

from validate_linux_server import validate_versions


class GlibcCompatibilityTest(unittest.TestCase):
    def test_accepts_baseline_and_older_symbols(self):
        self.assertEqual(
            validate_versions("Name: GLIBC_2.2.5\nName: GLIBC_2.17", "2.17"),
            "2.17",
        )

    def test_rejects_the_reported_release_including_weak_requirements(self):
        with self.assertRaisesRegex(ValueError, "GLIBC_2.39"):
            validate_versions(
                "Name: GLIBC_2.38 Flags: none\nName: GLIBC_2.39 Flags: WEAK", "2.17"
            )

    def test_compares_versions_numerically(self):
        with self.assertRaisesRegex(ValueError, "GLIBC_2.9"):
            validate_versions("Name: GLIBC_2.9\nName: GLIBC_2.8", "2.8")
        self.assertEqual(validate_versions("Name: GLIBC_2.9", "2.17"), "2.9")

    def test_rejects_a_newer_loader_abi_even_with_old_numbered_symbols(self):
        with self.assertRaisesRegex(ValueError, "Unsupported GLIBC ABI"):
            validate_versions("Name: GLIBC_2.17\nName: GLIBC_ABI_DT_RELR", "2.17")

    def test_rejects_an_unversioned_or_wrong_format_binary(self):
        for output in (
            "",
            "No version information found in this file.",
            "GLIBC_PRIVATE",
        ):
            with self.subTest(output=output), self.assertRaisesRegex(
                ValueError, "No GLIBC"
            ):
                validate_versions(output, "2.17")


if __name__ == "__main__":
    unittest.main()
