#!/usr/bin/env python3
"""Tests for immutable full-release plans."""

import io
import json
import tempfile
import unittest
from pathlib import Path

from .feeds.render import extract_enclosure_urls, extract_manifest_versions
from .feeds.spec import EXTENSIONS, extension_by_name
from .plan import Component, PlanResult, _write_github_output, create_release_plan
from .resource_pins import RESOURCE_FAMILIES, ResourceObjectPin, ResourcePin


SOURCE_SHA = "a" * 40


class PreconditionFailedError(Exception):
    def __init__(self) -> None:
        self.response = {
            "Error": {"Code": "PreconditionFailed"},
            "ResponseMetadata": {"HTTPStatusCode": 412},
        }


class ConditionalConflictError(Exception):
    def __init__(self) -> None:
        self.response = {
            "Error": {"Code": "ConditionalRequestConflict"},
            "ResponseMetadata": {"HTTPStatusCode": 409},
        }


class FakeR2Client:
    def __init__(self) -> None:
        self.objects: dict[str, bytes] = {}
        self.metadata: dict[str, dict[str, str]] = {}
        self.puts: list[dict[str, object]] = []
        self.gets: list[str] = []

    def put_object(self, **kwargs):
        self.puts.append(kwargs)
        key = str(kwargs["Key"])
        if key in self.objects:
            raise PreconditionFailedError()
        self.objects[key] = bytes(kwargs["Body"])
        self.metadata[key] = dict(kwargs["Metadata"])

    def get_object(self, *, Bucket, Key):
        del Bucket
        self.gets.append(Key)
        return {
            "Body": io.BytesIO(self.objects[Key]),
            "Metadata": self.metadata[Key],
        }


class ConflictR2Client(FakeR2Client):
    def put_object(self, **kwargs):
        self.puts.append(kwargs)
        raise ConditionalConflictError()


def bundled_manifest(versions: dict[str, str] | None = None) -> str:
    selected = versions or {
        "agent": "0.0.123.0",
        "bugreporter": "54.0.0.0",
        "browserclaw": "0.1.7.0",
    }
    lines = [
        "<?xml version='1.0' encoding='UTF-8'?>",
        '<gupdate xmlns="http://www.google.com/update2/response" protocol="2.0">',
    ]
    for name in ("bugreporter", "agent", "browserclaw"):
        spec = extension_by_name(name)
        version = selected[name]
        lines.extend(
            [
                f'  <app appid="{spec.extension_id}">',
                f'    <updatecheck codebase="{spec.crx_url(version)}" version="{version}"/>',
                "  </app>",
            ]
        )
    lines.append("</gupdate>")
    return "\n".join(lines) + "\n"


def resource_pins(components: list[Component]) -> dict[str, ResourcePin]:
    selected = {
        {
            "browseros-server": "browseros_server",
            "browserclaw-server": "browserclaw_server",
            "browserclaw-onboard": "browserclaw_onboard",
        }[component.name]: component
        for component in components
        if component.name
        in {"browseros-server", "browserclaw-server", "browserclaw-onboard"}
    }
    pins = {}
    for index, family in enumerate(RESOURCE_FAMILIES, start=1):
        component = selected.get(family.name)
        version = component.version if component else f"0.0.{index}"
        pins[family.name] = ResourcePin(
            family.name,
            version,
            tuple(
                ResourceObjectPin(
                    target=target,
                    key=family.version_key(version, target),
                    etag=f'"{family.name}-{target}"',
                    size=index,
                    sha256="",
                    release_sha=component.source_sha if component else "",
                )
                for target in family.targets
            ),
        )
    return pins


