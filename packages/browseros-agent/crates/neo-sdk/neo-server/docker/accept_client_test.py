"""Acceptance probes read Neo's text value and retain host error metadata."""

import json
import subprocess
import unittest
from pathlib import Path
from unittest.mock import Mock

from accept_client import RUN_META, call_run


class AcceptClientTests(unittest.TestCase):
    def test_reads_the_result_section_and_preserves_metadata(self):
        value = {"title": "neo-sdk-accept", "value": "zuzu"}
        meta = {"id": "run", "status": "ok", "tasks": []}
        reply = {"content": [{"type": "text", "text": f"### Result\n{json.dumps(value)}\n### Console\nhello"}],
                 "_meta": {RUN_META: meta}}
        client = Mock()
        client.call.return_value = reply
        got = call_run(client, "return 1", {"requestId": "request"})
        self.assertEqual(got["value"], value)
        self.assertEqual(got["meta"], meta)
        self.assertFalse(got["isError"])

    def test_failed_runs_keep_the_hosts_error(self):
        error = {"code": "result_too_large", "message": "use Buffer.from"}
        client = Mock()
        client.call.return_value = {"content": [{"type": "text", "text": "### Error\nneo / result_too_large: use Buffer.from"}],
                                    "isError": True, "_meta": {RUN_META: {"error": error}}}
        got = call_run(client, "return 'x'.repeat(300 * 1024)", {})
        self.assertIsNone(got["value"])
        self.assertEqual(got["meta"]["error"], error)
        self.assertTrue(got["isError"])

    def test_example_checks_read_json_from_result(self):
        script = Path(__file__).parent / "devkit/examples/check.sh"
        definitions = script.read_text().split("DEFS='", 1)[1].split("'", 1)[0]
        for value in (None, {"value": "zuzu"}, "text\n### Console\nmore text"):
            with self.subTest(value=value):
                record = {"response": {"content": [{"type": "text", "text":
                    f"### Result\n{json.dumps(value)}\n### Console\nhello"}]}}
                process = subprocess.run(["jq", "-c", definitions + " value"], input=json.dumps(record),
                                         capture_output=True, text=True, check=True)
                self.assertEqual(json.loads(process.stdout), value)


if __name__ == "__main__":
    unittest.main()
