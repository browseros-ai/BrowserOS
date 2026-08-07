#!/usr/bin/env python3
"""Tests for the patch-stack doctor."""

import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from typing import Dict, List, Optional
from unittest import mock

from bos_build.lib.paths import get_package_root
from bos_build.patchkit.features_io import canonical_features_path
from bos_build.patchkit.doctor import (
    ApplyFailure,
    ApplyReport,
    build_report,
    check_apply,
    check_repo,
    compute_claims,
    diagnose_repo,
    load_features,
    patch_base_paths,
)


def _patches_dir(case: unittest.TestCase, files: List[str]) -> Path:
    tmp = tempfile.TemporaryDirectory()
    case.addCleanup(tmp.cleanup)
    root = Path(tmp.name) / "chromium_patches"
    for rel in files:
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("diff")
    return root


def _feature(files: List[str], description: str = "feat: test feature") -> Dict:
    return {"description": description, "files": files}


class PatchBasePathsTest(unittest.TestCase):
    def test_markers_map_to_base_and_dotfiles_skipped(self):
        patches = _patches_dir(
            self,
            [
                "chrome/a.cc",
                "chrome/b.cc.deleted",
                "chrome/c.mm.binary",
                "chrome/d.h.rename",
                "chrome/.gitignore",
            ],
        )
        self.assertEqual(
            patch_base_paths(patches),
            {"chrome/a.cc", "chrome/b.cc", "chrome/c.mm", "chrome/d.h"},
        )

    def test_missing_dir_is_empty(self):
        self.assertEqual(patch_base_paths(Path("/nonexistent/patches")), set())


class EntryResolutionTest(unittest.TestCase):
    def test_clean_tree_has_no_findings(self):
        patches = _patches_dir(self, ["chrome/a.cc", "third_party/lib/x.gn"])
        features = {
            "one": _feature(["chrome/a.cc"]),
            "two": _feature(["third_party/lib/"]),
        }
        self.assertEqual(check_repo(features, patches), [])

    def test_missing_file_entry_reported_with_feature_and_path(self):
        patches = _patches_dir(self, ["chrome/a.cc"])
        features = {"one": _feature(["chrome/a.cc", "chrome/gone.cc"])}
        findings = check_repo(features, patches)
        self.assertEqual(len(findings), 1)
        f = findings[0]
        self.assertEqual(
            (f.check, f.severity, f.feature, f.path),
            ("missing-patch", "error", "one", "chrome/gone.cc"),
        )

    def test_marker_variants_resolve_file_entries(self):
        patches = _patches_dir(
            self, ["chrome/a.cc.deleted", "chrome/b.mm.binary", "chrome/c.h.rename"]
        )
        features = {
            "one": _feature(["chrome/a.cc", "chrome/b.mm", "chrome/c.h"]),
        }
        self.assertEqual(check_repo(features, patches), [])

    def test_empty_directory_entry_reported(self):
        patches = _patches_dir(self, ["chrome/a.cc"])
        features = {
            "one": _feature(["chrome/a.cc"]),
            "two": _feature(["third_party/lib/"]),
        }
        findings = check_repo(features, patches)
        self.assertEqual(len(findings), 1)
        f = findings[0]
        self.assertEqual(
            (f.check, f.severity, f.feature, f.path),
            ("empty-dir", "error", "two", "third_party/lib/"),
        )

    def test_directory_prefix_does_not_match_sibling(self):
        patches = _patches_dir(self, ["chrome/subfoo/a.cc"])
        features = {
            "one": _feature(["chrome/sub/"]),
            "two": _feature(["chrome/subfoo/a.cc"]),
        }
        findings = check_repo(features, patches)
        self.assertEqual([f.check for f in findings], ["empty-dir"])
        self.assertEqual(findings[0].feature, "one")


