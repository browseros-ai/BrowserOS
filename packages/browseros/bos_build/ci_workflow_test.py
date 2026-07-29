#!/usr/bin/env python3
"""Regression tests for the reusable Chromium build workflow."""

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

import yaml


REPO_ROOT = Path(__file__).resolve().parents[3]
WORKFLOW_DIR = REPO_ROOT / ".github" / "workflows"
GIT_BOOTSTRAP_STEP = "Configure Git for depot_tools"
EXPECTED_GIT_CONFIG = {
    "core.autocrlf": "false",
    "core.filemode": "false",
    "core.fscache": "true",
    "core.preloadindex": "true",
    "depot-tools.allowGlobalGitConfig": "true",
}


def git_bash_path() -> str:
    """Return Git for Windows' Bash instead of the unrelated WSL stub."""
    if os.name != "nt":
        return "bash"

    result = subprocess.run(
        ["git", "--exec-path"],
        capture_output=True,
        check=True,
        text=True,
    )
    git_install_root = Path(result.stdout.strip()).parents[2]
    git_bash = git_install_root / "bin" / "bash.exe"
    if not git_bash.is_file():
        raise AssertionError(f"Git Bash not found at {git_bash}")
    return str(git_bash)


class ChromiumBuildWorkflowTest(unittest.TestCase):
    def load_workflow(self, workflow_name: str) -> dict[str, object]:
        workflow_path = WORKFLOW_DIR / workflow_name
        return yaml.safe_load(workflow_path.read_text(encoding="utf-8"))

    def build_steps(self) -> list[dict[str, object]]:
        workflow = self.load_workflow("build-browseros.yml")
        return workflow["jobs"]["build"]["steps"]

    def git_bootstrap_step(self) -> dict[str, object]:
        return next(
            step
            for step in self.build_steps()
            if step.get("name") == GIT_BOOTSTRAP_STEP
        )

    def test_git_bootstrap_is_windows_only_and_immediately_after_checkout(self):
        steps = self.build_steps()
        checkout_index = next(
            index
            for index, step in enumerate(steps)
            if str(step.get("uses", "")).startswith("actions/checkout@")
        )
        bootstrap_index = next(
            index
            for index, step in enumerate(steps)
            if step.get("name") == GIT_BOOTSTRAP_STEP
        )

        self.assertEqual(bootstrap_index, checkout_index + 1)
        self.assertEqual(
            steps[bootstrap_index]["if"],
            "runner.os == 'Windows'",
        )

    def test_git_bootstrap_precedes_every_chromium_lifecycle_phase(self):
        steps = self.build_steps()
        indexes = {
            step.get("name"): index
            for index, step in enumerate(steps)
            if "name" in step
        }
        bootstrap_index = indexes[GIT_BOOTSTRAP_STEP]

        for phase in (
            "Resolve chromium pin and paths",
            "Restore chromium checkout (WarpCache)",
            "Restore chromium checkout (R2)",
            "Ensure chromium checkout at pinned tag",
            "Reset chromium tree (clean module)",
            "Sync chromium dependencies (gclient)",
        ):
            with self.subTest(phase=phase):
                self.assertLess(bootstrap_index, indexes[phase])

    def test_git_bootstrap_uses_isolated_global_config_and_exact_values(self):
        script = self.git_bootstrap_step()["run"]

        self.assertIn("set -euo pipefail", script)
        self.assertIn(
            'git_config_dir="$(cd "$RUNNER_TEMP" && pwd -W)"',
            script,
        )
        self.assertIn(
            'git_config="$git_config_dir/browseros-global.gitconfig"',
            script,
        )
        self.assertIn('export GIT_CONFIG_GLOBAL="$git_config"', script)
        self.assertIn(
            "printf 'GIT_CONFIG_GLOBAL=%s\\n' \"$git_config\" >> \"$GITHUB_ENV\"",
            script,
        )
        self.assertNotIn("GIT_CONFIG_NOSYSTEM", script)

        for key, value in EXPECTED_GIT_CONFIG.items():
            with self.subTest(key=key):
                self.assertIn(
                    f"git config --global --replace-all {key} {value}",
                    script,
                )
                self.assertIn(
                    f'test "$(git config --global --get {key})" = {value}',
                    script,
                )

    def test_literal_git_bootstrap_is_home_independent_and_idempotent(self):
        script = self.git_bootstrap_step()["run"]

        with tempfile.TemporaryDirectory(prefix="browseros git bootstrap ") as tmp:
            temp_root = Path(tmp)
            runner_temp = temp_root / "runner temp"
            runner_temp.mkdir()
            missing_home = temp_root / "missing home"
            github_env = temp_root / "github env"
            config_path = runner_temp / "browseros-global.gitconfig"

            env = os.environ.copy()
            env.pop("GIT_CONFIG_GLOBAL", None)
            env.update(
                {
                    "GITHUB_ENV": str(github_env),
                    "HOME": str(missing_home),
                    "RUNNER_OS": "Windows" if os.name == "nt" else "Linux",
                    "RUNNER_TEMP": str(runner_temp),
                }
            )

            subprocess.run(
                [git_bash_path(), "-c", script],
                check=True,
                env=env,
            )
            self.assertTrue(config_path.is_file())

            for key in EXPECTED_GIT_CONFIG:
                subprocess.run(
                    [
                        "git",
                        "config",
                        "--file",
                        str(config_path),
                        "--add",
                        key,
                        "stale",
                    ],
                    check=True,
                )
                subprocess.run(
                    [
                        "git",
                        "config",
                        "--file",
                        str(config_path),
                        "--add",
                        key,
                        "duplicate",
                    ],
                    check=True,
                )

            subprocess.run(
                [git_bash_path(), "-c", script],
                check=True,
                env=env,
            )

            self.assertFalse(missing_home.exists())
            assignments = github_env.read_text(encoding="utf-8").splitlines()
            self.assertEqual(len(assignments), 2)
            for assignment in assignments:
                name, emitted_path = assignment.split("=", maxsplit=1)
                self.assertEqual(name, "GIT_CONFIG_GLOBAL")
                self.assertNotIn("\\", emitted_path)
                self.assertEqual(
                    Path(emitted_path).resolve(),
                    config_path.resolve(),
                )
            for key, value in EXPECTED_GIT_CONFIG.items():
                with self.subTest(key=key):
                    result = subprocess.run(
                        [
                            "git",
                            "config",
                            "--file",
                            str(config_path),
                            "--get-all",
                            key,
                        ],
                        capture_output=True,
                        check=True,
                        text=True,
                    )
                    self.assertEqual(result.stdout.splitlines(), [value])

    @unittest.skipUnless(os.name == "nt", "requires Git for Windows")
    def test_native_git_wrapper_reads_config_propagated_by_git_bash(self):
        script = self.git_bootstrap_step()["run"]

        with tempfile.TemporaryDirectory(prefix="browseros native git ") as tmp:
            temp_root = Path(tmp)
            runner_temp = temp_root / "runner temp"
            runner_temp.mkdir()
            missing_home = temp_root / "missing home"
            github_env = temp_root / "github env"

            bash_env = os.environ.copy()
            bash_env.pop("GIT_CONFIG_GLOBAL", None)
            bash_env.update(
                {
                    "GITHUB_ENV": str(github_env),
                    "HOME": str(missing_home),
                    "RUNNER_OS": "Windows",
                    "RUNNER_TEMP": str(runner_temp),
                }
            )
            subprocess.run(
                [git_bash_path(), "-c", script],
                check=True,
                env=bash_env,
            )

            assignment = github_env.read_text(encoding="utf-8").strip()
            name, config_path = assignment.split("=", maxsplit=1)
            self.assertEqual(name, "GIT_CONFIG_GLOBAL")

            git_wrapper = temp_root / "git-wrapper.bat"
            git_wrapper.write_bytes(b"@echo off\r\ngit %*\r\n")
            native_env = os.environ.copy()
            native_env.update(
                {
                    "GIT_CONFIG_GLOBAL": config_path,
                    "HOME": str(missing_home),
                }
            )

            for key, value in EXPECTED_GIT_CONFIG.items():
                with self.subTest(key=key):
                    command = subprocess.list2cmdline(
                        [
                            git_wrapper.name,
                            "config",
                            "--global",
                            "--get-all",
                            key,
                        ]
                    )
                    result = subprocess.run(
                        ["cmd.exe", "/d", "/c", command],
                        capture_output=True,
                        cwd=temp_root,
                        env=native_env,
                        text=True,
                    )
                    self.assertEqual(
                        result.returncode,
                        0,
                        msg=f"stdout={result.stdout!r} stderr={result.stderr!r}",
                    )
                    self.assertEqual(result.stdout.splitlines(), [value])

    def test_reusable_workflow_changes_trigger_build_system_tests(self):
        test_workflow = self.load_workflow("bos-build-tests.yml")
        # PyYAML's YAML 1.1 resolver treats the GitHub Actions `on` key as a
        # boolean, so accept either representation here.
        triggers = test_workflow.get("on", test_workflow.get(True))
        pull_request_paths = triggers["pull_request"]["paths"]

        self.assertIn(
            ".github/workflows/build-browseros.yml",
            pull_request_paths,
        )

    def test_build_system_tests_cross_git_bash_to_native_git_on_windows(self):
        test_workflow = self.load_workflow("bos-build-tests.yml")
        windows_job = test_workflow["jobs"]["windows-git-bootstrap"]
        verification_step = next(
            step
            for step in windows_job["steps"]
            if step.get("name") == "Verify Windows Git bootstrap"
        )

        self.assertEqual(windows_job["runs-on"], "windows-latest")
        self.assertEqual(verification_step["shell"], "bash")
        self.assertEqual(
            verification_step["working-directory"],
            "packages/browseros",
        )
        self.assertEqual(
            verification_step["run"],
            "uv run python -m unittest bos_build.ci_workflow_test -v",
        )


class ChromiumGitRunbookTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        runbook_path = (
            REPO_ROOT
            / "packages"
            / "browseros"
            / "bos_build"
            / "docs"
            / "warpbuild-ci.md"
        )
        cls.runbook = runbook_path.read_text(encoding="utf-8")

    def test_release_flow_puts_windows_git_bootstrap_before_source_ensure(self):
        release_flow = self.runbook.split(
            "## Release lane flow",
            maxsplit=1,
        )[1].split("## Caching strategy", maxsplit=1)[0]

        bootstrap_index = release_flow.index("`GIT_CONFIG_GLOBAL`")
        source_ensure_index = release_flow.index(
            "`browseros source ensure --step checkout`"
        )
        self.assertLess(bootstrap_index, source_ensure_index)
        self.assertIn(
            "`$RUNNER_TEMP/browseros-global.gitconfig`",
            release_flow,
        )

    def test_missing_global_config_failure_has_deterministic_recovery(self):
        heading = "## Troubleshooting: depot_tools cannot read global Git config"
        self.assertIn(heading, self.runbook)
        troubleshooting = self.runbook.split(heading, maxsplit=1)[1].split(
            "\n## ",
            maxsplit=1,
        )[0]

        self.assertIn(
            "C:/Users/runneradmin/.gitconfig",
            troubleshooting,
        )
        self.assertIn("gclient exit `9009`", troubleshooting)
        self.assertIn("PATH Git", troubleshooting)
        self.assertIn("depot_tools `git.bat`", troubleshooting)
        self.assertIn("`GIT_CONFIG_GLOBAL`", troubleshooting)
        self.assertIn("do not modify the runner image", troubleshooting)


if __name__ == "__main__":
    unittest.main()
