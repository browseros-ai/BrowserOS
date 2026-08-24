#!/usr/bin/env python3
"""Tests for immutable browser resource pin resolution."""

import unittest

from .resource_pins import (
    RESOURCE_FAMILIES,
    resolve_resource_pins,
    verify_prepared_resource_pin,
)


SOURCE_SHA = "a" * 40


class FakeR2Client:
    def __init__(self) -> None:
        self.objects: dict[str, dict] = {}
        self.head_reads: dict[str, int] = {}
        self.replacements: dict[str, tuple[int, dict]] = {}

    def add_family(
        self,
        family,
        version: str,
        *,
        metadata: bool,
        release_sha: str = SOURCE_SHA,
        marker: str | None = None,
    ) -> None:
        marker = marker or version
        for target in family.targets:
            values = self._values(
                family, target, version, release_sha, marker, metadata
            )
            self.objects[family.version_key(version, target)] = values
            self.objects[family.latest_key(target)] = values

    def replace_after_head(self, key: str, read_number: int, values: dict) -> None:
        self.replacements[key] = (read_number, values)

    def head_object(self, *, Bucket, Key):
        del Bucket
        reads = self.head_reads.get(Key, 0) + 1
        self.head_reads[Key] = reads
        replacement = self.replacements.get(Key)
        if replacement and reads >= replacement[0]:
            self.objects[Key] = replacement[1]
        return dict(self.objects[Key])

    def list_objects_v2(self, *, Bucket, Prefix, ContinuationToken=None):
        del Bucket, ContinuationToken
        return {
            "IsTruncated": False,
            "Contents": [
                {
                    "Key": key,
                    "ETag": value["ETag"],
                    "Size": value["ContentLength"],
                }
                for key, value in sorted(self.objects.items())
                if key.startswith(Prefix)
            ],
        }

    @staticmethod
    def _values(family, target, version, release_sha, marker, metadata):
        binding = {}
        if metadata:
            binding = {
                "component": family.component,
                "release-sha": release_sha,
                "sha256": marker[0].encode().hex().ljust(64, "0")[:64],
                "target": target,
                "version": version,
            }
        identity = f"{family.name}:{target}:{marker}"
        return {
            "ETag": f'"{identity}"',
            "ContentLength": len(identity),
            "Metadata": binding,
        }