class ClassificationTest(unittest.TestCase):
    def test_unclassified_patch_reported(self):
        patches = _patches_dir(self, ["chrome/a.cc", "chrome/orphan.cc"])
        features = {"one": _feature(["chrome/a.cc"])}
        findings = check_repo(features, patches)
        self.assertEqual(len(findings), 1)
        f = findings[0]
        self.assertEqual(
            (f.check, f.severity, f.feature, f.path),
            ("unclassified", "error", None, "chrome/orphan.cc"),
        )

    def test_unclaimed_marker_reported_under_base_path(self):
        patches = _patches_dir(self, ["chrome/gone.cc.deleted"])
        findings = check_repo({}, patches)
        self.assertEqual(len(findings), 1)
        self.assertEqual(findings[0].check, "unclassified")
        self.assertEqual(findings[0].path, "chrome/gone.cc")

    def test_multi_file_claim_is_warning_naming_all_claimants(self):
        patches = _patches_dir(self, ["chrome/a.cc"])
        features = {
            "one": _feature(["chrome/a.cc"]),
            "two": _feature(["chrome/a.cc"]),
        }
        findings = check_repo(features, patches)
        self.assertEqual(len(findings), 1)
        f = findings[0]
        self.assertEqual(
            (f.check, f.severity, f.path), ("multi-claim", "warning", "chrome/a.cc")
        )
        self.assertIn("one", f.message)
        self.assertIn("two", f.message)

    def test_dir_and_file_entry_overlap_is_multi_claim(self):
        patches = _patches_dir(self, ["chrome/sub/a.cc"])
        features = {
            "one": _feature(["chrome/sub/"]),
            "two": _feature(["chrome/sub/a.cc"]),
        }
        findings = check_repo(features, patches)
        self.assertEqual([f.check for f in findings], ["multi-claim"])

    def test_same_feature_claiming_via_file_and_dir_is_not_multi_claim(self):
        patches = _patches_dir(self, ["chrome/sub/a.cc"])
        features = {"one": _feature(["chrome/sub/", "chrome/sub/a.cc"])}
        self.assertEqual(check_repo(features, patches), [])


class SeriesExemptionTest(unittest.TestCase):
    def test_series_feature_entries_do_not_need_patches(self):
        patches = _patches_dir(self, ["chrome/a.cc"])
        features = {
            "one": _feature(["chrome/a.cc"]),
            "win": _feature(
                ["build/config.h", "chrome/app/x.rc"],
                description="series: windows platform patches",
            ),
        }
        self.assertEqual(check_repo(features, patches), [])

    def test_series_feature_does_not_claim_disk_patches(self):
        patches = _patches_dir(self, ["chrome/a.cc"])
        features = {
            "win": _feature(["chrome/a.cc"], description="series: windows"),
        }
        findings = check_repo(features, patches)
        self.assertEqual([f.check for f in findings], ["unclassified"])


class FeatureMetadataTest(unittest.TestCase):
    def test_invalid_name_and_description_reported(self):
        patches = _patches_dir(self, ["chrome/a.cc"])
        features = {
            "Bad Name": _feature(["chrome/a.cc"]),
            "two": _feature([], description="no prefix here"),
        }
        findings = check_repo(features, patches)
        checks = [
            (f.check, f.feature) for f in findings if f.check == "invalid-feature"
        ]
        self.assertIn(("invalid-feature", "Bad Name"), checks)
        self.assertIn(("invalid-feature", "two"), checks)

    def test_series_features_still_get_metadata_checks(self):
        patches = _patches_dir(self, [])
        features = {"BAD": _feature([], description="series: x")}
        findings = check_repo(features, patches)
        self.assertEqual([f.check for f in findings], ["invalid-feature"])

    def test_store_false_features_are_skipped(self):
        patches = _patches_dir(self, ["chrome/a.cc"])
        features = {
            "patches": _feature(["chrome/a.cc"]),
            "build-resources": {
                "store": False,
                "description": "resource: generated output",
                "files": ["chrome/generated.txt"],
            },
        }
        self.assertEqual(check_repo(features, patches), [])


class FeatureFilterTest(unittest.TestCase):
    def _fixture(self):
        patches = _patches_dir(self, ["chrome/a.cc", "chrome/orphan.cc"])
        features = {
            "one": _feature(["chrome/a.cc", "chrome/gone1.cc"]),
            "two": _feature(["chrome/a.cc", "chrome/gone2.cc"]),
        }
        return features, patches

    def test_filter_keeps_only_that_features_findings(self):
        features, patches = self._fixture()
        findings = check_repo(features, patches, feature="one")
        self.assertEqual(
            [(f.check, f.feature, f.path) for f in findings],
            [
                ("missing-patch", "one", "chrome/gone1.cc"),
                ("multi-claim", None, "chrome/a.cc"),
            ],
        )

    def test_filter_drops_unclassified(self):
        features, patches = self._fixture()
        findings = check_repo(features, patches, feature="two")
        self.assertNotIn("unclassified", [f.check for f in findings])

    def test_unknown_feature_raises(self):
        features, patches = self._fixture()
        with self.assertRaises(ValueError):
            check_repo(features, patches, feature="nope")

    def test_findings_sorted_deterministically(self):
        patches = _patches_dir(self, [])
        features = {
            "zed": _feature(["chrome/z.cc"]),
            "abc": _feature(["chrome/b.cc", "chrome/a.cc"]),
        }
        findings = check_repo(features, patches)
        self.assertEqual(
            [(f.feature, f.path) for f in findings],
            [
                ("abc", "chrome/a.cc"),
                ("abc", "chrome/b.cc"),
                ("zed", "chrome/z.cc"),
            ],
        )


