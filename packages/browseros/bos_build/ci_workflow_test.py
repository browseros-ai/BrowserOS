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


class ReleaseIntegrityWorkflowTest(unittest.TestCase):
    RELEASE_WORKFLOWS = (
        ("release-browseros.yml", "browseros", ""),
        ("release-browserclaw.yml", "browserclaw", "-claw"),
    )

    def load_workflow(self, workflow_name: str) -> dict[str, object]:
        path = WORKFLOW_DIR / workflow_name
        return yaml.safe_load(path.read_text(encoding="utf-8"))

    def named_step(
        self,
        workflow_name: str,
        job_name: str,
        step_name: str,
    ) -> dict[str, object]:
        workflow = self.load_workflow(workflow_name)
        return next(
            step
            for step in workflow["jobs"][job_name]["steps"]
            if step.get("name") == step_name
        )

    def run_shell(
        self,
        script: str,
        *,
        env: dict[str, str],
        cwd: Path,
    ) -> subprocess.CompletedProcess[str]:
        resolved_env = os.environ.copy()
        resolved_env.update(env)
        return subprocess.run(
            ["bash", "-c", script],
            cwd=cwd,
            env=resolved_env,
            capture_output=True,
            text=True,
        )

    def workflow_env(self, root: Path, product: str) -> dict[str, str]:
        output = root / "github-output"
        summary = root / "github-summary"
        calls = root / "uv-calls"
        runner_temp = root / "runner-temp"
        runner_temp.mkdir()
        return {
            "EXTENSION_NAME": (
                "browserclaw" if product == "browserclaw" else "agent"
            ),
            "EXTENSIONS_VERSION": "1.2.3",
            "FAKE_APPCAST_RC": "0",
            "FAKE_EXTENSIONS_RC": "0",
            "FAKE_UV_CALLS": str(calls),
            "GITHUB_OUTPUT": str(output),
            "GITHUB_RUN_ATTEMPT": "2",
            "GITHUB_RUN_ID": "30418029456",
            "GITHUB_SHA": "a" * 40,
            "GITHUB_STEP_SUMMARY": str(summary),
            "GITHUB_WORKSPACE": str(root),
            "INPUT_EXTENSIONS": "skip",
            "INPUT_GITHUB_RELEASE_DRAFT": "true",
            "INPUT_MACOS_ARCH": "universal",
            "INPUT_PLATFORMS": "all",
            "PRODUCT": product,
            "PRODUCT_LABEL": (
                "BrowserClaw" if product == "browserclaw" else "BrowserOS"
            ),
            "RUNNER_TEMP": str(runner_temp),
            "VERSION": "0.49.0",
        }

    def install_fake_uv(self, root: Path) -> Path:
        fake_bin = root / "fake-bin"
        fake_bin.mkdir()
        uv = fake_bin / "uv"
        uv.write_text(
            """#!/usr/bin/env bash
set -euo pipefail
printf '%s\\n' "$*" >> "$FAKE_UV_CALLS"
if [[ "$*" == *"release appcast"* ]]; then
  if [ "$FAKE_APPCAST_RC" -ne 0 ]; then
    exit "$FAKE_APPCAST_RC"
  fi
  for file in $FAKE_APPCAST_FILES; do
    mkdir -p bos_build/config/appcast
    printf '<rss/>\\n' > "bos_build/config/appcast/$file"
  done
elif [[ "$*" == *"release extensions"* ]]; then
  if [ "$FAKE_EXTENSIONS_RC" -ne 0 ]; then
    exit "$FAKE_EXTENSIONS_RC"
  fi
  for file in $FAKE_EXTENSION_FILES; do
    mkdir -p ../../updates/extensions
    printf '<feed/>\\n' > "../../updates/extensions/$file"
  done
fi
""",
            encoding="utf-8",
        )
        uv.chmod(0o755)
        return fake_bin

    def run_stage_script(
        self,
        workflow_name: str,
        product: str,
        appcast_files: list[str],
        *,
        appcast_rc: int = 0,
        extensions: str = "skip",
        extension_files: list[str] | None = None,
        extensions_rc: int = 0,
    ) -> tuple[subprocess.CompletedProcess[str], Path, dict[str, str]]:
        script = self.named_step(
            workflow_name,
            "stage_updates",
            "Render staged update feeds",
        )["run"]
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        workdir = root / "packages" / "browseros"
        workdir.mkdir(parents=True)
        fake_bin = self.install_fake_uv(root)
        env = self.workflow_env(root, product)
        extension_files = extension_files or []
        env.update(
            {
                "FAKE_APPCAST_FILES": " ".join(appcast_files),
                "FAKE_APPCAST_RC": str(appcast_rc),
                "FAKE_EXTENSION_FILES": " ".join(extension_files),
                "FAKE_EXTENSIONS_RC": str(extensions_rc),
                "INPUT_EXTENSIONS": extensions,
                "PATH": f"{fake_bin}{os.pathsep}{os.environ['PATH']}",
            }
        )
        return self.run_shell(script, env=env, cwd=workdir), root, env

    def test_literal_stage_scripts_emit_exact_three_appcasts_for_all_universal(self):
        for workflow_name, product, infix in self.RELEASE_WORKFLOWS:
            expected = [
                f"appcast{infix}.xml",
                f"appcast{infix}-x86_64.xml",
                f"appcast{infix}-win.xml",
            ]
            with self.subTest(workflow=workflow_name):
                result, root, env = self.run_stage_script(
                    workflow_name,
                    product,
                    expected,
                )

                self.assertEqual(
                    result.returncode,
                    0,
                    msg=f"stdout={result.stdout!r} stderr={result.stderr!r}",
                )
                staged = sorted(
                    path.name
                    for path in (root / "staged-update-feeds").rglob("*.xml")
                )
                self.assertEqual(staged, sorted(expected))
                self.assertIn(
                    "has_files=true",
                    Path(env["GITHUB_OUTPUT"]).read_text(encoding="utf-8"),
                )
                call = Path(env["FAKE_UV_CALLS"]).read_text(encoding="utf-8")
                for token in (
                    "--platforms all",
                    "--macos-arch universal",
                    f"--source-sha {'a' * 40}",
                    "--workflow-run-id 30418029456",
                    "--workflow-run-attempt 2",
                ):
                    self.assertIn(token, call)

    def test_literal_stage_scripts_fail_on_command_error_or_missing_output(self):
        for workflow_name, product, infix in self.RELEASE_WORKFLOWS:
            expected = [
                f"appcast{infix}.xml",
                f"appcast{infix}-x86_64.xml",
                f"appcast{infix}-win.xml",
            ]
            for label, files, rc in (
                ("command", expected, 19),
                ("missing", expected[:-1], 0),
            ):
                with self.subTest(workflow=workflow_name, failure=label):
                    result, _, _ = self.run_stage_script(
                        workflow_name,
                        product,
                        files,
                        appcast_rc=rc,
                    )
                    self.assertNotEqual(
                        result.returncode,
                        0,
                        msg=f"stdout={result.stdout!r} stderr={result.stderr!r}",
                    )

    def test_literal_stage_scripts_fail_on_extension_error_or_missing_output(self):
        extension_files = [
            "update-manifest.alpha.xml",
            "extensions.alpha.json",
            "bundled-manifest.xml",
        ]
        for workflow_name, product, infix in self.RELEASE_WORKFLOWS:
            appcast_files = [
                f"appcast{infix}.xml",
                f"appcast{infix}-x86_64.xml",
                f"appcast{infix}-win.xml",
            ]
            for label, files, rc in (
                ("command", extension_files, 23),
                ("missing", extension_files[:-1], 0),
            ):
                with self.subTest(workflow=workflow_name, failure=label):
                    result, _, _ = self.run_stage_script(
                        workflow_name,
                        product,
                        appcast_files,
                        extensions="alpha",
                        extension_files=files,
                        extensions_rc=rc,
                    )
                    self.assertNotEqual(
                        result.returncode,
                        0,
                        msg=f"stdout={result.stdout!r} stderr={result.stderr!r}",
                    )

    def test_literal_stage_gates_skip_linux_without_feed_surface(self):
        for workflow_name, product, _ in self.RELEASE_WORKFLOWS:
            script = self.named_step(
                workflow_name,
                "stage_updates",
                "Evaluate staged feed gate",
            )["run"]
            with tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                output = root / "output"
                env = {
                    **self.workflow_env(root, product),
                    "GITHUB_OUTPUT": str(output),
                    "INPUT_PLATFORMS": "linux",
                    "INPUT_EXTENSIONS": "skip",
                    "INPUT_INCLUDE_SERVERS": "false",
                    "INPUT_UPLOAD_TO_R2": "true",
                    "PREFLIGHT_RESULT": "success",
                    "ONBOARD_RESULT": "skipped",
                    "SERVER_RESULT": "skipped",
                    "LINUX_RESULT": "success",
                    "WINDOWS_RESULT": "skipped",
                    "MACOS_RESULT": "skipped",
                    "EXTENSIONS_RESULT": "skipped",
                }
                result = self.run_shell(script, env=env, cwd=root)
                gate = output.read_text(encoding="utf-8")

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("should_stage=false", gate)
            self.assertIn("no update feeds", gate)

    def test_literal_finalize_gates_require_successful_staging(self):
        for workflow_name, product, _ in self.RELEASE_WORKFLOWS:
            script = self.named_step(
                workflow_name,
                "finalize",
                "Evaluate draft release gate",
            )["run"]
            for stage_result, expected in (("failure", "false"), ("success", "true")):
                with self.subTest(
                    workflow=workflow_name,
                    stage_result=stage_result,
                ), tempfile.TemporaryDirectory() as tmp:
                    root = Path(tmp)
                    output = root / "output"
                    env = {
                        **self.workflow_env(root, product),
                        "GITHUB_OUTPUT": str(output),
                        "INPUT_PLATFORMS": "linux",
                        "INPUT_INCLUDE_SERVERS": "false",
                        "INPUT_UPLOAD_TO_R2": "true",
                        "PREFLIGHT_RESULT": "success",
                        "ONBOARD_RESULT": "skipped",
                        "SERVER_RESULT": "skipped",
                        "LINUX_RESULT": "success",
                        "WINDOWS_RESULT": "skipped",
                        "MACOS_RESULT": "skipped",
                        "EXTENSIONS_RESULT": "skipped",
                        "STAGE_UPDATES_RESULT": stage_result,
                    }
                    result = self.run_shell(script, env=env, cwd=root)

                    self.assertEqual(result.returncode, 0, result.stderr)
                    gate = output.read_text(encoding="utf-8")
                    self.assertIn(f"should_create={expected}", gate)
                    if stage_result == "failure":
                        self.assertIn("staged feed result is failure", gate)

    def test_release_workflows_pin_source_and_draft_to_workflow_sha(self):
        reusable = (WORKFLOW_DIR / "build-browseros.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn("ref: ${{ inputs.ref || github.sha }}", reusable)

        for workflow_name, _, _ in self.RELEASE_WORKFLOWS:
            workflow = (WORKFLOW_DIR / workflow_name).read_text(encoding="utf-8")
            self.assertNotIn("github.ref_name", workflow)
            self.assertNotIn("ref: ${{ github.ref }}", workflow)
            self.assertIn('gh release edit "$tag" --target "$GITHUB_SHA"', workflow)
            self.assertIn('--target "$GITHUB_SHA"', workflow)

    def test_top_level_release_changes_trigger_build_system_tests(self):
        workflow = self.load_workflow("bos-build-tests.yml")
        triggers = workflow.get("on", workflow.get(True))
        paths = triggers["pull_request"]["paths"]
        self.assertIn(".github/workflows/release-browseros.yml", paths)
        self.assertIn(".github/workflows/release-browserclaw.yml", paths)


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
