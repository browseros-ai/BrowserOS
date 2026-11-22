#!/usr/bin/env python3
"""Component version planning and stamping tests."""

import json
import subprocess
import tempfile
import unittest
from pathlib import Path

from bos_build.release.components import (
    COMPONENTS,
    AllocationRecord,
    component_for_source,
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
                '    "apps/claw-app": {\n'
                '      "name": "@browseros/claw-app",\n'
                '      "version": "0.0.127",\n'
                "    },\n"
                "  },\n"
                "}\n"
            )

            before_neighbour = (
                '    "apps/claw-app": {\n'
                '      "name": "@browseros/claw-app",\n'
                '      "version": "0.0.127",\n'
                "    },"
            )
            changed = stamp_component(root, "agent", "0.0.101.0")

            self.assertEqual(changed, (app, lock))
            self.assertEqual(json.loads(app.read_text())["version"], "0.0.101")
            self.assertIn('"version": "0.0.101"', lock.read_text())
            self.assertIn(before_neighbour, lock.read_text())
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
            cargo = root / "packages/browseros-agent/apps/claw-server/Cargo.toml"
            lock = root / "packages/browseros-agent/Cargo.lock"
            cargo.parent.mkdir(parents=True)
            cargo.write_text(
                '[package]\nname = "claw-server"\nversion = "0.0.17"\n\n'
                '[dependencies]\nserde = "1"\n'
            )
            lock.parent.mkdir(parents=True, exist_ok=True)
            lock.write_text(
                '[[package]]\nname = "claw-api"\nversion = "1.0.0"\n\n'
                '[[package]]\nname = "claw-server"\nversion = "0.0.17"\n'
            )

            changed = stamp_component(root, "claw-server-rust", "0.0.18")

            self.assertEqual(changed, (cargo, lock))
            self.assertIn('version = "0.0.18"', cargo.read_text())
            self.assertIn('name = "claw-api"\nversion = "1.0.0"', lock.read_text())
            self.assertIn(
                'name = "claw-server"\nversion = "0.0.18"',
                lock.read_text(),
            )

    def test_server_component_stamps_its_crate_and_cargo_lock_entry(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            cargo = root / COMPONENTS["server"].manifest_path
            lock = root / COMPONENTS["server"].lockfile_path
            cargo.parent.mkdir(parents=True)
            cargo.write_text(
                '[package]\nname = "browseros-server"\nversion = "0.1.0"\n'
                'edition.workspace = true\n'
            )
            lock.write_text(
                '[[package]]\nname = "browseros-server"\nversion = "0.1.0"\n\n'
                '[[package]]\nname = "claw-server"\nversion = "0.1.0"\n'
            )

            self.assertEqual(stamp_component(root, "server", "0.1.1"), (cargo, lock))
            self.assertEqual(read_component_version(root, "server"), "0.1.1")
            self.assertIn(
                'name = "browseros-server"\nversion = "0.1.1"', lock.read_text()
            )
            self.assertIn('name = "claw-server"\nversion = "0.1.0"', lock.read_text())


class ComponentSourceTest(unittest.TestCase):
    def _git(self, root: Path, *args: str) -> str:
        return subprocess.run(
            ["git", *args], cwd=root, capture_output=True, text=True, check=True
        ).stdout.strip()

    def _commit(self, root: Path, message: str) -> str:
        self._git(root, "add", "-A")
        self._git(root, "commit", "-q", "-m", message)
        return self._git(root, "rev-parse", "HEAD")

    def test_server_resolves_the_layout_present_at_each_ref(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            self._git(root, "init", "-q", "-b", "main")
            self._git(root, "config", "user.name", "Component Test")
            self._git(root, "config", "user.email", "component@example.invalid")
            agent = root / "packages/browseros-agent"
            package = agent / "apps/server/package.json"
            package.parent.mkdir(parents=True)
            package.write_text('{"name": "@browseros/server", "version": "0.0.168"}\n')
            (agent / "bun.lock").write_text(
                '{\n  "workspaces": {\n    "apps/server": {\n'
                '      "name": "@browseros/server",\n'
                '      "version": "0.0.168",\n    },\n  },\n}\n'
            )
            typescript = self._commit(root, "TypeScript server")
            crate = root / COMPONENTS["server"].manifest_path
            crate.parent.mkdir(parents=True)
            crate.write_text('[package]\nname = "browseros-server"\nversion = "0.1.0"\n')
            (agent / "Cargo.lock").write_text(
                '[[package]]\nname = "browseros-server"\nversion = "0.1.0"\n'
            )
            both = self._commit(root, "Rust server beside TypeScript")
            package.unlink()
            rust = self._commit(root, "retire TypeScript server")

            legacy = component_for_source(root, "server", ref=typescript)
            self.assertEqual(
                (
                    legacy.manifest_path.as_posix(),
                    legacy.lockfile_path.as_posix(),
                    legacy.package_name,
                    legacy.workspace_path,
                ),
                (
                    "packages/browseros-agent/apps/server/package.json",
                    "packages/browseros-agent/bun.lock",
                    "@browseros/server",
                    "apps/server",
                ),
            )
            for ref in (both, rust, None):
                with self.subTest(ref=ref):
                    current = component_for_source(root, "server", ref=ref)
                    self.assertEqual(
                        (
                            current.manifest_path.as_posix(),
                            current.lockfile_path.as_posix(),
                            current.package_name,
                        ),
                        (
                            "packages/browseros-agent/crates/browseros-server/Cargo.toml",
                            "packages/browseros-agent/Cargo.lock",
                            "browseros-server",
                        ),
                    )
            self.assertEqual(read_component_version(root, "server"), "0.1.0")

    def test_server_checkout_without_the_crate_stamps_the_bun_workspace(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            package = root / "packages/browseros-agent/apps/server/package.json"
            lock = root / "packages/browseros-agent/bun.lock"
            package.parent.mkdir(parents=True)
            package.write_text('{"name": "@browseros/server", "version": "0.0.168"}\n')
            lock.write_text(
                '{\n  "workspaces": {\n    "apps/server": {\n'
                '      "name": "@browseros/server",\n'
                '      "version": "0.0.168",\n    },\n  },\n}\n'
            )

            self.assertEqual(stamp_component(root, "server", "0.0.169"), (package, lock))
            self.assertEqual(json.loads(package.read_text())["version"], "0.0.169")
            self.assertIn('"version": "0.0.169"', lock.read_text())


if __name__ == "__main__":
    unittest.main()