def _fake_git(failing_fragments: set, calls: Optional[List] = None):
    """run_git_command stand-in failing patches whose path contains a fragment."""

    def fake(cmd, cwd, **kwargs):
        if calls is not None:
            calls.append((cmd, cwd))
        failed = any(fragment in cmd[-1] for fragment in failing_fragments)
        return SimpleNamespace(
            returncode=1 if failed else 0,
            stderr="error: patch failed\n" if failed else "",
        )

    return fake


class CheckApplyTest(unittest.TestCase):
    def test_failures_grouped_by_owning_feature(self):
        patches = _patches_dir(self, ["chrome/a.cc", "chrome/b.cc", "chrome/orphan.cc"])
        features = {"one": _feature(["chrome/a.cc", "chrome/b.cc"])}
        with mock.patch(
            "bos_build.patchkit.batch_apply.run_git_command",
            _fake_git({"b.cc", "orphan.cc"}),
        ):
            report = check_apply(features, patches, Path("/fake/src"))
        self.assertEqual((report.total, report.clean), (3, 1))
        self.assertEqual(
            {(f.patch, f.feature) for f in report.failures},
            {("chrome/b.cc", "one"), ("chrome/orphan.cc", "(unclassified)")},
        )
        self.assertEqual(report.features_affected, 2)
        self.assertEqual(report.against, "/fake/src")
        self.assertIn("patch failed", report.failures[0].error)

    def test_markers_excluded_from_apply_set(self):
        patches = _patches_dir(self, ["chrome/a.cc", "chrome/gone.cc.deleted"])
        features = {"one": _feature(["chrome/a.cc", "chrome/gone.cc"])}
        calls = []
        with mock.patch(
            "bos_build.patchkit.batch_apply.run_git_command", _fake_git(set(), calls)
        ):
            report = check_apply(features, patches, Path("/fake/src"))
        self.assertEqual((report.total, report.clean, report.failures), (1, 1, []))
        self.assertEqual(len(calls), 1)
        self.assertIn("chrome/a.cc", calls[0][0][-1])

    def test_feature_filter_limits_apply_set_to_claimed_patches(self):
        patches = _patches_dir(
            self, ["chrome/a.cc", "sub/dir/b.cc", "chrome/orphan.cc"]
        )
        features = {
            "one": _feature(["chrome/a.cc", "sub/dir/"]),
            "two": _feature(["chrome/orphan.cc"]),
        }
        calls = []
        with mock.patch(
            "bos_build.patchkit.batch_apply.run_git_command", _fake_git(set(), calls)
        ):
            report = check_apply(features, patches, Path("/fake/src"), feature="one")
        self.assertEqual(report.total, 2)
        ran = {call[0][-1] for call in calls}
        self.assertTrue(all("orphan" not in path for path in ran))

    def test_unknown_feature_raises(self):
        patches = _patches_dir(self, ["chrome/a.cc"])
        with self.assertRaises(ValueError):
            check_apply({}, patches, Path("/fake/src"), feature="nope")

    def test_filtered_failures_attributed_to_the_filtered_feature(self):
        patches = _patches_dir(self, ["chrome/a.cc"])
        features = {
            "aaa": _feature(["chrome/a.cc"]),
            "zzz": _feature(["chrome/a.cc"]),
        }
        with mock.patch(
            "bos_build.patchkit.batch_apply.run_git_command", _fake_git({"a.cc"})
        ):
            report = check_apply(features, patches, Path("/fake/src"), feature="zzz")
        self.assertEqual([f.feature for f in report.failures], ["zzz"])


