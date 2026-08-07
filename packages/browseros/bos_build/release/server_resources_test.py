#!/usr/bin/env python3
"""Target-aware local server resource tests."""

import hashlib
import json
import os
import tempfile
import unittest
from pathlib import Path

from bos_build.release.server_resources import (
    ServerResourceBuilder,
    server_build_command,
    target_ids_for_lane,
    validate_server_resources,
)


SOURCE_SHA = "2" * 40


def _metadata(version: str, target: str, files: dict[str, bytes]) -> dict:
    return {
        "version": version,
        "target": target,
        "files": [
            {
                "path": path,
                "size": len(content),
                "sha256": hashlib.sha256(content).hexdigest(),
            }
            for path, content in sorted(files.items())
        ],
    }


class ServerTargetTest(unittest.TestCase):
    def test_lane_targets_cover_release_matrix_and_universal_macos(self) -> None:
        self.assertEqual(target_ids_for_lane("linux", "x64"), ("linux-x64",))
        self.assertEqual(target_ids_for_lane("windows", "x64"), ("windows-x64",))
        self.assertEqual(
            target_ids_for_lane("macos", "universal"),
            ("darwin-arm64", "darwin-x64"),
        )

    def test_browseros_delegates_each_target_to_bun_builder(self) -> None:
        root = Path("/repo/packages/browseros-agent")
        for target in (
            "linux-x64",
            "windows-x64",
            "darwin-arm64",
            "darwin-x64",
        ):
            with self.subTest(target=target):
                command, artifact = server_build_command(
                    "browseros", target, root, root / "target"
                )
                self.assertEqual(
                    command,
                    (
                        "bun",
                        "scripts/build/server.ts",
                        f"--target={target}",
                        "--no-upload",
                    ),
                )
                self.assertEqual(artifact, root / "dist/prod/server" / target)

    def test_browserclaw_maps_targets_to_cargo_triples_and_runtime_names(self) -> None:
        root = Path("/repo/packages/browseros-agent")
        target_root = root / "target"
        cases = {
            "linux-x64": ("x86_64-unknown-linux-gnu", "browseros-claw-server-rs"),
            "windows-x64": (
                "x86_64-pc-windows-msvc",
                "browseros-claw-server-rs.exe",
            ),
            "darwin-arm64": ("aarch64-apple-darwin", "browseros-claw-server-rs"),
            "darwin-x64": ("x86_64-apple-darwin", "browseros-claw-server-rs"),
        }
        for target, (triple, filename) in cases.items():
            with self.subTest(target=target):
                command, artifact = server_build_command(
                    "browserclaw", target, root, target_root
                )
                self.assertIn(("--target", triple), tuple(zip(command, command[1:])))
                self.assertEqual(artifact, target_root / triple / "release" / filename)


class ServerResourceBuilderTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name)
        self.agent = self.repo / "packages/browseros-agent"
        self.browseros = self.repo / "packages/browseros"
        self.agent.mkdir(parents=True)
        self.browseros.mkdir(parents=True)
        server_manifest = self.agent / "apps/server/package.json"
        server_manifest.parent.mkdir(parents=True)
        server_manifest.write_text('{"version":"0.0.128"}\n')
        claw_manifest = self.agent / "apps/claw-server-rust/Cargo.toml"
        claw_manifest.parent.mkdir(parents=True)
        claw_manifest.write_text(
            '[package]\nname = "claw-server-rust"\nversion = "0.0.18"\n'
        )
        skill = self.agent / "resources/skills/browserclaw/SKILL.md"
        skill.parent.mkdir(parents=True)
        skill.write_text("browserclaw skill")

    def test_rust_build_stages_runtime_name_skill_metadata_and_clears_stale(self) -> None:
        target_root = self.agent / "target"

        def run(command, cwd, env):
            triple = command[command.index("--target") + 1]
            binary = target_root / triple / "release/browseros-claw-server-rs"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"rust-server")

        destination = (
            self.browseros
            / "resources/binaries/browseros_claw_server_rust/darwin-arm64"
        )
        destination.mkdir(parents=True)
        (destination / "stale").write_text("stale")
        builder = ServerResourceBuilder(
            self.repo,
            host_platform="macos",
            run=run,
            which=lambda name: f"/usr/bin/{name}",
            rust_targets=lambda: {"aarch64-apple-darwin"},
            cargo_target_dir=target_root,
        )

        result = builder.prepare(
            product="browserclaw",
            target="darwin-arm64",
            version="0.0.18",
            source_sha=SOURCE_SHA,
        )

        runtime = destination / "resources/bin/browseros-claw-server"
        self.assertEqual(runtime.read_bytes(), b"rust-server")
        self.assertFalse((destination / "resources/bin/browseros-claw-server-rs").exists())
        self.assertEqual(
            (destination / "resources/skills/browserclaw/SKILL.md").read_text(),
            "browserclaw skill",
        )
        self.assertFalse((destination / "stale").exists())
        metadata = json.loads((destination / "artifact-metadata.json").read_text())
        self.assertEqual(metadata["version"], "0.0.18")
        self.assertEqual(metadata["sourceSha"], SOURCE_SHA)
        self.assertEqual(result.destination, destination.resolve())
        if os.name != "nt":
            self.assertTrue(runtime.stat().st_mode & 0o100)

    def test_bun_build_validates_builder_artifact_and_stages_current_target(self) -> None:
        output = self.agent / "dist/prod/server/linux-x64"
        files = {"resources/bin/browseros_server": b"bun-server"}

        def run(command, cwd, env):
            for relative, content in files.items():
                path = output / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(content)
            (output / "artifact-metadata.json").write_text(
                json.dumps(_metadata("0.0.128", "linux-x64", files))
            )

        builder = ServerResourceBuilder(
            self.repo,
            host_platform="linux",
            run=run,
            which=lambda name: f"/usr/bin/{name}",
        )
        result = builder.prepare(
            product="browseros",
            target="linux-x64",
            version="0.0.128",
            source_sha=SOURCE_SHA,
        )

        self.assertEqual(
            (result.destination / "resources/bin/browseros_server").read_bytes(),
            b"bun-server",
        )
        self.assertEqual(
            json.loads((result.destination / "artifact-metadata.json").read_text())[
                "sourceSha"
            ],
            SOURCE_SHA,
        )

    def test_incompatible_host_and_missing_toolchain_fail_preflight(self) -> None:
        builder = ServerResourceBuilder(
            self.repo,
            host_platform="linux",
            run=lambda *args: None,
            which=lambda name: None,
        )
        with self.assertRaisesRegex(RuntimeError, "darwin-arm64.*macos"):
            builder.prepare(
                product="browserclaw",
                target="darwin-arm64",
                version="0.0.18",
                source_sha=SOURCE_SHA,
            )
        with self.assertRaisesRegex(RuntimeError, "install Bun"):
            builder.preflight(product="browseros", target="linux-x64")

    def test_source_manifest_must_match_requested_version(self) -> None:
        builder = ServerResourceBuilder(
            self.repo,
            host_platform="macos",
            run=lambda *args: None,
            which=lambda name: f"/usr/bin/{name}",
            rust_targets=lambda: {"aarch64-apple-darwin"},
        )

        with self.assertRaisesRegex(ValueError, "source version"):
            builder.prepare(
                product="browserclaw",
                target="darwin-arm64",
                version="0.0.19",
                source_sha=SOURCE_SHA,
            )

    def test_missing_rust_target_fails_preflight_with_install_command(self) -> None:
        builder = ServerResourceBuilder(
            self.repo,
            host_platform="linux",
            run=lambda *args: None,
            which=lambda name: f"/usr/bin/{name}",
            rust_targets=lambda: set(),
        )

        with self.assertRaisesRegex(
            RuntimeError, "rustup target add x86_64-unknown-linux-gnu"
        ):
            builder.preflight(product="browserclaw", target="linux-x64")

    def test_validation_rejects_version_source_and_checksum_drift(self) -> None:
        destination = self.browseros / "resources/binaries/browseros_server/linux-x64"
        binary = destination / "resources/bin/browseros_server"
        binary.parent.mkdir(parents=True)
        binary.write_bytes(b"server")
        binary.chmod(0o755)
        document = _metadata(
            "0.0.128", "linux-x64", {"resources/bin/browseros_server": b"server"}
        )
        document["sourceSha"] = SOURCE_SHA
        (destination / "artifact-metadata.json").write_text(json.dumps(document))

        validate_server_resources(
            destination,
            product="browseros",
            target="linux-x64",
            version="0.0.128",
            source_sha=SOURCE_SHA,
        )
        binary.write_bytes(b"tamper")
        with self.assertRaisesRegex(ValueError, "checksum"):
            validate_server_resources(
                destination,
                product="browseros",
                target="linux-x64",
                version="0.0.128",
                source_sha=SOURCE_SHA,
            )


if __name__ == "__main__":
    unittest.main()
