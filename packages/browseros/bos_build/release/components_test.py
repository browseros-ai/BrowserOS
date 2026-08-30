#!/usr/bin/env python3
"""Component version planning and stamping tests."""

import json
import tempfile
import unittest
from pathlib import Path

from bos_build.release.components import (
    AllocationRecord,
    read_component_version,
    components_for_candidate,
    resolve_candidate_versions,
    resolve_standalone_version,
    stamp_component,
)


class ComponentPlanningTest(unittest.TestCase):
    def test_product_candidates_select_only_versioned_components(self) -> None:
        self.assertEqual(
            tuple(component.id for component in components_for_candidate("browseros")),
            ("server", "agent"),
        )
        self.assertEqual(
            tuple(
                component.id for component in components_for_candidate("browserclaw")
            ),
            ("claw-server-rust", "browserclaw"),
        )

    def test_candidate_advances_once_and_skips_reservations(self) -> None:
        allocations = (
            AllocationRecord(
                component="server",
                version="0.0.128",
                kind="tag",
            ),
            AllocationRecord(
                component="agent",
                version="0.0.101.0",
                kind="candidate",
                candidate_id="other",
            ),
        )

        planned = resolve_candidate_versions(
            product_id="browseros",
            committed_versions={"server": "0.0.127", "agent": "0.0.100"},
            allocations=allocations,
            candidate_id="candidate-1",
        )

        self.assertEqual(planned, {"server": "0.0.129", "agent": "0.0.102.0"})

    def test_candidate_retry_reuses_every_reserved_version(self) -> None:
        allocations = (
            AllocationRecord(
                component="server",
                version="0.0.128",
                kind="candidate",
                candidate_id="candidate-1",
            ),
            AllocationRecord(
                component="agent",
                version="0.0.101.0",
                kind="candidate",
                candidate_id="candidate-1",
            ),
        )

        planned = resolve_candidate_versions(
            product_id="browseros",
            committed_versions={"server": "0.0.127", "agent": "0.0.100"},
            allocations=allocations,
            candidate_id="candidate-1",
        )

        self.assertEqual(planned, {"server": "0.0.128", "agent": "0.0.101.0"})

    def test_candidate_advances_beyond_newer_release_history(self) -> None:
        planned = resolve_candidate_versions(
            product_id="browseros",
            committed_versions={"server": "0.0.127", "agent": "0.0.100"},
            allocations=(
                AllocationRecord(
                    component="server",
                    version="0.0.200",
                    kind="release",
                    public=True,
                ),
                AllocationRecord(
                    component="agent",
                    version="0.0.150.0",
                    kind="release",
                    public=True,
                ),
            ),
            candidate_id="candidate-1",
        )

        self.assertEqual(planned, {"server": "0.0.201", "agent": "0.0.151.0"})

    def test_incomplete_candidate_reservation_fails_closed(self) -> None:
        allocations = (
            AllocationRecord(
                component="server",
                version="0.0.128",
                kind="candidate",
                candidate_id="candidate-1",
            ),
        )

        with self.assertRaisesRegex(ValueError, "incomplete"):
            resolve_candidate_versions(
                product_id="browseros",
                committed_versions={"server": "0.0.127", "agent": "0.0.100"},
                allocations=allocations,
                candidate_id="candidate-1",
            )

    def test_standalone_prefers_committed_unpublished_version(self) -> None:
        self.assertEqual(
            resolve_standalone_version(
                component_id="server",
                committed_version="0.0.127",
                allocations=(),
            ),
            "0.0.127",
        )
        self.assertEqual(
            resolve_standalone_version(
                component_id="server",
                committed_version="0.0.127",
                allocations=(
                    AllocationRecord(component="server", version="0.0.127", kind="tag"),
                ),
            ),
            "0.0.128",
        )

    def test_standalone_treats_open_candidates_as_temporary_reservations(self) -> None:
        candidate = AllocationRecord(
            component="server",
            version="0.0.127",
            kind="candidate",
            candidate_id="bot/release-browseros",
        )
        self.assertEqual(
            resolve_standalone_version(
                component_id="server",
                committed_version="0.0.127",
                allocations=(candidate,),
            ),
            "0.0.128",
        )
        self.assertEqual(
            resolve_standalone_version(
                component_id="server",
                committed_version="0.0.127",
                allocations=(),
            ),
            "0.0.127",
        )

    def test_standalone_does_not_reuse_a_draft_with_conflicting_resources(self) -> None:
        allocations = (
            AllocationRecord(
                component="server",
                version="0.0.128",
                kind="release",
                source_sha="1" * 40,
                reference="agent-server/v0.0.128",
                reusable=True,
            ),
            AllocationRecord(
                component="server",
                version="0.0.128",
                kind="resource",
                reference="r2://browseros/artifacts/server/0.0.128",
            ),
        )

        self.assertEqual(
            resolve_standalone_version(
                component_id="server",
                committed_version="0.0.127",
                allocations=allocations,
                source_sha="1" * 40,
            ),
            "0.0.129",
        )

    def test_standalone_reuses_a_draft_with_matching_resources(self) -> None:
        allocations = (
            AllocationRecord(
                component="server",
                version="0.0.128",
                kind="release",
                source_sha="1" * 40,
                reference="agent-server/v0.0.128",
                reusable=True,
            ),
            AllocationRecord(
                component="server",
                version="0.0.128",
                kind="resource",
                source_sha="1" * 40,
                reference="agent-server/v0.0.128",
                reusable=True,
            ),
        )

        self.assertEqual(
            resolve_standalone_version(
                component_id="server",
                committed_version="0.0.127",
                allocations=allocations,
                source_sha="1" * 40,
            ),
            "0.0.128",
        )

    def test_closed_suite_ownership_vetoes_same_source_draft_reuse(self) -> None:
        allocations = (
            AllocationRecord(
                component="server",
                version="0.0.128",
                kind="release",
                source_sha="1" * 40,
                reference="agent-server/v0.0.128",
                reusable=True,
            ),
            AllocationRecord(
                component="server",
                version="0.0.128",
                kind="candidate",
                source_sha="1" * 40,
                reference="agent-server/v0.0.128",
                reuse_forbidden=True,
            ),
        )

        with self.assertRaisesRegex(ValueError, "already allocated"):
            resolve_standalone_version(
                component_id="server",
                committed_version="0.0.127",
                allocations=allocations,
                source_sha="1" * 40,
                requested_version="0.0.128",
            )
        self.assertEqual(
            resolve_standalone_version(
                component_id="server",
                committed_version="0.0.127",
                allocations=allocations,
                source_sha="1" * 40,
            ),
            "0.0.129",
        )

    def test_standalone_rejects_versions_older_than_public_history(self) -> None:
        allocations = (
            AllocationRecord(
                component="server",
                version="0.0.130",
                kind="tag",
                public=True,
            ),
        )
        with self.assertRaisesRegex(ValueError, "older than newest public"):
            resolve_standalone_version(
                component_id="server",
                committed_version="0.0.127",
                allocations=allocations,
                requested_version="0.0.129",
            )


