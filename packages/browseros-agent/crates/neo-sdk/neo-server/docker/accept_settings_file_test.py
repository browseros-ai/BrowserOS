"""The recording probe releases SQLite readers before its next server restart."""

from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import patch

import accept_settings_file as probe


class RecordingReaderTest(unittest.TestCase):
    def test_recording_scan_closes_sqlite_handles_before_a_server_restart(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            state = root / "claw-sdk"
            state.mkdir()
            database = state / "trace.db"
            seed = sqlite3.connect(database)
            try:
                seed.executescript('''
                    CREATE TABLE runs (id TEXT PRIMARY KEY, "values" TEXT, code TEXT, complete INTEGER);
                    CREATE TABLE operations (run_id TEXT, screenshot TEXT);
                    INSERT INTO runs VALUES ('probe-run', 'off', NULL, 1);
                    INSERT INTO operations VALUES ('probe-run', NULL);
                ''')
            finally:
                seed.close()

            readers = []
            connect = sqlite3.connect

            def open_reader(*args, **kwargs):
                reader = connect(*args, **kwargs)
                readers.append(reader)
                return reader

            try:
                with (
                    patch.object(probe, "ROOT", root),
                    patch.object(probe, "STATE", state),
                    patch.object(probe, "connect", return_value=object()),
                    patch.object(probe, "call_run", return_value={
                        "meta": {"id": "probe-run", "status": "ok"},
                        "value": {"value": True},
                    }),
                    patch.object(probe, "host", return_value={"status": 200, "body": {"values": "off"}}),
                    patch.object(probe.sqlite3, "connect", side_effect=open_reader),
                ):
                    proof = probe.Proof()
                    proof.recording_off()
                    self.assertTrue(all(check["ok"] for check in proof.checks))

                self.assertTrue(readers, "the probe inspected the real SQLite fixture")
                for reader in readers:
                    with self.assertRaises(sqlite3.ProgrammingError):
                        reader.execute("SELECT 1")
            finally:
                for reader in readers:
                    reader.close()
