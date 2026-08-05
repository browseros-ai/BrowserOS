#!/usr/bin/env python3
"""Regression tests for the reusable Chromium build workflow."""

import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import yaml

from bos_build.steps.source import provision


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
    RESOURCE_INPUT_ENV = {
        "browseros_server_version": "BROWSEROS_SERVER_RESOURCE_VERSION",
        "browserclaw_server_version": "BROWSERCLAW_SERVER_RESOURCE_VERSION",
        "browserclaw_onboard_version": "BROWSERCLAW_ONBOARD_RESOURCE_VERSION",
        "bundled_extensions_manifest_url": "BUNDLED_EXTENSIONS_MANIFEST_URL",
    }

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

    def test_build_workflow_exposes_resource_inputs_to_build_environment(self):
        workflow = self.load_workflow("build-browseros.yml")
        triggers = workflow.get("on", workflow.get(True))
        inputs = triggers["workflow_call"]["inputs"]
        build_step = next(
            step
            for step in self.build_steps()
            if step.get("name") == "Build ${{ inputs.product }}"
        )

        for input_name, env_name in self.RESOURCE_INPUT_ENV.items():
            with self.subTest(input=input_name):
                self.assertEqual(
                    {"required": False, "type": "string", "default": ""},
                    {
                        key: inputs[input_name][key]
                        for key in ("required", "type", "default")
                    },
                )
                self.assertEqual(
                    f"${{{{ inputs.{input_name} }}}}",
                    build_step["env"][env_name],
                )

    def test_reusable_platform_workflows_forward_resource_inputs(self):
        for workflow_name in ("release-linux.yml", "release-windows.yml"):
            with self.subTest(workflow=workflow_name):
                workflow = self.load_workflow(workflow_name)
                triggers = workflow.get("on", workflow.get(True))
                inputs = triggers["workflow_call"]["inputs"]
                build_with = workflow["jobs"]["build"]["with"]

                for input_name in self.RESOURCE_INPUT_ENV:
                    self.assertFalse(inputs[input_name]["required"])
                    self.assertEqual(inputs[input_name]["default"], "")
                    self.assertEqual(inputs[input_name]["type"], "string")
                    self.assertEqual(
                        f"${{{{ inputs.{input_name} }}}}",
                        build_with[input_name],
                    )

    def test_persistent_macos_build_receives_resource_inputs(self):
        workflow = self.load_workflow("release-macos.yml")
        triggers = workflow.get("on", workflow.get(True))
        inputs = triggers["workflow_call"]["inputs"]
        build_step = next(
            step
            for step in workflow["jobs"]["build"]["steps"]
            if step.get("name") == "Build selected products"
        )

        for input_name, env_name in self.RESOURCE_INPUT_ENV.items():
            with self.subTest(input=input_name):
                self.assertFalse(inputs[input_name]["required"])
                self.assertEqual(inputs[input_name]["default"], "")
                self.assertEqual(inputs[input_name]["type"], "string")
                self.assertEqual(
                    f"${{{{ inputs.{input_name} }}}}",
                    build_step["env"][env_name],
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

    def test_source_ensure_explicitly_repairs_disposable_depot_tools_cache(self):
        steps = self.build_steps()

        for phase in (
            "Ensure chromium checkout at pinned tag",
            "Sync chromium dependencies (gclient)",
        ):
            with self.subTest(phase=phase):
                step = next(step for step in steps if step.get("name") == phase)
                self.assertIn("--repair-cached-depot-tools", step["run"])

    def test_checkout_cache_uses_v2_without_v1_fallback(self):
        steps = self.build_steps()
        pin_step = next(
            step
            for step in steps
            if step.get("name") == "Resolve chromium pin and paths"
        )
        warp_restore = next(
            step
            for step in steps
            if step.get("name") == "Restore chromium checkout (WarpCache)"
        )

        self.assertIn(
            "chromium-src-${{ inputs.platform }}-${{ inputs.arch }}-v2-$version",
            pin_step["run"],
        )
        self.assertIn(
            "chromium-src-${{ inputs.platform }}-${{ inputs.arch }}-v2-",
            warp_restore["with"]["restore-keys"],
        )
        self.assertNotIn("-v1-", pin_step["run"])
        self.assertNotIn("-v1-", warp_restore["with"]["restore-keys"])

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
            'printf \'GIT_CONFIG_GLOBAL=%s\\n\' "$git_config" >> "$GITHUB_ENV"',
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

    @unittest.skipUnless(os.name == "nt", "requires Git for Windows")
    def test_cached_crlf_depot_tools_repairs_before_native_batch_self_update(self):
        script = self.git_bootstrap_step()["run"]

        with tempfile.TemporaryDirectory(
            prefix="browseros depot tools cache ",
        ) as tmp:
            temp_root = Path(tmp)
            legacy_config = temp_root / "legacy-global.gitconfig"
            legacy_env = os.environ.copy()
            legacy_env["GIT_CONFIG_GLOBAL"] = str(legacy_config)
            subprocess.run(
                ["git", "config", "--global", "core.autocrlf", "true"],
                check=True,
                env=legacy_env,
            )

            seed = temp_root / "seed"
            seed.mkdir()
            subprocess.run(
                ["git", "init", "--initial-branch=main"],
                check=True,
                cwd=seed,
                env=legacy_env,
            )
            subprocess.run(
                ["git", "config", "user.name", "BrowserOS test"],
                check=True,
                cwd=seed,
                env=legacy_env,
            )
            subprocess.run(
                ["git", "config", "user.email", "ci@example.invalid"],
                check=True,
                cwd=seed,
                env=legacy_env,
            )
            tracked = seed / "gclient.py"
            tracked.write_bytes(b"first line\nsecond line\n")
            subprocess.run(
                ["git", "add", tracked.name],
                check=True,
                cwd=seed,
                env=legacy_env,
            )
            subprocess.run(
                ["git", "commit", "-m", "initial"],
                check=True,
                cwd=seed,
                env=legacy_env,
            )

            origin = temp_root / "origin.git"
            subprocess.run(
                ["git", "clone", "--bare", str(seed), str(origin)],
                check=True,
                env=legacy_env,
            )
            root = temp_root / "chromium"
            root.mkdir()
            depot_tools = root / "depot_tools"
            subprocess.run(
                ["git", "clone", str(origin), str(depot_tools)],
                check=True,
                env=legacy_env,
            )
            cached_bytes = (depot_tools / tracked.name).read_bytes()
            self.assertIn(b"\r\n", cached_bytes)

            tracked.write_bytes(b"first line\nupstream second line\n")
            subprocess.run(
                ["git", "add", tracked.name],
                check=True,
                cwd=seed,
                env=legacy_env,
            )
            subprocess.run(
                ["git", "commit", "-m", "upstream update"],
                check=True,
                cwd=seed,
                env=legacy_env,
            )
            subprocess.run(
                ["git", "push", str(origin), "main"],
                check=True,
                cwd=seed,
                env=legacy_env,
            )

            runner_temp = temp_root / "runner temp"
            runner_temp.mkdir()
            github_env = temp_root / "github env"
            missing_home = temp_root / "missing home"
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
            _, config_path = (
                github_env.read_text(encoding="utf-8")
                .strip()
                .split(
                    "=",
                    maxsplit=1,
                )
            )
            native_env = os.environ.copy()
            native_env.update(
                {
                    "GIT_CONFIG_GLOBAL": config_path,
                    "GITHUB_ENV": str(github_env),
                    "GITHUB_PATH": str(temp_root / "github path"),
                    "HOME": str(missing_home),
                }
            )

            # R2 extraction restores working bytes independently of Git's
            # index stat cache. Rewriting the legacy bytes models that cache
            # boundary and forces native Git to inspect their line endings.
            (depot_tools / tracked.name).write_bytes(cached_bytes)
            dirty = subprocess.run(
                [
                    "git",
                    "status",
                    "--porcelain=v1",
                    "--untracked-files=no",
                ],
                capture_output=True,
                check=True,
                cwd=depot_tools,
                env=native_env,
                text=True,
            )
            self.assertIn(tracked.name, dirty.stdout)
            subprocess.run(
                ["git", "fetch", "origin"],
                check=True,
                cwd=depot_tools,
                env=native_env,
            )
            blocked = subprocess.run(
                ["git", "merge", "--ff-only", "origin/main"],
                capture_output=True,
                cwd=depot_tools,
                env=native_env,
                text=True,
            )
            self.assertNotEqual(blocked.returncode, 0)
            self.assertIn(
                "local changes",
                (blocked.stdout + blocked.stderr).lower(),
            )

            with mock.patch.dict(os.environ, native_env, clear=True):
                provision.ensure_depot_tools(
                    root,
                    repair_cached_depot_tools=True,
                )

            git_wrapper = depot_tools / "git-wrapper.bat"
            git_wrapper.write_bytes(b"@echo off\r\ngit %*\r\n")
            command = subprocess.list2cmdline(
                [git_wrapper.name, "merge", "--ff-only", "origin/main"]
            )
            subprocess.run(
                ["cmd.exe", "/d", "/c", command],
                check=True,
                cwd=depot_tools,
                env=native_env,
            )
            git_wrapper.unlink()

            clean = subprocess.run(
                ["git", "status", "--porcelain=v1"],
                capture_output=True,
                check=True,
                cwd=depot_tools,
                env=native_env,
                text=True,
            )
            self.assertEqual(clean.stdout, "")
            self.assertIn(
                b"upstream second line\n",
                (depot_tools / tracked.name).read_bytes(),
            )

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
    FULL_RELEASE_CONFIG = {
        "release-browseros.yml": {
            "product": "browseros",
            "extension": "agent",
            "builds": {
                "build_server": "release-server.yml",
                "build_extension": "release-extensions.yml",
            },
            "finalizers": {
                "finalize_server": "release-server.yml",
                "finalize_extension": "release-extensions.yml",
            },
            "pins": {
                "browseros_server_version": "browseros_server_version",
                "browserclaw_server_version": "browserclaw_server_version",
                "browserclaw_onboard_version": "browserclaw_onboard_version",
                "bundled_extensions_manifest_url": "manifest_url",
            },
        },
        "release-browserclaw.yml": {
            "product": "browserclaw",
            "extension": "browserclaw",
            "builds": {
                "build_onboarding": "release-claw-onboard.yml",
                "build_server": "release-claw-server-rust.yml",
                "build_extension": "release-extensions.yml",
            },
            "finalizers": {
                "finalize_onboarding": "release-claw-onboard.yml",
                "finalize_server": "release-claw-server-rust.yml",
                "finalize_extension": "release-extensions.yml",
            },
            "pins": {
                "browseros_server_version": "browseros_server_version",
                "browserclaw_server_version": "browserclaw_server_version",
                "browserclaw_onboard_version": "browserclaw_onboard_version",
                "bundled_extensions_manifest_url": "manifest_url",
            },
        },
    }

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
            [git_bash_path(), "-c", script],
            cwd=cwd,
            env=resolved_env,
            capture_output=True,
            text=True,
        )

    def test_full_release_dispatch_uses_automatic_extension_versions(self):
        for workflow_name, config in self.FULL_RELEASE_CONFIG.items():
            with self.subTest(workflow=workflow_name):
                workflow = self.load_workflow(workflow_name)
                triggers = workflow.get("on", workflow.get(True))
                inputs = triggers["workflow_dispatch"]["inputs"]

                self.assertNotIn("extensions_version", inputs)
                self.assertEqual(inputs["extensions"]["default"], "alpha")
                self.assertIn(
                    "browser draft",
                    inputs["github_release_draft"]["description"].lower(),
                )

                text = (WORKFLOW_DIR / workflow_name).read_text(encoding="utf-8")
                self.assertNotIn("inputs.extensions_version", text)
                self.assertNotIn("extensions_version is required", text)
                self.assertIn(f"extension: {config['extension']}", text)

    def test_full_release_preflight_freezes_default_branch_source(self):
        for workflow_name in self.FULL_RELEASE_CONFIG:
            with self.subTest(workflow=workflow_name):
                workflow = self.load_workflow(workflow_name)
                preflight = workflow["jobs"]["preflight"]
                self.assertIn("source_sha", preflight["outputs"])
                source_step = next(
                    step
                    for step in preflight["steps"]
                    if step.get("name") == "Validate default branch source"
                )
                self.assertIn("refs/heads/$DEFAULT_BRANCH", source_step["run"])
                self.assertIn("source_sha=$GITHUB_SHA", source_step["run"])
                self.assertIn("github.ref", source_step["env"]["DISPATCH_REF"])

    def test_component_builds_are_deferred_and_plan_validates_optional_results(self):
        for workflow_name, config in self.FULL_RELEASE_CONFIG.items():
            with self.subTest(workflow=workflow_name):
                jobs = self.load_workflow(workflow_name)["jobs"]
                for job_name, called_workflow in config["builds"].items():
                    job = jobs[job_name]
                    self.assertEqual(
                        job["uses"], f"./.github/workflows/{called_workflow}"
                    )
                    self.assertEqual(job["with"]["mode"], "build")
                    self.assertTrue(job["with"]["defer_finalize"])
                    source_input = "branch" if job_name == "build_extension" else "ref"
                    self.assertEqual(
                        job["with"][source_input],
                        "${{ needs.preflight.outputs.source_sha }}",
                    )
                    if job_name == "build_extension":
                        self.assertNotIn("version", job["with"])
                    else:
                        self.assertEqual(job["with"].get("publish_ota", False), False)

                release_plan = jobs["release_plan"]
                self.assertIn("always()", release_plan["if"])
                self.assertEqual(
                    set(release_plan["needs"]),
                    {"preflight", *config["builds"]},
                )
                gate = next(
                    step
                    for step in release_plan["steps"]
                    if step.get("name") == "Validate prepared components"
                )
                self.assertIn("selected component", gate["run"])
                self.assertIn("unexpectedly ran", gate["run"])
                self.assertIn("source mismatch", gate["run"])

    def test_every_browser_call_receives_every_resource_plan_pin(self):
        all_pins = {
            "browseros_server_version",
            "browserclaw_server_version",
            "browserclaw_onboard_version",
            "bundled_extensions_manifest_url",
        }
        for workflow_name, config in self.FULL_RELEASE_CONFIG.items():
            jobs = self.load_workflow(workflow_name)["jobs"]
            for job_name in ("release_linux", "release_windows", "release_macos"):
                with self.subTest(workflow=workflow_name, job=job_name):
                    job = jobs[job_name]
                    self.assertEqual(job["needs"], "release_plan")
                    passed_pins = all_pins.intersection(job["with"])
                    self.assertEqual(passed_pins, set(config["pins"]))
                    for input_name, output_name in config["pins"].items():
                        self.assertEqual(
                            job["with"][input_name],
                            f"${{{{ needs.release_plan.outputs.{output_name} }}}}",
                        )
                    if job_name == "release_macos":
                        self.assertEqual(
                            job["with"]["ref"],
                            "${{ needs.release_plan.outputs.source_sha }}",
                        )

    def test_release_plan_always_snapshots_manifest_and_outputs_all_resource_pins(
        self,
    ):
        expected_outputs = {
            "browseros_server_version",
            "browserclaw_server_version",
            "browserclaw_onboard_version",
        }
        for workflow_name, _config in self.FULL_RELEASE_CONFIG.items():
            with self.subTest(workflow=workflow_name):
                plan = self.load_workflow(workflow_name)["jobs"]["release_plan"]
                self.assertTrue(expected_outputs.issubset(plan["outputs"]))
                download = next(
                    step
                    for step in plan["steps"]
                    if step.get("name") == "Download bundled extension manifest"
                )
                self.assertNotIn("if", download)
                create = next(
                    step
                    for step in plan["steps"]
                    if step.get("name") == "Create immutable release plan"
                )
                self.assertIn(
                    '--bundled-manifest "$RUNNER_TEMP/bundled-manifest.xml"',
                    create["run"],
                )

    def test_only_final_success_summary_contains_promotion_commands(self):
        for workflow_name, _config in self.FULL_RELEASE_CONFIG.items():
            with self.subTest(workflow=workflow_name):
                stage = self.named_step(
                    workflow_name, "stage_updates", "Render staged update feeds"
                )["run"]
                summary = self.named_step(
                    workflow_name, "finalize", "Write release summary"
                )["run"]
                self.assertNotIn("release publish --version", stage)
                self.assertNotIn("--publish", stage)
                guard = summary.index('if [ "$promotion_ready" = "true" ]')
                self.assertGreater(summary.index("release publish --version"), guard)
                self.assertGreater(summary.index("release appcast --version"), guard)
                self.assertGreater(summary.index("release extensions --channel"), guard)
                self.assertIn("browser-gate", summary)
                self.assertIn("-finalize", summary)
                self.assertIn("Promotion commands withheld", summary)

    def test_browser_gate_and_finalizers_enforce_ordering(self):
        for workflow_name, config in self.FULL_RELEASE_CONFIG.items():
            with self.subTest(workflow=workflow_name):
                jobs = self.load_workflow(workflow_name)["jobs"]
                browser_gate = jobs["browser_gate"]
                self.assertIn("always()", browser_gate["if"])
                self.assertEqual(
                    set(browser_gate["needs"]),
                    {
                        "release_plan",
                        "release_linux",
                        "release_windows",
                        "release_macos",
                    },
                )
                gate_script = next(
                    step
                    for step in browser_gate["steps"]
                    if step.get("name") == "Validate selected browser builds"
                )["run"]
                self.assertIn("selected platform", gate_script)
                self.assertIn("unselected platform", gate_script)

                stage = jobs["stage_updates"]
                self.assertIn("always()", stage["if"])
                self.assertIn("browser_gate", stage["needs"])
                for job_name, called_workflow in config["finalizers"].items():
                    finalizer = jobs[job_name]
                    self.assertEqual(
                        finalizer["uses"], f"./.github/workflows/{called_workflow}"
                    )
                    self.assertIn("stage_updates", finalizer["needs"])
                    self.assertIn("browser_gate", finalizer["needs"])
                    self.assertNotIn("github_release_draft", finalizer["if"])
                    self.assertEqual(finalizer["with"]["mode"], "finalize")
                    source_input = (
                        "branch" if job_name == "finalize_extension" else "ref"
                    )
                    self.assertEqual(
                        finalizer["with"][source_input],
                        "${{ needs.release_plan.outputs.source_sha }}",
                    )

                finalize = jobs["finalize"]
                self.assertTrue(set(config["finalizers"]).issubset(finalize["needs"]))

    def test_release_plan_and_feed_artifacts_are_retry_unique_and_canonical(self):
        for workflow_name in self.FULL_RELEASE_CONFIG:
            with self.subTest(workflow=workflow_name):
                workflow = self.load_workflow(workflow_name)
                plan_upload = next(
                    step
                    for step in workflow["jobs"]["release_plan"]["steps"]
                    if str(step.get("uses", "")).startswith("actions/upload-artifact@")
                )
                plan_script = next(
                    step
                    for step in workflow["jobs"]["release_plan"]["steps"]
                    if step.get("name") == "Create immutable release plan"
                )["run"]
                self.assertIn('--run-attempt "$GITHUB_RUN_ATTEMPT"', plan_script)
                self.assertIn("github.run_id", plan_upload["with"]["name"])
                self.assertIn("github.run_attempt", plan_upload["with"]["name"])

                stage = workflow["jobs"]["stage_updates"]
                script = next(
                    step
                    for step in stage["steps"]
                    if step.get("name") == "Render staged update feeds"
                )["run"]
                for value in (
                    "$GITHUB_WORKSPACE/updates/browser",
                    "$GITHUB_WORKSPACE/updates/extensions",
                    '"updates/browser/$file"',
                    '"updates/extensions/$file"',
                ):
                    self.assertIn(value, script)
                self.assertNotIn("bos_build/config/appcast", script)
                self.assertNotIn("--allow-downgrade", script)
                self.assertNotIn("--repair-invalid-live", script)
                artifact = next(
                    step
                    for step in stage["steps"]
                    if str(step.get("uses", "")).startswith("actions/upload-artifact@")
                )
                self.assertIn("github.run_id", artifact["with"]["name"])
                self.assertIn("github.run_attempt", artifact["with"]["name"])

                metadata_scripts = [
                    step["run"]
                    for job in workflow["jobs"].values()
                    for step in job.get("steps", [])
                    if "run" in step
                    and any(
                        command in step["run"]
                        for command in (
                            "release appcast",
                            "release publish",
                            "release github create",
                        )
                    )
                ]
                self.assertTrue(metadata_scripts)
                for metadata_script in metadata_scripts:
                    self.assertIn("--source-sha", metadata_script)
                    self.assertIn("--workflow-run-id", metadata_script)
                    self.assertNotIn("--workflow-run-attempt", metadata_script)

    def test_macos_product_artifact_globs_do_not_overlap(self):
        workflow = self.load_workflow("release-macos.yml")
        upload_steps = {
            step["name"]: step
            for step in workflow["jobs"]["build"]["steps"]
            if str(step.get("uses", "")).startswith("actions/upload-artifact@")
        }
        browseros_path = upload_steps["Upload BrowserOS DMG artifact"]["with"][
            "path"
        ]
        neo_path = upload_steps["Upload BrowserOS neo DMG artifact"]["with"]["path"]

        self.assertIn("/BrowserOS_*.dmg\n!", browseros_path)
        self.assertIn("/BrowserOS_neo_*.dmg", browseros_path)
        self.assertTrue(neo_path.endswith("/BrowserOS_neo_*.dmg"))

    def test_literal_component_gate_truth_table(self):
        sha = "a" * 40
        products = {
            "release-browseros.yml": {
                "components": (
                    ("SERVER", "browseros-server", "0.0.22", "agent-server/v0.0.22"),
                    (
                        "EXTENSION",
                        "agent-extension",
                        "0.0.124.0",
                        "ext-agent/v0.0.124.0",
                    ),
                ),
            },
            "release-browserclaw.yml": {
                "components": (
                    ("ONBOARD", "browserclaw-onboard", "0.0.4", "claw-onboard/v0.0.4"),
                    ("SERVER", "browserclaw-server", "0.0.22", "claw-server/v0.0.22"),
                    (
                        "EXTENSION",
                        "browserclaw-extension",
                        "0.1.8.0",
                        "ext-browserclaw/v0.1.8.0",
                    ),
                ),
            },
        }

        for workflow_name, config in products.items():
            script = self.named_step(
                workflow_name,
                "release_plan",
                "Validate prepared components",
            )["run"]

            def run_case(overrides: dict[str, str]):
                temp = tempfile.TemporaryDirectory()
                self.addCleanup(temp.cleanup)
                root = Path(temp.name)
                output = root / "output"
                env = {
                    "GITHUB_OUTPUT": str(output),
                    "SOURCE_SHA": sha,
                }
                for prefix, _, _, _ in config["components"]:
                    env.update(
                        {
                            f"{prefix}_SELECTED": "false",
                            f"{prefix}_RESULT": "skipped",
                            f"{prefix}_VERSION": "",
                            f"{prefix}_TAG": "",
                            f"{prefix}_RELEASE_SHA": "",
                        }
                    )
                env.update(overrides)
                return self.run_shell(script, env=env, cwd=root), output

            result, output = run_case({})
            self.assertEqual(result.returncode, 0, result.stderr)
            normalized = output.read_text(encoding="utf-8")
            self.assertIn("server_version=\n", normalized)
            self.assertIn("extension_version=\n", normalized)

            all_selected = {}
            for prefix, _, version, tag in config["components"]:
                all_selected.update(
                    {
                        f"{prefix}_SELECTED": "true",
                        f"{prefix}_RESULT": "success",
                        f"{prefix}_VERSION": version,
                        f"{prefix}_TAG": tag,
                        f"{prefix}_RELEASE_SHA": sha,
                    }
                )
            result, _ = run_case(all_selected)
            self.assertEqual(result.returncode, 0, result.stderr)

            for prefix, _, _, _ in config["components"]:
                for bad_result in ("skipped", "failure", "cancelled"):
                    with self.subTest(
                        workflow=workflow_name,
                        component=prefix,
                        result=bad_result,
                    ):
                        overrides = dict(all_selected)
                        overrides[f"{prefix}_RESULT"] = bad_result
                        result, _ = run_case(overrides)
                        self.assertNotEqual(result.returncode, 0)

            first_prefix, _, first_version, first_tag = config["components"][0]
            result, _ = run_case(
                {
                    f"{first_prefix}_RESULT": "success",
                    f"{first_prefix}_VERSION": first_version,
                    f"{first_prefix}_TAG": first_tag,
                    f"{first_prefix}_RELEASE_SHA": sha,
                }
            )
            self.assertNotEqual(result.returncode, 0)

            mismatch = dict(all_selected)
            mismatch[f"{first_prefix}_RELEASE_SHA"] = "b" * 40
            result, _ = run_case(mismatch)
            self.assertNotEqual(result.returncode, 0)

    def test_literal_browser_gate_truth_table(self):
        for workflow_name in self.FULL_RELEASE_CONFIG:
            script = self.named_step(
                workflow_name,
                "browser_gate",
                "Validate selected browser builds",
            )["run"]

            def run_case(
                platforms: str, linux: str, windows: str, macos: str, plan="success"
            ):
                with tempfile.TemporaryDirectory() as temp_dir:
                    return self.run_shell(
                        script,
                        env={
                            "INPUT_PLATFORMS": platforms,
                            "LINUX_RESULT": linux,
                            "MACOS_RESULT": macos,
                            "PLAN_RESULT": plan,
                            "WINDOWS_RESULT": windows,
                        },
                        cwd=Path(temp_dir),
                    )

            for case in (
                ("all", "success", "success", "success"),
                ("linux", "success", "skipped", "skipped"),
                ("windows", "skipped", "success", "skipped"),
                ("macos", "skipped", "skipped", "success"),
            ):
                with self.subTest(workflow=workflow_name, case=case):
                    result = run_case(*case)
                    self.assertEqual(result.returncode, 0, result.stderr)

            for bad_result in ("skipped", "failure", "cancelled"):
                with self.subTest(workflow=workflow_name, selected=bad_result):
                    result = run_case("linux", bad_result, "skipped", "skipped")
                    self.assertNotEqual(result.returncode, 0)

            unselected = run_case("linux", "success", "success", "skipped")
            self.assertNotEqual(unselected.returncode, 0)
            failed_plan = run_case(
                "linux", "success", "skipped", "skipped", plan="failure"
            )
            self.assertNotEqual(failed_plan.returncode, 0)

    def test_literal_preflight_rejects_non_default_branch_source(self):
        script = self.named_step(
            "release-browseros.yml",
            "preflight",
            "Validate default branch source",
        )["run"]
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            subprocess.run(
                ["git", "init", "--initial-branch=main"],
                cwd=root,
                check=True,
                capture_output=True,
            )
            subprocess.run(
                ["git", "config", "user.name", "Release test"], cwd=root, check=True
            )
            subprocess.run(
                ["git", "config", "user.email", "release@example.invalid"],
                cwd=root,
                check=True,
            )
            (root / "tracked").write_text("source\n", encoding="utf-8")
            subprocess.run(["git", "add", "tracked"], cwd=root, check=True)
            subprocess.run(
                ["git", "commit", "-m", "source"],
                cwd=root,
                check=True,
                capture_output=True,
            )
            sha = subprocess.run(
                ["git", "rev-parse", "HEAD"],
                cwd=root,
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()
            output = root / "output"
            base_env = {
                "DEFAULT_BRANCH": "main",
                "GITHUB_OUTPUT": str(output),
                "GITHUB_SHA": sha,
            }
            accepted = self.run_shell(
                script,
                env={**base_env, "DISPATCH_REF": "refs/heads/main"},
                cwd=root,
            )
            rejected = self.run_shell(
                script,
                env={**base_env, "DISPATCH_REF": "refs/heads/feature"},
                cwd=root,
            )

        self.assertEqual(accepted.returncode, 0, accepted.stderr)
        self.assertNotEqual(rejected.returncode, 0)
        self.assertIn("must be dispatched", rejected.stdout + rejected.stderr)

    def test_browser_draft_gate_blocks_selected_finalizer_failure(self):
        for workflow_name, product, _ in self.RELEASE_WORKFLOWS:
            script = self.named_step(
                workflow_name,
                "finalize",
                "Evaluate draft release gate",
            )["run"]
            with (
                self.subTest(workflow=workflow_name),
                tempfile.TemporaryDirectory() as temp_dir,
            ):
                root = Path(temp_dir)
                output = root / "output"
                env = {
                    **self.workflow_env(root, product),
                    "EXTENSION_FINALIZE_RESULT": "failure",
                    "GITHUB_OUTPUT": str(output),
                    "INPUT_EXTENSIONS": "alpha",
                }
                result = self.run_shell(script, env=env, cwd=root)
                gate = output.read_text(encoding="utf-8")

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("should_create=false", gate)
            self.assertIn("extension-finalize result is failure", gate)

    def workflow_env(self, root: Path, product: str) -> dict[str, str]:
        output = root / "github-output"
        summary = root / "github-summary"
        calls = root / "uv-calls"
        runner_temp = root / "runner-temp"
        runner_temp.mkdir()
        return {
            "EXTENSION_NAME": ("browserclaw" if product == "browserclaw" else "agent"),
            "EXTENSIONS_VERSION": "1.2.3",
            "FAKE_APPCAST_RC": "0",
            "FAKE_EXTENSIONS_RC": "0",
            "FAKE_UV_CALLS": str(calls),
            "GITHUB_OUTPUT": str(output),
            "GITHUB_RUN_ATTEMPT": "2",
            "GITHUB_RUN_ID": "30418029456",
            "GITHUB_REPOSITORY": "browseros-ai/BrowserOS",
            "GITHUB_SHA": "a" * 40,
            "GITHUB_STEP_SUMMARY": str(summary),
            "GITHUB_WORKSPACE": str(root),
            "INPUT_EXTENSIONS": "skip",
            "INPUT_GITHUB_RELEASE_DRAFT": "true",
            "INPUT_INCLUDE_SERVERS": "false",
            "INPUT_MACOS_ARCH": "universal",
            "INPUT_PLATFORMS": "all",
            "INPUT_UPLOAD_TO_R2": "true",
            "EXTENSION_FINALIZE_RESULT": "skipped",
            "ONBOARD_FINALIZE_RESULT": "skipped",
            "PLAN_RESULT": "success",
            "PREFLIGHT_RESULT": "success",
            "PRODUCT": product,
            "PRODUCT_LABEL": (
                "BrowserOS neo" if product == "browserclaw" else "BrowserOS"
            ),
            "RUNNER_TEMP": str(runner_temp),
            "SERVER_FINALIZE_RESULT": "skipped",
            "SOURCE_SHA": "a" * 40,
            "STAGE_UPDATES_RESULT": "success",
            "VERSION": "0.49.0",
        }

    def install_fake_uv(self, root: Path) -> Path:
        fake_bin = root / "fake-bin"
        fake_bin.mkdir(exist_ok=True)
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
    mkdir -p ../../updates/browser
    printf '<rss/>\\n' > "../../updates/browser/$file"
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

    def install_fake_gh(self, root: Path) -> Path:
        fake_bin = root / "fake-bin"
        fake_bin.mkdir(exist_ok=True)
        gh = fake_bin / "gh"
        gh.write_text(
            """#!/usr/bin/env bash
set -euo pipefail
printf '%s\\n' "$*" >> "$FAKE_GH_CALLS"
if [ "$1" = "api" ] && [[ "$*" == *"/releases/tags/"* ]]; then
  printf '%s\\n' "$FAKE_RELEASE_PROBE"
  exit "$FAKE_RELEASE_RC"
fi
if [ "$1" = "api" ] && [[ "$*" == *"/commits/"* ]]; then
  printf '%s\\n' "$FAKE_TAG_PROBE"
  exit "$FAKE_TAG_RC"
fi
""",
            encoding="utf-8",
        )
        gh.chmod(0o755)
        return fake_bin

    def api_probe(self, status: int, body: str) -> str:
        return f"HTTP/2.0 {status} Status\nContent-Type: application/json\n\n{body}"

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
        platforms: str = "all",
        upload_to_r2: bool = True,
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
                "INPUT_PLATFORMS": platforms,
                "INPUT_UPLOAD_TO_R2": str(upload_to_r2).lower(),
                "PATH": f"{fake_bin}{os.pathsep}{os.environ['PATH']}",
            }
        )
        return self.run_shell(script, env=env, cwd=workdir), root, env

    def run_finalize_script(
        self,
        workflow_name: str,
        product: str,
        *,
        release_probe: str,
        release_rc: int,
        tag_probe: str = "",
        tag_rc: int = 1,
    ) -> tuple[subprocess.CompletedProcess[str], Path, dict[str, str]]:
        script = self.named_step(
            workflow_name,
            "finalize",
            "Create or refresh draft GitHub release",
        )["run"]
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        workdir = root / "packages" / "browseros"
        workdir.mkdir(parents=True)
        fake_bin = self.install_fake_uv(root)
        self.install_fake_gh(root)
        env = self.workflow_env(root, product)
        env.update(
            {
                "FAKE_APPCAST_FILES": "",
                "FAKE_EXTENSION_FILES": "",
                "FAKE_GH_CALLS": str(root / "gh-calls"),
                "FAKE_RELEASE_PROBE": release_probe,
                "FAKE_RELEASE_RC": str(release_rc),
                "FAKE_TAG_PROBE": tag_probe,
                "FAKE_TAG_RC": str(tag_rc),
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
                artifact_root = root / "staged-update-feeds"
                staged = sorted(
                    path.relative_to(artifact_root).as_posix()
                    for path in artifact_root.rglob("*.xml")
                )
                self.assertEqual(
                    staged,
                    sorted(f"updates/browser/{name}" for name in expected),
                )
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
                ):
                    self.assertIn(token, call)
                self.assertNotIn("--workflow-run-attempt", call)

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
            self.assertIn("no browser appcast or extension feed", gate)

    def test_linux_extension_only_staging_ignores_browser_r2_switch(self):
        extension_files = [
            "update-manifest.alpha.xml",
            "extensions.alpha.json",
            "bundled-manifest.xml",
        ]
        for workflow_name, product, _ in self.RELEASE_WORKFLOWS:
            gate_script = self.named_step(
                workflow_name,
                "stage_updates",
                "Evaluate staged feed gate",
            )["run"]
            with (
                self.subTest(workflow=workflow_name),
                tempfile.TemporaryDirectory() as tmp,
            ):
                gate_root = Path(tmp)
                gate_output = gate_root / "output"
                gate_env = {
                    **self.workflow_env(gate_root, product),
                    "GITHUB_OUTPUT": str(gate_output),
                    "INPUT_PLATFORMS": "linux",
                    "INPUT_EXTENSIONS": "alpha",
                    "INPUT_INCLUDE_SERVERS": "false",
                    "INPUT_UPLOAD_TO_R2": "false",
                    "PREFLIGHT_RESULT": "success",
                    "ONBOARD_RESULT": "skipped",
                    "SERVER_RESULT": "skipped",
                    "LINUX_RESULT": "success",
                    "WINDOWS_RESULT": "skipped",
                    "MACOS_RESULT": "skipped",
                    "EXTENSIONS_RESULT": "success",
                }
                gate_result = self.run_shell(
                    gate_script,
                    env=gate_env,
                    cwd=gate_root,
                )
                gate = gate_output.read_text(encoding="utf-8")

            self.assertEqual(gate_result.returncode, 0, gate_result.stderr)
            self.assertIn("should_stage=true", gate)

            result, root, env = self.run_stage_script(
                workflow_name,
                product,
                [],
                extensions="alpha",
                extension_files=extension_files,
                platforms="linux",
                upload_to_r2=False,
            )
            self.assertEqual(
                result.returncode,
                0,
                msg=f"stdout={result.stdout!r} stderr={result.stderr!r}",
            )
            artifact_root = root / "staged-update-feeds"
            staged = sorted(
                path.relative_to(artifact_root).as_posix()
                for path in artifact_root.rglob("*")
                if path.is_file()
            )
            self.assertEqual(
                staged,
                sorted(f"updates/extensions/{name}" for name in extension_files),
            )
            calls = Path(env["FAKE_UV_CALLS"]).read_text(encoding="utf-8")
            self.assertNotIn("release appcast", calls)
            self.assertIn("release extensions", calls)
            summary = Path(env["GITHUB_STEP_SUMMARY"]).read_text(encoding="utf-8")
            self.assertIn("withheld until the final release gate", summary)
            self.assertNotIn("release publish", summary)
            self.assertNotIn("--publish", summary)

    def test_partial_stage_summary_withholds_promotion_commands(self):
        for workflow_name, product, infix in self.RELEASE_WORKFLOWS:
            expected = [f"appcast{infix}-win.xml"]
            with self.subTest(workflow=workflow_name):
                result, _, env = self.run_stage_script(
                    workflow_name,
                    product,
                    expected,
                    platforms="windows",
                )

                self.assertEqual(
                    result.returncode,
                    0,
                    msg=f"stdout={result.stdout!r} stderr={result.stderr!r}",
                )
                summary = Path(env["GITHUB_STEP_SUMMARY"]).read_text(encoding="utf-8")
                self.assertIn("withheld until the final release gate", summary)
                self.assertNotIn("release publish", summary)
                self.assertNotIn("release appcast", summary)
                self.assertNotIn("release extensions", summary)
                self.assertNotIn("--publish", summary)

    def test_literal_finalize_gates_require_successful_staging(self):
        for workflow_name, product, _ in self.RELEASE_WORKFLOWS:
            script = self.named_step(
                workflow_name,
                "finalize",
                "Evaluate draft release gate",
            )["run"]
            for stage_result, expected in (("failure", "false"), ("success", "true")):
                with (
                    self.subTest(
                        workflow=workflow_name,
                        stage_result=stage_result,
                    ),
                    tempfile.TemporaryDirectory() as tmp,
                ):
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
                        self.assertIn("staged-feed result is failure", gate)

    def test_finalize_release_inspection_and_tag_target_fail_closed(self):
        sha = "a" * 40
        other_sha = "b" * 40
        cases = (
            (
                "inspection-error",
                self.api_probe(500, '{"message":"server error"}'),
                1,
                "",
                1,
                "Could not inspect release",
            ),
            (
                "published",
                self.api_probe(
                    200,
                    f'{{"draft":false,"target_commitish":"{sha}"}}',
                ),
                0,
                "",
                1,
                "not a draft",
            ),
            (
                "tag-mismatch",
                self.api_probe(
                    200,
                    f'{{"draft":true,"target_commitish":"{sha}"}}',
                ),
                0,
                self.api_probe(200, f'{{"sha":"{other_sha}"}}'),
                0,
                "not workflow SHA",
            ),
            (
                "orphaned-tag-mismatch",
                self.api_probe(404, '{"message":"Not Found"}'),
                1,
                self.api_probe(200, f'{{"sha":"{other_sha}"}}'),
                0,
                "not workflow SHA",
            ),
            (
                "unrelated-tag-lookup-error",
                self.api_probe(404, '{"message":"Not Found"}'),
                1,
                self.api_probe(422, '{"message":"Validation Failed"}'),
                1,
                "Could not resolve release tag",
            ),
        )
        for workflow_name, product, _ in self.RELEASE_WORKFLOWS:
            for label, release, release_rc, tag, tag_rc, error in cases:
                with self.subTest(workflow=workflow_name, failure=label):
                    result, root, _ = self.run_finalize_script(
                        workflow_name,
                        product,
                        release_probe=release,
                        release_rc=release_rc,
                        tag_probe=tag,
                        tag_rc=tag_rc,
                    )

                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(error, result.stdout + result.stderr)
                    self.assertFalse((root / "uv-calls").exists())

    def test_finalize_accepts_verified_absence_or_matching_draft_target(self):
        sha = "a" * 40
        for workflow_name, product, _ in self.RELEASE_WORKFLOWS:
            expected_tag = (
                "browserclaw/v0.49.0" if product == "browserclaw" else "v0.49.0"
            )
            missing_tag = self.api_probe(
                422,
                f'{{"message":"No commit found for SHA: {expected_tag}"}}',
            )
            cases = (
                (
                    "absent",
                    self.api_probe(404, '{"message":"Not Found"}'),
                    1,
                    missing_tag,
                    1,
                ),
                (
                    "matching-draft",
                    self.api_probe(
                        200,
                        f'{{"draft":true,"target_commitish":"{sha}"}}',
                    ),
                    0,
                    self.api_probe(200, f'{{"sha":"{sha}"}}'),
                    0,
                ),
                (
                    "matching-untagged-draft",
                    self.api_probe(
                        200,
                        f'{{"draft":true,"target_commitish":"{sha}"}}',
                    ),
                    0,
                    missing_tag,
                    1,
                ),
            )
            for label, release, release_rc, tag, tag_rc in cases:
                with self.subTest(workflow=workflow_name, state=label):
                    result, root, env = self.run_finalize_script(
                        workflow_name,
                        product,
                        release_probe=release,
                        release_rc=release_rc,
                        tag_probe=tag,
                        tag_rc=tag_rc,
                    )

                    self.assertEqual(
                        result.returncode,
                        0,
                        msg=f"stdout={result.stdout!r} stderr={result.stderr!r}",
                    )
                    uv_calls = Path(env["FAKE_UV_CALLS"]).read_text(encoding="utf-8")
                    for token in (
                        "release github create",
                        "--version 0.49.0",
                        "--draft",
                        f"--product {product}",
                        "--platforms all",
                        "--macos-arch universal",
                        f"--source-sha {sha}",
                        "--workflow-run-id 30418029456",
                        f"--target {sha}",
                    ):
                        self.assertIn(token, uv_calls)
                    self.assertNotIn("--workflow-run-attempt", uv_calls)
                    gh_calls = (root / "gh-calls").read_text(encoding="utf-8")
                    expected_tag_uri = expected_tag.replace("/", "%2F")
                    self.assertIn(
                        f"/releases/tags/{expected_tag_uri}",
                        gh_calls,
                    )
                    self.assertNotIn("release edit", gh_calls)

    def test_finalize_never_retargets_an_untagged_draft(self):
        for workflow_name, product, _ in self.RELEASE_WORKFLOWS:
            expected_tag = (
                "browserclaw/v0.49.0" if product == "browserclaw" else "v0.49.0"
            )
            missing_tag = self.api_probe(
                422,
                f'{{"message":"No commit found for SHA: {expected_tag}"}}',
            )

            result, root, _ = self.run_finalize_script(
                workflow_name,
                product,
                release_probe=self.api_probe(
                    200,
                    '{"draft":true,"target_commitish":"old"}',
                ),
                release_rc=0,
                tag_probe=missing_tag,
                tag_rc=1,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn(
                "Refusing to retarget a draft",
                result.stdout + result.stderr,
            )
            self.assertFalse((root / "uv-calls").exists())
            gh_calls = (root / "gh-calls").read_text(encoding="utf-8")
            self.assertNotIn("release edit", gh_calls)

    def test_release_workflows_pin_source_and_draft_to_workflow_sha(self):
        reusable = (WORKFLOW_DIR / "build-browseros.yml").read_text(encoding="utf-8")
        self.assertIn("ref: ${{ inputs.ref || github.sha }}", reusable)

        for workflow_name, _, _ in self.RELEASE_WORKFLOWS:
            workflow = (WORKFLOW_DIR / workflow_name).read_text(encoding="utf-8")
            self.assertNotIn("github.ref_name", workflow)
            self.assertNotIn("ref: ${{ github.ref }}", workflow)
            self.assertIn('actual_target" != "$SOURCE_SHA"', workflow)
            self.assertIn('release_status" = "404"', workflow)
            self.assertIn('--target "$SOURCE_SHA"', workflow)
            self.assertIn(
                "SOURCE_SHA: ${{ needs.release_plan.outputs.source_sha }}",
                workflow,
            )

        browserclaw = (WORKFLOW_DIR / "release-browserclaw.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn('echo "browserclaw/v$major.$minor.$build"', browserclaw)

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
            "`browseros source ensure --step checkout --repair-cached-depot-tools`"
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

    def test_dirty_depot_tools_cache_failure_has_fail_closed_recovery(self):
        heading = "## Troubleshooting: cached depot_tools appears all-dirty"
        self.assertIn(heading, self.runbook)
        troubleshooting = self.runbook.split(heading, maxsplit=1)[1].split(
            "\n## ",
            maxsplit=1,
        )[0]

        self.assertIn("Your local changes", troubleshooting)
        self.assertIn("Failed to update depot_tools", troubleshooting)
        self.assertIn("line-ending-only", troubleshooting)
        self.assertIn("substantive tracked changes", troubleshooting)
        self.assertIn("non-default", troubleshooting)
        self.assertIn("index flags", troubleshooting)
        self.assertIn("`--repair-cached-depot-tools`", troubleshooting)
        self.assertIn("`v2`", troubleshooting)


if __name__ == "__main__":
    unittest.main()