class BuildReportTest(unittest.TestCase):
    def test_repo_only_report_shape_and_health(self):
        patches = _patches_dir(self, ["chrome/a.cc"])
        features = {"one": _feature(["chrome/a.cc"])}
        findings = check_repo(features, patches)

        report = build_report(patches.parent, features, findings)

        self.assertEqual(set(report), {"root", "feature", "repo", "apply", "healthy"})
        self.assertIsNone(report["apply"])
        self.assertIsNone(report["feature"])
        self.assertTrue(report["healthy"])
        self.assertEqual(report["repo"]["patches"], 1)
        self.assertEqual(report["repo"]["features"], 1)
        self.assertEqual((report["repo"]["errors"], report["repo"]["warnings"]), (0, 0))
        self.assertEqual(report["repo"]["findings"], [])

    def test_errors_flip_health_and_serialize_findings(self):
        patches = _patches_dir(self, ["chrome/a.cc"])
        features = {"one": _feature(["chrome/a.cc", "chrome/gone.cc"])}
        findings = check_repo(features, patches)

        report = build_report(patches.parent, features, findings)

        self.assertFalse(report["healthy"])
        self.assertEqual(report["repo"]["errors"], 1)
        finding = report["repo"]["findings"][0]
        self.assertEqual(
            set(finding), {"check", "severity", "message", "feature", "path"}
        )
        self.assertEqual(finding["check"], "missing-patch")

    def test_warnings_alone_stay_healthy(self):
        patches = _patches_dir(self, ["chrome/a.cc"])
        features = {
            "one": _feature(["chrome/a.cc"]),
            "two": _feature(["chrome/a.cc"]),
        }
        findings = check_repo(features, patches)

        report = build_report(patches.parent, features, findings)

        self.assertEqual(report["repo"]["warnings"], 1)
        self.assertTrue(report["healthy"])

    def test_apply_failures_flip_health_with_stable_keys(self):
        patches = _patches_dir(self, ["chrome/a.cc"])
        features = {"one": _feature(["chrome/a.cc"])}
        apply_report = ApplyReport(
            against="/src",
            total=3,
            clean=2,
            failures=[ApplyFailure("chrome/a.cc", "one", "boom")],
        )

        report = build_report(patches.parent, features, [], apply_report)

        self.assertFalse(report["healthy"])
        self.assertEqual(
            set(report["apply"]),
            {"against", "total", "clean", "failed", "features_affected", "failures"},
        )
        self.assertEqual(report["apply"]["failed"], 1)
        self.assertEqual(report["apply"]["features_affected"], 1)
        self.assertEqual(
            set(report["apply"]["failures"][0]), {"patch", "feature", "error"}
        )

    def test_clean_apply_report_stays_healthy(self):
        patches = _patches_dir(self, ["chrome/a.cc"])
        features = {"one": _feature(["chrome/a.cc"])}
        apply_report = ApplyReport(against="/src", total=1, clean=1, failures=[])

        report = build_report(patches.parent, features, [], apply_report)

        self.assertTrue(report["healthy"])
        self.assertEqual(report["apply"]["failed"], 0)

    def test_feature_filter_recorded_in_report(self):
        patches = _patches_dir(self, ["chrome/a.cc"])
        features = {"one": _feature(["chrome/a.cc"])}

        report = build_report(patches.parent, features, [], feature="one")

        self.assertEqual(report["feature"], "one")


class ComputeClaimsTest(unittest.TestCase):
    def test_claims_are_sorted_and_deduplicated(self):
        patches = _patches_dir(self, ["chrome/sub/a.cc"])
        bases = patch_base_paths(patches)
        features = {
            "zed": _feature(["chrome/sub/"]),
            "abc": _feature(["chrome/sub/a.cc", "chrome/sub/"]),
        }
        self.assertEqual(
            compute_claims(features, bases), {"chrome/sub/a.cc": ["abc", "zed"]}
        )


class LoadFeaturesTest(unittest.TestCase):
    def _root(self, features_yaml: str) -> Path:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        features_file = canonical_features_path(root)
        features_file.parent.mkdir(parents=True)
        features_file.write_text(features_yaml)
        return root

    def test_well_formed_file_loads(self):
        root = self._root(
            'version: "1.0"\nfeatures:\n  one:\n'
            '    description: "feat: x"\n    files:\n      - chrome/a.cc\n'
        )
        self.assertEqual(list(load_features(root)), ["one"])

    def test_feature_with_null_body_is_a_value_error(self):
        root = self._root("features:\n  foo:\n")
        with self.assertRaisesRegex(ValueError, "foo"):
            load_features(root)

    def test_non_mapping_top_level_is_a_value_error(self):
        root = self._root("- not\n- a\n- mapping\n")
        with self.assertRaisesRegex(ValueError, "mapping"):
            load_features(root)

    def test_non_string_file_entry_is_a_value_error(self):
        root = self._root(
            'features:\n  one:\n    description: "feat: x"\n'
            "    files:\n      - [nested, list]\n"
        )
        with self.assertRaisesRegex(ValueError, "list of paths"):
            load_features(root)

    def test_non_string_feature_key_is_a_value_error(self):
        # yaml parses a bare `on:` key as a boolean
        root = self._root('features:\n  on:\n    description: "feat: x"\n')
        with self.assertRaisesRegex(ValueError, "must be a string"):
            load_features(root)

    def test_store_false_features_are_filtered_after_shape_validation(self):
        root = self._root(
            'features:\n  one:\n    description: "feat: x"\n'
            "    files:\n      - chrome/a.cc\n"
            "  build-resources:\n    store: false\n"
            '    description: "resource: generated output"\n'
            "    files:\n      - chrome/generated.txt\n"
        )
        self.assertEqual(list(load_features(root)), ["one"])


class RepoTruthTest(unittest.TestCase):
    def test_repo_features_consistent_with_patches(self):
        errors = [
            finding
            for finding in diagnose_repo(get_package_root())
            if finding.severity == "error"
        ]
        self.assertEqual(
            errors, [], "\n" + "\n".join(finding.message for finding in errors)
        )


if __name__ == "__main__":
    unittest.main()
