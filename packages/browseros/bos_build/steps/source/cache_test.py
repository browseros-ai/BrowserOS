#!/usr/bin/env python3
"""Tests for chromium source cache command construction."""

import tempfile
import unittest
from pathlib import Path
from unittest import mock

from bos_build.steps.source import cache


class PipelineTest(unittest.TestCase):
    def test_run_pipeline_passes_process_cwds(self):
        producer_stdout = mock.Mock()
        producer = mock.Mock(stdout=producer_stdout)
        producer.wait.return_value = 0
        consumer = mock.Mock()
        consumer.wait.return_value = 0

        producer_cwd = Path("/tmp/producer")
        consumer_cwd = Path("/tmp/consumer")
        with mock.patch.object(
            cache.subprocess, "Popen", side_effect=[producer, consumer]
        ) as popen:
            cache._run_pipeline(
                ["producer"],
                ["consumer"],
                producer_cwd=producer_cwd,
                consumer_cwd=consumer_cwd,
            )

        self.assertEqual(
            popen.call_args_list[0],
            mock.call(["producer"], stdout=cache.subprocess.PIPE, cwd=producer_cwd),
        )
        self.assertEqual(
            popen.call_args_list[1],
            mock.call(["consumer"], stdin=producer_stdout, cwd=consumer_cwd),
        )
        producer_stdout.close.assert_called_once()


class RestoreTest(unittest.TestCase):
    def test_restore_posix_creates_root_and_extracts_with_streaming_pipeline(self):
        client = mock.Mock()

        def fake_download(bucket, key, filename, Config=None):
            Path(filename).write_bytes(b"tarball")

        client.download_file.side_effect = fake_download
        transfer_config = object()

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            root = tmp_path / "chromium"
            expected_root = root.resolve()
            tarball = tmp_path / "chromium-cache.tar.zst"

            with (
                mock.patch.object(
                    cache, "_get_r2_client", return_value=(client, transfer_config)
                ),
                mock.patch.object(cache, "_object_exists", return_value=True),
                mock.patch.object(
                    cache, "find_tool", side_effect=lambda name: f"{name}.exe"
                ),
                mock.patch.object(cache.sys, "platform", "linux"),
                mock.patch.object(cache.tempfile, "gettempdir", return_value=tmp),
                mock.patch.object(cache, "_run_pipeline") as run_pipeline,
                mock.patch.object(cache, "write_github_output"),
            ):
                self.assertTrue(cache.restore("win-key", root))
                self.assertTrue(expected_root.is_dir())

        run_pipeline.assert_called_once_with(
            ["zstd.exe", "-d", "-c", str(tarball)],
            ["tar.exe", "-xf", "-"],
            consumer_cwd=expected_root,
        )
        self.assertNotIn("-C", run_pipeline.call_args.args[1])

    def test_restore_windows_retries_tar_once_with_msys_env(self):
        client = mock.Mock()

        def fake_download(bucket, key, filename, Config=None):
            Path(filename).write_bytes(b"tarball")

        tar_attempts = []

        def fake_run(cmd, *, cwd=None, env=None):
            if cmd[0] == "zstd.exe":
                Path(cmd[cmd.index("-o") + 1]).write_bytes(b"tar")
                return 0
            tar_attempts.append(cmd)
            return 1 if len(tar_attempts) == 1 else 0

        client.download_file.side_effect = fake_download
        transfer_config = object()
        run_calls = []

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            root = tmp_path / "chromium"
            expected_root = root.resolve()
            tarball = tmp_path / "chromium-cache.tar.zst"
            tar_file = tmp_path / "chromium-cache.tar"

            with (
                mock.patch.object(
                    cache, "_get_r2_client", return_value=(client, transfer_config)
                ),
                mock.patch.object(cache, "_object_exists", return_value=True),
                mock.patch.object(
                    cache, "find_tool", side_effect=lambda name: f"{name}.exe"
                ),
                mock.patch.object(cache.sys, "platform", "win32"),
                mock.patch.object(cache.tempfile, "gettempdir", return_value=tmp),
                mock.patch.object(cache, "write_github_output"),
                mock.patch.dict(
                    cache.os.environ, {"KEEP_ENV": "1", "MSYS": "old"}, clear=False
                ),
                mock.patch.object(cache, "_run_command") as run_command,
            ):
                run_command.side_effect = lambda cmd, **kwargs: (
                    run_calls.append((cmd, kwargs)) or fake_run(cmd, **kwargs)
                )
                self.assertTrue(cache.restore("win-key", root))

        self.assertEqual(
            [call[0] for call in run_calls],
            [
                ["zstd.exe", "-d", "-f", "-o", str(tar_file), str(tarball)],
                ["tar.exe", "--force-local", "-xf", str(tar_file)],
                ["tar.exe", "--force-local", "-xf", str(tar_file)],
            ],
        )
        self.assertNotIn("env", run_calls[0][1])
        for _, kwargs in run_calls[1:]:
            self.assertEqual(kwargs["cwd"], expected_root)
            self.assertEqual(kwargs["env"]["KEEP_ENV"], "1")
            self.assertEqual(kwargs["env"]["MSYS"], "winsymlinks:nativestrict")
        self.assertFalse(tarball.exists())
        self.assertFalse(tar_file.exists())

    def test_restore_windows_skips_second_pass_when_first_tar_succeeds(self):
        client = mock.Mock()

        def fake_download(bucket, key, filename, Config=None):
            Path(filename).write_bytes(b"tarball")

        def fake_run(cmd, **kwargs):
            if cmd[0] == "zstd.exe":
                Path(cmd[cmd.index("-o") + 1]).write_bytes(b"tar")
            return 0

        client.download_file.side_effect = fake_download
        transfer_config = object()
        run_calls = []

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "chromium"
            tarball = Path(tmp) / "chromium-cache.tar.zst"
            tar_file = Path(tmp) / "chromium-cache.tar"

            with (
                mock.patch.object(
                    cache, "_get_r2_client", return_value=(client, transfer_config)
                ),
                mock.patch.object(cache, "_object_exists", return_value=True),
                mock.patch.object(
                    cache, "find_tool", side_effect=lambda name: f"{name}.exe"
                ),
                mock.patch.object(cache.sys, "platform", "win32"),
                mock.patch.object(cache.tempfile, "gettempdir", return_value=tmp),
                mock.patch.object(cache, "write_github_output"),
                mock.patch.object(cache, "_run_command") as run_command,
            ):
                run_command.side_effect = lambda cmd, **kwargs: (
                    run_calls.append((cmd, kwargs)) or fake_run(cmd, **kwargs)
                )
                self.assertTrue(cache.restore("win-key", root))

        self.assertEqual(
            [call[0] for call in run_calls],
            [
                ["zstd.exe", "-d", "-f", "-o", str(tar_file), str(tarball)],
                ["tar.exe", "--force-local", "-xf", str(tar_file)],
            ],
        )

    def test_restore_windows_fails_when_retry_tar_pass_fails(self):
        client = mock.Mock()

        def fake_download(bucket, key, filename, Config=None):
            Path(filename).write_bytes(b"tarball")

        def fake_run(cmd, **kwargs):
            if cmd[0] == "zstd.exe":
                Path(cmd[cmd.index("-o") + 1]).write_bytes(b"tar")
                return 0
            return 1

        client.download_file.side_effect = fake_download
        transfer_config = object()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "chromium"
            tar_file = Path(tmp) / "chromium-cache.tar"

            with (
                mock.patch.object(
                    cache, "_get_r2_client", return_value=(client, transfer_config)
                ),
                mock.patch.object(cache, "_object_exists", return_value=True),
                mock.patch.object(
                    cache, "find_tool", side_effect=lambda name: f"{name}.exe"
                ),
                mock.patch.object(cache.sys, "platform", "win32"),
                mock.patch.object(cache.tempfile, "gettempdir", return_value=tmp),
                mock.patch.object(cache, "write_github_output"),
                mock.patch.object(
                    cache, "_run_command", side_effect=fake_run
                ) as run_command,
            ):
                with self.assertRaises(SystemExit):
                    cache.restore("win-key", root)

            self.assertFalse(tar_file.exists())

        self.assertEqual(run_command.call_count, 3)

    def test_restore_returns_miss_without_r2_credentials(self):
        with mock.patch.object(cache, "_get_r2_client", return_value=None):
            self.assertFalse(cache.restore("missing", Path("/tmp/chromium")))


