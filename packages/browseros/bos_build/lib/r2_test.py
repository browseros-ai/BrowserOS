#!/usr/bin/env python3
"""Tests for the R2 JSON compare-and-swap used by parallel release lanes."""

import io
import json
import unittest
from unittest import mock

from botocore.exceptions import ClientError

from bos_build.lib.r2 import update_json_in_r2


class _NoSuchKey(Exception):
    pass


class _ConditionalStore:
    """In-memory R2 object honoring PutObject If-Match/If-None-Match.

    ``interleave`` runs once after the first read, standing in for a sibling
    lane that writes the shared receipt between our read and our put.
    """

    exceptions = type("Exceptions", (), {"NoSuchKey": _NoSuchKey})

    def __init__(self, document=None, interleave=None):
        self.document = document
        self.version = 0 if document is None else 1
        self.interleave = interleave
        self.puts = []

    def get_object(self, Bucket, Key):
        snapshot, etag = self.document, f'"v{self.version}"'
        if self.interleave:
            interleave, self.interleave = self.interleave, None
            interleave(self)
        if snapshot is None:
            raise _NoSuchKey()
        return {"Body": io.BytesIO(json.dumps(snapshot).encode()), "ETag": etag}

    def put_object(self, Bucket, Key, Body, IfMatch=None, IfNoneMatch=None):
        self.puts.append({"IfMatch": IfMatch, "IfNoneMatch": IfNoneMatch})
        exists = self.document is not None
        stale = IfMatch is not None and IfMatch != f'"v{self.version}"'
        if stale or (IfNoneMatch == "*" and exists):
            raise ClientError(
                {
                    "Error": {"Code": "PreconditionFailed"},
                    "ResponseMetadata": {"HTTPStatusCode": 412},
                },
                "PutObject",
            )
        self.document = json.loads(Body)
        self.version += 1


def _merge_artifact(key):
    def update(current):
        merged = dict(current or {"artifacts": {}})
        merged["artifacts"] = {**merged["artifacts"], key: {"filename": key}}
        return merged

    return update


@mock.patch("bos_build.lib.r2.time.sleep", lambda _seconds: None)
class UpdateJsonInR2Test(unittest.TestCase):
    def test_first_writer_creates_only_when_absent(self):
        store = _ConditionalStore()

        result = update_json_in_r2(store, "k", "b", _merge_artifact("x64_deb"))

        self.assertEqual(result, {"artifacts": {"x64_deb": {"filename": "x64_deb"}}})
        self.assertEqual(store.puts, [{"IfMatch": None, "IfNoneMatch": "*"}])

    def test_stale_writer_remerges_instead_of_dropping_sibling(self):
        sibling = _merge_artifact("arm64_deb")

        def sibling_writes(store):
            store.document = sibling(store.document)
            store.version += 1

        store = _ConditionalStore({"artifacts": {}}, interleave=sibling_writes)

        result = update_json_in_r2(store, "k", "b", _merge_artifact("x64_deb"))
        assert result is not None

        self.assertEqual(set(result["artifacts"]), {"arm64_deb", "x64_deb"})
        self.assertEqual(store.document, result)
        self.assertEqual(
            store.puts,
            [
                {"IfMatch": '"v1"', "IfNoneMatch": None},
                {"IfMatch": '"v2"', "IfNoneMatch": None},
            ],
        )

    def test_concurrent_first_writers_both_land(self):
        def sibling_creates(store):
            store.document = _merge_artifact("arm64_deb")(None)
            store.version = 1

        store = _ConditionalStore(interleave=sibling_creates)

        result = update_json_in_r2(store, "k", "b", _merge_artifact("x64_deb"))
        assert result is not None

        self.assertEqual(set(result["artifacts"]), {"arm64_deb", "x64_deb"})
        self.assertEqual(store.puts[0], {"IfMatch": None, "IfNoneMatch": "*"})
        self.assertEqual(store.puts[1], {"IfMatch": '"v1"', "IfNoneMatch": None})

    def test_non_conflict_errors_fail_without_retry(self):
        store = _ConditionalStore()

        def denied(**_kwargs):
            raise ClientError(
                {
                    "Error": {"Code": "AccessDenied"},
                    "ResponseMetadata": {"HTTPStatusCode": 403},
                },
                "PutObject",
            )

        store.put_object = denied

        self.assertIsNone(
            update_json_in_r2(store, "k", "b", _merge_artifact("x64_deb"))
        )


if __name__ == "__main__":
    unittest.main()