class ResourcePinsTest(unittest.TestCase):
    def setUp(self) -> None:
        self.client = FakeR2Client()

    def add_all(self, *, metadata: bool) -> None:
        for index, family in enumerate(RESOURCE_FAMILIES, start=1):
            self.client.add_family(family, f"0.0.{index}", metadata=metadata)

    def resolve(self, prepared=None, attempts=1):
        return resolve_resource_pins(
            self.client,
            "browseros",
            prepared or {},
            attempts=attempts,
            sleep=lambda _: None,
        )

    def test_resolves_complete_metadata_bindings(self) -> None:
        self.add_all(metadata=True)

        pins = self.resolve()

        self.assertEqual(
            {name: pin.version for name, pin in pins.items()},
            {
                "browseros_server": "0.0.1",
                "browserclaw_server": "0.0.2",
                "browserclaw_onboard": "0.0.3",
                "browseros_onboard": "0.0.4",
            },
        )
        self.assertTrue(
            all(
                item.sha256 and item.release_sha
                for pin in pins.values()
                for item in pin.objects
            )
        )

    def test_legacy_scan_requires_one_version_matching_every_alias(self) -> None:
        self.add_all(metadata=False)
        family = RESOURCE_FAMILIES[0]
        self.client.add_family(family, "0.0.9", metadata=False, marker="different")
        for target in family.targets:
            self.client.objects[family.latest_key(target)] = self.client.objects[
                family.version_key("0.0.1", target)
            ]

        pins = self.resolve()

        self.assertEqual(pins[family.name].version, "0.0.1")
        self.assertTrue(all(not item.sha256 for item in pins[family.name].objects))

    def test_prepared_component_uses_exact_objects_without_reading_latest(self) -> None:
        self.add_all(metadata=True)
        family = RESOURCE_FAMILIES[0]
        self.client.add_family(family, "0.0.8", metadata=True)
        before = {
            key: self.client.head_reads.get(key, 0)
            for key in (family.latest_key(target) for target in family.targets)
        }

        pins = self.resolve({family.name: ("0.0.8", SOURCE_SHA)})

        self.assertEqual(pins[family.name].version, "0.0.8")
        self.assertEqual(
            before,
            {
                key: self.client.head_reads.get(key, 0)
                for key in (family.latest_key(target) for target in family.targets)
            },
        )

    def test_rejects_cross_target_version_skew(self) -> None:
        self.add_all(metadata=True)
        family = RESOURCE_FAMILIES[0]
        target = family.targets[-1]
        self.client.objects[family.latest_key(target)] = FakeR2Client._values(
            family, target, "0.0.9", SOURCE_SHA, "0.0.9", True
        )

        with self.assertRaisesRegex(RuntimeError, "version-skewed"):
            self.resolve()

    def test_rejects_mixed_legacy_and_bound_aliases(self) -> None:
        self.add_all(metadata=False)
        family = RESOURCE_FAMILIES[1]
        target = family.targets[0]
        self.client.objects[family.latest_key(target)] = FakeR2Client._values(
            family, target, "0.0.2", SOURCE_SHA, "0.0.2", True
        )

        with self.assertRaisesRegex(RuntimeError, "mixed bindings"):
            self.resolve()

    def test_rejects_ambiguous_legacy_match(self) -> None:
        self.add_all(metadata=False)
        family = RESOURCE_FAMILIES[2]
        self.client.add_family(family, "0.0.9", metadata=False, marker="0.0.3")

        with self.assertRaisesRegex(RuntimeError, "match 2 versions"):
            self.resolve()

    def test_retries_when_alias_changes_during_snapshot(self) -> None:
        self.add_all(metadata=True)
        family = RESOURCE_FAMILIES[2]
        target = family.targets[0]
        replacement = FakeR2Client._values(
            family, target, "0.0.3", SOURCE_SHA, "0.0.3", True
        )
        replacement["ETag"] = '"changed-once"'
        replacement["ContentLength"] = 99
        self.client.objects[family.version_key("0.0.3", target)] = replacement
        self.client.replace_after_head(family.latest_key(target), 2, replacement)

        pins = self.resolve(attempts=2)

        self.assertEqual(pins[family.name].objects[0].etag, '"changed-once"')
        self.assertGreaterEqual(self.client.head_reads[family.latest_key(target)], 3)

    def test_prepared_binding_must_match_source(self) -> None:
        self.add_all(metadata=True)
        family = RESOURCE_FAMILIES[1]

        with self.assertRaisesRegex(RuntimeError, "source binding mismatch"):
            self.resolve({family.name: ("0.0.2", "b" * 40)})

    def test_verifies_one_prepared_family_without_reading_latest(self) -> None:
        family = RESOURCE_FAMILIES[0]
        self.client.add_family(family, "0.0.8", metadata=True)

        pin = verify_prepared_resource_pin(
            self.client,
            "browseros",
            family.name,
            "0.0.8",
            SOURCE_SHA,
        )

        self.assertEqual(pin.version, "0.0.8")
        self.assertEqual(
            {item.key for item in pin.objects},
            {
                family.version_key("0.0.8", target)
                for target in family.targets
            },
        )
        self.assertFalse(
            any("/latest/" in key for key in self.client.head_reads)
        )

    def test_single_prepared_family_rejects_binding_mismatch(self) -> None:
        family = RESOURCE_FAMILIES[1]
        self.client.add_family(family, "0.0.8", metadata=True)
        target = family.targets[0]
        key = family.version_key("0.0.8", target)
        self.client.objects[key]["Metadata"]["sha256"] = "invalid"

        with self.assertRaisesRegex(RuntimeError, "invalid sha256 binding"):
            verify_prepared_resource_pin(
                self.client,
                "browseros",
                family.name,
                "0.0.8",
                SOURCE_SHA,
            )


if __name__ == "__main__":
    unittest.main()