class SaveTest(unittest.TestCase):
    def test_save_archives_relative_entries_with_tar_cwd(self):
        client = mock.Mock()
        transfer_config = object()
        pipeline_calls = []

        def fake_pipeline(
            producer,
            consumer,
            *,
            producer_cwd=None,
            consumer_cwd=None,
        ):
            pipeline_calls.append((producer, consumer, producer_cwd, consumer_cwd))
            output_path = Path(consumer[consumer.index("-o") + 1])
            output_path.write_bytes(b"tarball")

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "chromium"
            root.mkdir()
            expected_root = root.resolve()
            tarball = expected_root.parent / "chromium-cache.tar.zst"

            with (
                mock.patch.object(
                    cache, "_get_r2_client", return_value=(client, transfer_config)
                ),
                mock.patch.object(cache, "_object_exists", return_value=False),
                mock.patch.object(
                    cache, "find_tool", side_effect=lambda name: f"{name}.exe"
                ),
                mock.patch.object(cache, "_run_pipeline", side_effect=fake_pipeline),
            ):
                cache.save("win-key", root)

        self.assertEqual(
            pipeline_calls,
            [
                (
                    ["tar.exe", "-cf", "-", "--exclude=./src/out", "."],
                    ["zstd.exe", "-T0", "-3", "-f", "-o", str(tarball)],
                    expected_root,
                    None,
                )
            ],
        )
        self.assertNotIn("-C", pipeline_calls[0][0])
        client.upload_file.assert_called_once_with(
            str(tarball),
            "browseros",
            "ci-cache/chromium/win-key.tar.zst",
            Config=transfer_config,
        )


if __name__ == "__main__":
    unittest.main()
