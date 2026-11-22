#!/usr/bin/env python3
"""Target-aware local server resource tests."""

import hashlib
import json
import os
import shutil
import tempfile
import unittest
from pathlib import Path
from typing import Mapping
from unittest import mock

from bos_build.release.components import COMPONENTS
from bos_build.release.server_resources import (
    TARGETS,
    ServerResourceBuilder,
    load_resource_manifest,
    server_build_command,
    target_ids_for_lane,
    target_resource_rules,
    validate_server_resources,
)


SOURCE_SHA = "2" * 40
AGENT_SOURCE = Path(__file__).resolve().parents[3] / "browseros-agent"
BROWSEROS_RESOURCES = "scripts/build/config/server-prod-resources.json"


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

    def test_browseros_builds_the_server_crate_binary_for_each_target(self) -> None:
        root = Path("/repo/packages/browseros-agent")
        target_root = root / "target"
        cases = {
            "linux-x64": ("x86_64-unknown-linux-gnu", "browseros_server"),
            "windows-x64": ("x86_64-pc-windows-msvc", "browseros_server.exe"),
            "darwin-arm64": ("aarch64-apple-darwin", "browseros_server"),
            "darwin-x64": ("x86_64-apple-darwin", "browseros_server"),
        }
        for target, (triple, filename) in cases.items():
            with self.subTest(target=target):
                command, artifact = server_build_command(
                    "browseros", target, root, target_root
                )
                self.assertEqual(
                    command,
                    (
                        "cargo",
                        "build",
                        "--release",
                        "--locked",
                        "-p",
                        "browseros-server",
                        "--bin",
                        "browseros_server",
                        "--target",
                        triple,
                        "--manifest-path",
                        str(root / "Cargo.toml"),
                    ),
                )
                self.assertEqual(artifact, target_root / triple / "release" / filename)

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
                pairs = tuple(zip(command, command[1:]))
                self.assertIn(("--target", triple), pairs)
                self.assertIn(("-p", "claw-server"), pairs)
                self.assertIn(("--bin", "browseros-claw-server-rs"), pairs)
                self.assertEqual(artifact, target_root / triple / "release" / filename)

    def test_checked_in_manifests_place_the_files_each_product_ships(self) -> None:
        bun = {
            "linux-x64": "resources/bin/third_party/bun",
            "windows-x64": "resources/bin/third_party/bun.exe",
            "darwin-arm64": "resources/bin/third_party/bun",
            "darwin-x64": "resources/bin/third_party/bun",
        }
        for target_id, spec in TARGETS.items():
            with self.subTest(target=target_id):
                browseros = target_resource_rules("browseros", spec, AGENT_SOURCE)
                self.assertEqual(
                    [rule.destination for rule in browseros], [bun[target_id]]
                )
                self.assertTrue(browseros[0].executable)
                claw = target_resource_rules("browserclaw", spec, AGENT_SOURCE)
                self.assertEqual(
                    [rule.destination for rule in claw],
                    ["resources/skills/browserclaw/SKILL.md"],
                )

    def test_manifest_rejects_rules_the_packager_cannot_stage(self) -> None:
        cases = {
            "unknown os": {"os": ["freebsd"]},
            "recursive copy": {"recursive": True},
            "escaping destination": {"destination": "../outside"},
            "empty key": {"source": {"type": "r2", "key": ""}},
        }
        with tempfile.TemporaryDirectory() as temp_dir:
            for label, override in cases.items():
                with self.subTest(case=label):
                    rule = {
                        "name": "Bun",
                        "source": {"type": "r2", "key": "third_party/bun/bun"},
                        "destination": "resources/bin/third_party/bun",
                        **override,
                    }
                    path = Path(temp_dir) / "manifest.json"
                    path.write_text(json.dumps({"resources": [rule]}))
                    with self.assertRaises(ValueError):
                        load_resource_manifest(path)


class ServerResourceBuilderTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name)
        self.agent = self.repo / "packages/browseros-agent"
        self.browseros = self.repo / "packages/browseros"
        self.agent.mkdir(parents=True)
        self.browseros.mkdir(parents=True)
        server_manifest = self.repo / COMPONENTS["server"].manifest_path
        server_manifest.parent.mkdir(parents=True)
        server_manifest.write_text(
            '[package]\nname = "browseros-server"\nversion = "0.1.0"\n'
        )
        browseros_resources = self.agent / BROWSEROS_RESOURCES
        browseros_resources.parent.mkdir(parents=True)
        shutil.copy2(AGENT_SOURCE / BROWSEROS_RESOURCES, browseros_resources)
        claw_manifest = self.agent / "apps/claw-server/Cargo.toml"
        claw_manifest.parent.mkdir(parents=True)
        claw_manifest.write_text(
            '[package]\nname = "claw-server"\nversion = "0.0.18"\n'
        )
        skill = self.agent / "resources/skills/browserclaw/SKILL.md"
        skill.parent.mkdir(parents=True)
        skill.write_text("browserclaw skill")
        claw_resources = (
            self.agent / "scripts/build/config/claw-server-prod-resources.json"
        )
        claw_resources.write_text(
            json.dumps(
                {
                    "resources": [
                        {
                            "name": "BrowserOS neo skill",
                            "source": {
                                "type": "local",
                                "path": "resources/skills/browserclaw/SKILL.md",
                            },
                            "destination": "resources/skills/browserclaw/SKILL.md",
                        }
                    ]
                }
            )
        )

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

    def test_browserclaw_staging_matches_golden_metadata(self) -> None:
        # Literal bytes captured from the claw-only Cargo route before the
        # manifest-driven staging replaced it; the release zip and OTA digests
        # depend on them staying identical.
        target_root = self.agent / "target"

        def run(command, cwd, env):
            triple = command[command.index("--target") + 1]
            binary = target_root / triple / "release/browseros-claw-server-rs"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"rust-server")

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

        destination = result.destination
        self.assertEqual(
            (destination / "artifact-metadata.json").read_bytes(),
            b'{\n  "files": [\n    {\n'
            b'      "path": "resources/bin/browseros-claw-server",\n'
            b'      "sha256": '
            b'"da19f7e4fda25c492bdc690855da5c74e12b0dd29a28c9f7ffaff27a344b750d",\n'
            b'      "size": 11\n    },\n    {\n'
            b'      "path": "resources/skills/browserclaw/SKILL.md",\n'
            b'      "sha256": '
            b'"36ef038c94ab85fe582e3640698ec82d77ff30adac757b427aa0c44fd60c3b28",\n'
            b'      "size": 17\n    }\n  ],\n'
            b'  "sourceSha": "2222222222222222222222222222222222222222",\n'
            b'  "target": "darwin-arm64",\n'
            b'  "version": "0.0.18"\n}\n',
        )
        self.assertEqual(
            sorted(
                path.relative_to(destination).as_posix()
                for path in destination.rglob("*")
                if path.is_file()
            ),
            [
                "artifact-metadata.json",
                "resources/bin/browseros-claw-server",
                "resources/skills/browserclaw/SKILL.md",
            ],
        )
        self.assertEqual(
            result.manifest_sha256,
            "5381957654f89702741a91c82782d334210dfe242d18a1e5f118ecafb140708c",
        )

    def _browseros_builder(self, host: str, triple: str, downloads: list[str]):
        target_root = self.agent / "target"

        def run(command, cwd, env):
            binary = target_root / triple / "release" / command[
                command.index("--bin") + 1
            ]
            if host == "windows":
                binary = binary.with_name(binary.name + ".exe")
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"rust-server")

        def download(key, destination):
            downloads.append(key)
            destination.write_bytes(b"bun")
            return True

        return ServerResourceBuilder(
            self.repo,
            host_platform=host,
            run=run,
            which=lambda name: f"/usr/bin/{name}",
            rust_targets=lambda: {triple},
            download_r2=download,
            r2_configured=lambda: True,
            cargo_target_dir=target_root,
        )

    def test_browseros_stages_server_then_vendored_bun_from_r2(self) -> None:
        downloads: list[str] = []
        builder = self._browseros_builder(
            "linux", "x86_64-unknown-linux-gnu", downloads
        )

        result = builder.prepare(
            product="browseros",
            target="linux-x64",
            version="0.1.0",
            source_sha=SOURCE_SHA,
        )

        self.assertEqual(
            downloads, ["artifacts/vendor/third_party/bun/bun-linux-x64-baseline"]
        )
        runtime = result.destination / "resources/bin/browseros_server"
        bun = result.destination / "resources/bin/third_party/bun"
        self.assertEqual(runtime.read_bytes(), b"rust-server")
        self.assertEqual(bun.read_bytes(), b"bun")
        if os.name != "nt":
            self.assertEqual(bun.stat().st_mode & 0o777, 0o755)
        metadata = json.loads((result.destination / "artifact-metadata.json").read_text())
        self.assertEqual(
            [entry["path"] for entry in metadata["files"]],
            ["resources/bin/browseros_server", "resources/bin/third_party/bun"],
        )
        self.assertEqual(metadata["version"], "0.1.0")
        self.assertEqual(metadata["sourceSha"], SOURCE_SHA)

    def test_browseros_windows_stages_exe_names(self) -> None:
        downloads: list[str] = []
        builder = self._browseros_builder("windows", "x86_64-pc-windows-msvc", downloads)

        result = builder.prepare(
            product="browseros",
            target="windows-x64",
            version="0.1.0",
            source_sha=SOURCE_SHA,
        )

        self.assertEqual(
            downloads,
            ["artifacts/vendor/third_party/bun/bun-windows-x64-baseline.exe"],
        )
        self.assertTrue((result.destination / "resources/bin/browseros_server.exe").is_file())
        self.assertTrue(
            (result.destination / "resources/bin/third_party/bun.exe").is_file()
        )

    def test_only_browseros_cargo_links_the_windows_crt_statically(self) -> None:
        key = "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS"
        target_root = self.agent / "target"
        seen: dict[str, Mapping[str, str]] = {}

        def run(command, cwd, env):
            product = "browseros" if "browseros_server" in command else "browserclaw"
            seen[product] = env
            triple = command[command.index("--target") + 1]
            name = command[command.index("--bin") + 1] + ".exe"
            binary = target_root / triple / "release" / name
            binary.parent.mkdir(parents=True, exist_ok=True)
            binary.write_bytes(b"rust-server")

        builder = ServerResourceBuilder(
            self.repo,
            host_platform="windows",
            run=run,
            which=lambda name: f"/usr/bin/{name}",
            rust_targets=lambda: {"x86_64-pc-windows-msvc"},
            download_r2=lambda key, destination: destination.write_bytes(b"bun") > 0,
            r2_configured=lambda: True,
            cargo_target_dir=target_root,
        )
        with mock.patch.dict(os.environ, {}, clear=False):
            os.environ.pop(key, None)
            for product in ("browseros", "browserclaw"):
                builder.prepare(
                    product=product,
                    target="windows-x64",
                    version="0.1.0" if product == "browseros" else "0.0.18",
                    source_sha=SOURCE_SHA,
                )

        self.assertEqual(seen["browseros"][key], "-C target-feature=+crt-static")
        self.assertNotIn(key, seen["browserclaw"])

    def test_failed_r2_download_stops_staging(self) -> None:
        builder = self._browseros_builder("linux", "x86_64-unknown-linux-gnu", [])
        builder.download_r2 = lambda key, destination: False

        with self.assertRaisesRegex(RuntimeError, "Could not download Bun"):
            builder.prepare(
                product="browseros",
                target="linux-x64",
                version="0.1.0",
                source_sha=SOURCE_SHA,
            )

    def test_browseros_preflight_requires_r2_and_browserclaw_does_not(self) -> None:
        builder = ServerResourceBuilder(
            self.repo,
            host_platform="linux",
            run=lambda *args: None,
            which=lambda name: f"/usr/bin/{name}",
            rust_targets=lambda: {"x86_64-unknown-linux-gnu"},
            r2_configured=lambda: False,
        )

        with self.assertRaisesRegex(RuntimeError, "R2_ACCOUNT_ID"):
            builder.preflight(product="browseros", target="linux-x64")
        builder.preflight(product="browserclaw", target="linux-x64")

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
        with self.assertRaisesRegex(RuntimeError, "install Rust and Cargo"):
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

    def _staged_browseros(self, files: dict[str, bytes]) -> Path:
        destination = self.browseros / "resources/binaries/browseros_server/linux-x64"
        for relative, content in files.items():
            path = destination / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
            path.chmod(0o755)
        document = _metadata("0.1.0", "linux-x64", files)
        document["sourceSha"] = SOURCE_SHA
        (destination / "artifact-metadata.json").write_text(json.dumps(document))
        return destination

    def _validate(self, destination: Path) -> None:
        validate_server_resources(
            destination,
            product="browseros",
            target="linux-x64",
            version="0.1.0",
            source_sha=SOURCE_SHA,
            agent_root=self.agent,
        )

    def test_validation_rejects_version_source_and_checksum_drift(self) -> None:
        destination = self._staged_browseros(
            {
                "resources/bin/browseros_server": b"server",
                "resources/bin/third_party/bun": b"bun",
            }
        )

        self._validate(destination)
        (destination / "resources/bin/browseros_server").write_bytes(b"tamper")
        with self.assertRaisesRegex(ValueError, "checksum"):
            self._validate(destination)

    def test_validation_requires_every_manifest_file_for_the_target(self) -> None:
        destination = self._staged_browseros(
            {"resources/bin/browseros_server": b"server"}
        )

        with self.assertRaisesRegex(ValueError, "resources/bin/third_party/bun"):
            self._validate(destination)


if __name__ == "__main__":
    unittest.main()