class ComponentStampingTest(unittest.TestCase):
    def test_json_component_updates_only_its_manifest_and_workspace_entry(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            app = root / "packages/browseros-agent/apps/app/package.json"
            lock = root / "packages/browseros-agent/bun.lock"
            app.parent.mkdir(parents=True)
            app.write_text(
                json.dumps({"name": "@browseros/app", "version": "0.0.100"}, indent=2)
                + "\n"
            )
            lock.parent.mkdir(parents=True, exist_ok=True)
            lock.write_text(
                "{\n"
                '  "workspaces": {\n'
                '    "apps/app": {\n'
                '      "name": "@browseros/app",\n'
                '      "version": "0.0.100",\n'
                "    },\n"
                '    "apps/server": {\n'
                '      "name": "@browseros/server",\n'
                '      "version": "0.0.127",\n'
                "    },\n"
                "  },\n"
                "}\n"
            )

            before_server = (
                '    "apps/server": {\n'
                '      "name": "@browseros/server",\n'
                '      "version": "0.0.127",\n'
                "    },"
            )
            changed = stamp_component(root, "agent", "0.0.101.0")

            self.assertEqual(changed, (app, lock))
            self.assertEqual(json.loads(app.read_text())["version"], "0.0.101")
            self.assertIn('"version": "0.0.101"', lock.read_text())
            self.assertIn(before_server, lock.read_text())
            self.assertEqual(read_component_version(root, "agent"), "0.0.101.0")

    def test_chrome_build_component_uses_semver_build_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            app = root / "packages/browseros-agent/apps/app/package.json"
            lock = root / "packages/browseros-agent/bun.lock"
            app.parent.mkdir(parents=True)
            app.write_text(json.dumps({"version": "0.0.100"}))
            lock.parent.mkdir(parents=True, exist_ok=True)
            lock.write_text(
                "{\n"
                '  "workspaces": {\n'
                '    "apps/app": {\n'
                '      "version": "0.0.100",\n'
                "    },\n"
                "  },\n"
                "}\n"
            )

            stamp_component(root, "agent", "0.0.101.7")

            self.assertEqual(json.loads(app.read_text())["version"], "0.0.101+7")
            self.assertIn('"version": "0.0.101+7"', lock.read_text())
            self.assertEqual(read_component_version(root, "agent"), "0.0.101.7")

    def test_cargo_component_updates_only_matching_package_entries(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            cargo = root / "packages/browseros-agent/apps/claw-server-rust/Cargo.toml"
            lock = root / "packages/browseros-agent/Cargo.lock"
            cargo.parent.mkdir(parents=True)
            cargo.write_text(
                '[package]\nname = "claw-server-rust"\nversion = "0.0.17"\n\n'
                '[dependencies]\nserde = "1"\n'
            )
            lock.parent.mkdir(parents=True, exist_ok=True)
            lock.write_text(
                '[[package]]\nname = "claw-api"\nversion = "1.0.0"\n\n'
                '[[package]]\nname = "claw-server-rust"\nversion = "0.0.17"\n'
            )

            changed = stamp_component(root, "claw-server-rust", "0.0.18")

            self.assertEqual(changed, (cargo, lock))
            self.assertIn('version = "0.0.18"', cargo.read_text())
            self.assertIn('name = "claw-api"\nversion = "1.0.0"', lock.read_text())
            self.assertIn(
                'name = "claw-server-rust"\nversion = "0.0.18"',
                lock.read_text(),
            )


if __name__ == "__main__":
    unittest.main()