class ReleasePlanTest(unittest.TestCase):
    def setUp(self) -> None:
        self.client = FakeR2Client()
        self.probed: list[str] = []

    def probe(self, url: str) -> int:
        self.probed.append(url)
        return 200

    def create(
        self,
        root: Path,
        *,
        product: str = "browseros",
        components: list[Component] | None = None,
        extension_name: str = "agent",
        manifest: str | None = None,
        source_sha: str = SOURCE_SHA,
        run_id: str = "30418029456",
        run_attempt: str = "2",
        cdn_base_url: str = "https://cdn.browseros.com",
        include_manifest: bool = True,
    ):
        selected_components = components or []
        manifest_path = None
        if include_manifest:
            manifest_path = root / "input-bundled-manifest.xml"
            manifest_path.write_text(manifest or bundled_manifest(), encoding="utf-8")
        return create_release_plan(
            product=product,
            browser_version="0.49.0",
            source_sha=source_sha,
            run_id=run_id,
            run_attempt=run_attempt,
            components=selected_components,
            extension_name=extension_name if manifest is not None else "",
            bundled_manifest_path=manifest_path,
            output_dir=root / "output",
            client=self.client,
            bucket="browseros",
            cdn_base_url=cdn_base_url,
            http_head=self.probe,
            resource_pins=resource_pins(selected_components),
        )

    def test_selected_extension_preserves_other_pins_and_overrides_one(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            result = self.create(
                Path(temp_dir),
                components=[
                    Component(
                        "browseros-server", "0.0.22", "agent-server/v0.0.22", SOURCE_SHA
                    ),
                    Component(
                        "agent-extension",
                        "0.0.124.0",
                        "ext-agent/v0.0.124.0",
                        SOURCE_SHA,
                    ),
                ],
                manifest=bundled_manifest(),
            )

            manifest_bytes = result.manifest_path.read_bytes()
            versions = extract_manifest_versions(manifest_bytes.decode())
            self.assertEqual(
                versions,
                {
                    extension_by_name("agent").extension_id: "0.0.124.0",
                    extension_by_name("bugreporter").extension_id: "54.0.0.0",
                    extension_by_name("browserclaw").extension_id: "0.1.7.0",
                },
            )
            urls = extract_enclosure_urls(manifest_bytes.decode())
            self.assertIn(
                "https://cdn.browseros.com/extensions/agent-0.0.124.0.crx",
                urls,
            )
            self.assertNotIn("latest", result.manifest_url)
            self.assertEqual(self.client.objects[result.manifest_key], manifest_bytes)
            self.assertIn(result.manifest_url, self.probed)

    def test_skipped_extension_emits_an_immutable_manifest_snapshot(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            result = self.create(
                Path(temp_dir),
                components=[
                    Component(
                        "browseros-server", "0.0.22", "agent-server/v0.0.22", SOURCE_SHA
                    )
                ],
            )

            self.assertNotIn("latest", result.manifest_url)
            self.assertTrue(result.manifest_key.endswith("/bundled-manifest.xml"))
            self.assertIsNotNone(result.manifest_path)
            self.assertEqual(
                extract_manifest_versions(result.manifest_path.read_text()),
                extract_manifest_versions(bundled_manifest()),
            )
            self.assertEqual(len(self.client.objects), 2)
            for extension in EXTENSIONS:
                self.assertIn(
                    extension.crx_url(
                        extract_manifest_versions(bundled_manifest())[
                            extension.extension_id
                        ]
                    ),
                    self.probed,
                )

    def test_plan_bytes_and_keys_are_deterministic_for_one_attempt(self) -> None:
        components = [
            Component(
                "agent-extension", "0.0.124.0", "ext-agent/v0.0.124.0", SOURCE_SHA
            )
        ]
        with (
            tempfile.TemporaryDirectory() as first,
            tempfile.TemporaryDirectory() as second,
        ):
            first_result = self.create(
                Path(first), components=components, manifest=bundled_manifest()
            )
            first_bytes = first_result.plan_path.read_bytes()
            first_manifest = first_result.manifest_path.read_bytes()
            second_result = self.create(
                Path(second), components=components, manifest=bundled_manifest()
            )

            self.assertEqual(first_result.plan_key, second_result.plan_key)
            self.assertEqual(first_bytes, second_result.plan_path.read_bytes())
            self.assertEqual(first_manifest, second_result.manifest_path.read_bytes())
            self.assertTrue(all(put["IfNoneMatch"] == "*" for put in self.client.puts))
            self.assertTrue(
                all(
                    put["Metadata"]["source-sha"] == SOURCE_SHA
                    for put in self.client.puts
                )
            )
            self.assertTrue(
                all("sha256" in put["Metadata"] for put in self.client.puts)
            )
            self.assertTrue(
                all(
                    put["Metadata"]["object-schema"] == "browseros-release-plan-v2"
                    for put in self.client.puts
                )
            )

    def test_plan_records_every_resource_pin(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            result = self.create(Path(temp_dir))
            plan = json.loads(result.plan_path.read_text())

            self.assertEqual(plan["schema_version"], 2)
            self.assertEqual(
                result.resource_versions,
                {
                    "browserclaw_onboard": "0.0.3",
                    "browserclaw_server": "0.0.2",
                    "browseros_onboard": "0.0.4",
                    "browseros_server": "0.0.1",
                },
            )
            self.assertEqual(
                {
                    name: value["version"]
                    for name, value in plan["resource_pins"].items()
                },
                result.resource_versions,
            )

    def test_retry_rejects_same_bytes_with_conflicting_binding(self) -> None:
        with (
            tempfile.TemporaryDirectory() as first,
            tempfile.TemporaryDirectory() as second,
        ):
            result = self.create(Path(first))
            self.client.metadata[result.plan_key]["source-sha"] = "b" * 40

            with self.assertRaisesRegex(RuntimeError, "binding mismatch"):
                self.create(Path(second))

    def test_conditional_409_rereads_and_reuses_the_winner(self) -> None:
        with (
            tempfile.TemporaryDirectory() as first,
            tempfile.TemporaryDirectory() as second,
        ):
            self.create(Path(first))
            conflict = ConflictR2Client()
            conflict.objects = dict(self.client.objects)
            conflict.metadata = {
                key: dict(metadata) for key, metadata in self.client.metadata.items()
            }
            self.client = conflict

            result = self.create(Path(second))

            self.assertEqual(
                result.plan_path.read_bytes(),
                conflict.objects[result.plan_key],
            )
            self.assertGreaterEqual(conflict.gets.count(result.plan_key), 2)

    def test_retry_attempt_changes_identity_and_object_keys(self) -> None:
        with (
            tempfile.TemporaryDirectory() as first,
            tempfile.TemporaryDirectory() as second,
        ):
            first_result = self.create(Path(first))
            second_result = self.create(Path(second), run_attempt="3")

            self.assertNotEqual(first_result.plan_key, second_result.plan_key)
            self.assertIn("run-30418029456-attempt-2", first_result.plan_key)
            self.assertIn("run-30418029456-attempt-3", second_result.plan_key)

    def test_plan_records_exact_source_and_sorted_components(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            result = self.create(
                Path(temp_dir),
                product="browserclaw",
                extension_name="browserclaw",
                components=[
                    Component(
                        "browserclaw-extension",
                        "0.1.8.0",
                        "ext-browserclaw/v0.1.8.0",
                        SOURCE_SHA,
                    ),
                    Component(
                        "browserclaw-server",
                        "0.0.22",
                        "claw-server/v0.0.22",
                        SOURCE_SHA,
                    ),
                    Component(
                        "browserclaw-onboard",
                        "0.0.4",
                        "claw-onboard/v0.0.4",
                        SOURCE_SHA,
                    ),
                ],
                manifest=bundled_manifest(),
            )
            plan = json.loads(result.plan_path.read_text(encoding="utf-8"))

            self.assertEqual(plan["source_sha"], SOURCE_SHA)
            self.assertEqual(plan["workflow_run_id"], "30418029456")
            self.assertEqual(plan["workflow_run_attempt"], 2)
            self.assertEqual(
                [component["name"] for component in plan["components"]],
                [
                    "browserclaw-extension",
                    "browserclaw-onboard",
                    "browserclaw-server",
                ],
            )

    def test_source_mismatch_duplicate_and_unknown_components_fail(self) -> None:
        cases = (
            (
                [
                    Component(
                        "browseros-server", "0.0.22", "agent-server/v0.0.22", "b" * 40
                    )
                ],
                "source",
            ),
            (
                [
                    Component(
                        "browseros-server", "0.0.22", "agent-server/v0.0.22", SOURCE_SHA
                    ),
                    Component(
                        "browseros-server", "0.0.22", "agent-server/v0.0.22", SOURCE_SHA
                    ),
                ],
                "duplicate",
            ),
            ([Component("other", "1.0.0", "other/v1.0.0", SOURCE_SHA)], "unknown"),
        )
        for components, error in cases:
            with self.subTest(error=error), tempfile.TemporaryDirectory() as temp_dir:
                with self.assertRaisesRegex(ValueError, error):
                    self.create(Path(temp_dir), components=components)

    def test_manifest_rejects_unknown_duplicate_missing_and_mutable_entries(
        self,
    ) -> None:
        app = extension_by_name("agent")
        mutations = {
            "unknown": bundled_manifest().replace(app.extension_id, "unknown", 1),
            "duplicate": bundled_manifest().replace(
                "</gupdate>",
                f'  <app appid="{app.extension_id}"><updatecheck codebase="{app.crx_url("0.0.123.0")}" version="0.0.123.0"/></app>\n</gupdate>',
            ),
            "missing": bundled_manifest().replace(
                f'  <app appid="{app.extension_id}">\n    <updatecheck codebase="{app.crx_url("0.0.123.0")}" version="0.0.123.0"/>\n  </app>\n',
                "",
            ),
            "versioned": bundled_manifest().replace(
                app.crx_url("0.0.123.0"),
                "https://cdn.browseros.com/extensions/agent-latest.crx",
            ),
        }
        component = Component(
            "agent-extension", "0.0.124.0", "ext-agent/v0.0.124.0", SOURCE_SHA
        )
        for error, manifest in mutations.items():
            with self.subTest(error=error), tempfile.TemporaryDirectory() as temp_dir:
                with self.assertRaisesRegex(ValueError, error):
                    self.create(
                        Path(temp_dir),
                        components=[component],
                        manifest=manifest,
                    )

    def test_manifest_requires_selected_extension_component(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            with self.assertRaisesRegex(ValueError, "selected extension"):
                self.create(Path(temp_dir), manifest=bundled_manifest())

    def test_release_plan_requires_a_manifest_snapshot_when_extension_is_skipped(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            with self.assertRaisesRegex(ValueError, "manifest snapshot"):
                self.create(Path(temp_dir), include_manifest=False)

    def test_extension_component_cannot_hide_behind_skipped_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            with self.assertRaisesRegex(ValueError, "selected extension is empty"):
                self.create(
                    Path(temp_dir),
                    components=[
                        Component(
                            "agent-extension",
                            "0.0.124.0",
                            "ext-agent/v0.0.124.0",
                            SOURCE_SHA,
                        )
                    ],
                )

    def test_component_tag_must_exactly_match_name_and_version(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            with self.assertRaisesRegex(ValueError, "invalid tag"):
                self.create(
                    Path(temp_dir),
                    components=[
                        Component(
                            "browseros-server",
                            "0.0.22",
                            "agent-server/v0.0.23",
                            SOURCE_SHA,
                        )
                    ],
                )

    def test_manifest_rejects_nested_apps_and_unexpected_root_children(self) -> None:
        agent = extension_by_name("agent")
        nested = bundled_manifest().replace(
            f'    <updatecheck codebase="{agent.crx_url("0.0.123.0")}" version="0.0.123.0"/>',
            f'    <app appid="{agent.extension_id}"><updatecheck codebase="{agent.crx_url("0.0.123.0")}" version="0.0.123.0"/></app>',
        )
        unexpected = bundled_manifest().replace(
            "</gupdate>", "  <metadata/>\n</gupdate>"
        )
        component = Component(
            "agent-extension", "0.0.124.0", "ext-agent/v0.0.124.0", SOURCE_SHA
        )
        for error, manifest in (("nested", nested), ("unexpected", unexpected)):
            with self.subTest(error=error), tempfile.TemporaryDirectory() as temp_dir:
                with self.assertRaisesRegex(ValueError, error):
                    self.create(
                        Path(temp_dir),
                        components=[component],
                        manifest=manifest,
                    )

    def test_manifest_rejects_unknown_structure_and_attributes(self) -> None:
        canonical = bundled_manifest()
        agent = extension_by_name("agent")
        malformed = (
            canonical.replace(' xmlns="http://www.google.com/update2/response"', ""),
            canonical.replace('protocol="2.0"', 'protocol="3.0"'),
            canonical.replace('protocol="2.0"', 'protocol="2.0" extra="x"'),
            canonical.replace(">\n  <app", ">mixed\n  <app", 1),
            canonical.replace(
                f'appid="{agent.extension_id}"',
                f'appid="{agent.extension_id}" extra="x"',
                1,
            ),
            canonical.replace(
                f'    <updatecheck codebase="{agent.crx_url("0.0.123.0")}" version="0.0.123.0"/>',
                f'    <updatecheck codebase="{agent.crx_url("0.0.123.0")}" version="0.0.123.0"/>\n    <foo/>',
            ),
            canonical.replace(
                'version="0.0.123.0"/>',
                'version="0.0.123.0" extra="x"/>',
                1,
            ),
        )
        component = Component(
            "agent-extension", "0.0.124.0", "ext-agent/v0.0.124.0", SOURCE_SHA
        )
        for manifest in malformed:
            with (
                self.subTest(manifest=manifest),
                tempfile.TemporaryDirectory() as temp_dir,
            ):
                with self.assertRaises(ValueError):
                    self.create(
                        Path(temp_dir),
                        components=[component],
                        manifest=manifest,
                    )

    def test_unsafe_run_identity_and_failed_url_probe_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            with self.assertRaisesRegex(ValueError, "run id"):
                self.create(Path(temp_dir), run_id="304/latest")

        def missing(_url: str) -> int:
            return 404

        self.probe = missing
        with tempfile.TemporaryDirectory() as temp_dir:
            with self.assertRaisesRegex(RuntimeError, "HTTP 404"):
                self.create(
                    Path(temp_dir),
                    components=[
                        Component(
                            "agent-extension",
                            "0.0.124.0",
                            "ext-agent/v0.0.124.0",
                            SOURCE_SHA,
                        )
                    ],
                    manifest=bundled_manifest(),
                )

    def test_cdn_base_url_must_be_a_safe_https_origin(self) -> None:
        invalid = (
            "http://cdn.browseros.com",
            "https://user:secret@cdn.browseros.com",
            "https://cdn.browseros.com/release-plans",
            "https://cdn.browseros.com?token=secret",
            "https://cdn.browseros.com#fragment",
            "https:///missing-host",
            "https://cdn.browseros.com\rINJECTED=1",
            "https://cdn.browseros.com\nINJECTED=1",
            "https://cdn.browseros.com\tpath",
            "https://cdn.browseros.com\\evil",
        )
        for url in invalid:
            with self.subTest(url=url), tempfile.TemporaryDirectory() as temp_dir:
                with self.assertRaisesRegex(ValueError, "CDN base URL"):
                    self.create(Path(temp_dir), cdn_base_url=url)

    def test_run_identity_length_and_component_version_shapes_are_bounded(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            with self.assertRaisesRegex(ValueError, "run id"):
                self.create(Path(temp_dir), run_id="1" * 21)
        with tempfile.TemporaryDirectory() as temp_dir:
            with self.assertRaisesRegex(ValueError, "invalid version"):
                self.create(
                    Path(temp_dir),
                    components=[
                        Component(
                            "browseros-server",
                            "0.0.22.0",
                            "agent-server/v0.0.22.0",
                            SOURCE_SHA,
                        )
                    ],
                )
        with tempfile.TemporaryDirectory() as temp_dir:
            with self.assertRaisesRegex(ValueError, "invalid version"):
                self.create(
                    Path(temp_dir),
                    components=[
                        Component(
                            "agent-extension",
                            "0.0.124",
                            "ext-agent/v0.0.124",
                            SOURCE_SHA,
                        )
                    ],
                    manifest=bundled_manifest(),
                )

    def test_github_outputs_reject_carriage_return_injection(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            result = PlanResult(
                plan_path=root / "release-plan.json",
                plan_key="release-plans/plan.json",
                plan_url="https://cdn.browseros.com\rINJECTED=1",
                manifest_path=None,
                manifest_key="",
                manifest_url="",
                resource_versions={},
            )
            with self.assertRaisesRegex(ValueError, "unsafe output"):
                _write_github_output(root / "github-output", result)

    def test_complete_manifest_has_exactly_the_known_extensions(self) -> None:
        self.assertEqual(
            {extension.name for extension in EXTENSIONS},
            {"agent", "browserclaw", "bugreporter"},
        )


if __name__ == "__main__":
    unittest.main()
