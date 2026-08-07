#!/usr/bin/env python3
"""Regression tests for the reusable Chromium build workflow."""

import os
import subprocess
import tempfile
import unittest
from dataclasses import replace
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
    REMOVED_RESOURCE_INPUTS = {
        "browseros_server_version",
        "browserclaw_server_version",
        "browserclaw_onboard_version",
        "bundled_extensions_manifest_url",
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

    def test_build_workflow_exposes_source_and_published_resource_modes(self):
        workflow = self.load_workflow("build-browseros.yml")
        triggers = workflow.get("on", workflow.get(True))
        inputs = triggers["workflow_call"]["inputs"]
        self.assertEqual(inputs["resource-mode"]["default"], "published")
        self.assertEqual(inputs["candidate-sha"]["default"], "")
        self.assertEqual(inputs["prepared-resources-artifact"]["default"], "")
        self.assertEqual(inputs["lane-artifact-name"]["default"], "")
        self.assertTrue(self.REMOVED_RESOURCE_INPUTS.isdisjoint(inputs))

        validate = next(
            step for step in self.build_steps() if step.get("name") == "Validate inputs"
        )
        self.assertIn("source mode requires a full candidate-sha", validate["run"])
        self.assertIn(
            "source mode requires prepared-resources-artifact", validate["run"]
        )

    def test_source_lane_checks_out_and_verifies_exact_candidate(self):
        steps = self.build_steps()
        checkout = next(
            step
            for step in steps
            if str(step.get("uses", "")).startswith("actions/checkout@")
        )
        verify = next(
            step
            for step in steps
            if step.get("name") == "Verify exact candidate checkout"
        )

        self.assertEqual(
            checkout["with"]["ref"],
            "${{ inputs.candidate-sha || inputs.ref || github.sha }}",
        )
        self.assertEqual(verify["if"], "inputs.resource-mode == 'source'")
        self.assertIn('git rev-parse HEAD)', verify["run"])

    def test_source_lane_downloads_common_resources_and_emits_attestation(self):
        steps = self.build_steps()
        download = next(
            step
            for step in steps
            if step.get("name") == "Download prepared common resources"
        )
        build_step = next(
            step
            for step in steps
            if step.get("name") == "Build ${{ inputs.product }}"
        )
        upload = next(
            step for step in steps if step.get("name") == "Upload lane manifest"
        )

        self.assertEqual(download["uses"], "actions/download-artifact@v8")
        self.assertEqual(
            download["with"]["name"], "${{ inputs.prepared-resources-artifact }}"
        )
        for value in (
            '--resource-mode "${{ inputs.resource-mode }}"',
            '--prepared-resources "$RUNNER_TEMP/prepared-resources"',
            '--lane-manifest "$RUNNER_TEMP/lane-manifest.json"',
            'uv run browseros build "${args[@]}"',
        ):
            self.assertIn(value, build_step["run"])
        self.assertEqual(upload["if"], "inputs.resource-mode == 'source'")
        self.assertEqual(upload["with"]["if-no-files-found"], "error")

    def test_reusable_platform_workflows_forward_source_contract(self):
        for workflow_name in ("release-linux.yml", "release-windows.yml"):
            with self.subTest(workflow=workflow_name):
                workflow = self.load_workflow(workflow_name)
                triggers = workflow.get("on", workflow.get(True))
                inputs = triggers["workflow_call"]["inputs"]
                dispatch_inputs = triggers["workflow_dispatch"]["inputs"]
                build_with = workflow["jobs"]["build"]["with"]

                self.assertEqual(inputs["resource_mode"]["default"], "published")
                self.assertTrue(
                    {"resource_mode", "candidate_sha", "prepared_resources_artifact"}.isdisjoint(
                        dispatch_inputs
                    )
                )
                self.assertTrue(self.REMOVED_RESOURCE_INPUTS.isdisjoint(inputs))
                self.assertEqual(
                    build_with["resource-mode"],
                    "${{ inputs.resource_mode || 'published' }}",
                )
                self.assertEqual(
                    build_with["candidate-sha"],
                    "${{ inputs.candidate_sha || '' }}",
                )
                self.assertEqual(
                    build_with["prepared-resources-artifact"],
                    "${{ inputs.prepared_resources_artifact || '' }}",
                )
                self.assertEqual(build_with["arch"], "x64")

    def test_persistent_macos_lane_uses_one_source_build(self):
        workflow = self.load_workflow("release-macos.yml")
        triggers = workflow.get("on", workflow.get(True))
        inputs = triggers["workflow_call"]["inputs"]
        dispatch_inputs = triggers["workflow_dispatch"]["inputs"]
        steps = workflow["jobs"]["build"]["steps"]
        build_step = next(
            step
            for step in steps
            if step.get("name") == "Build selected products"
        )
        sync = next(
            step for step in steps if step.get("name") == "Sync build repo to exact ref"
        )
        download = next(
            step
            for step in steps
            if step.get("name") == "Download prepared common resources"
        )
        upload = next(
            step for step in steps if step.get("name") == "Upload lane manifest"
        )

        self.assertEqual(inputs["arch"]["default"], "universal")
        self.assertEqual(inputs["resource_mode"]["default"], "published")
        self.assertTrue(
            {"resource_mode", "candidate_sha", "prepared_resources_artifact"}.isdisjoint(
                dispatch_inputs
            )
        )
        self.assertTrue(self.REMOVED_RESOURCE_INPUTS.isdisjoint(inputs))
        self.assertIn('git checkout --detach "$CANDIDATE_SHA"', sync["run"])
        self.assertIn('git rev-parse HEAD)', sync["run"])
        self.assertEqual(download["uses"], "actions/download-artifact@v8")
        self.assertEqual(build_step["run"].count("uv run browseros build"), 1)
        for value in (
            "--resource-mode",
            "--prepared-resources",
            "--lane-manifest",
        ):
            self.assertIn(value, build_step["run"])
        self.assertEqual(upload["with"]["if-no-files-found"], "error")

    def test_browser_lanes_do_not_receive_extension_build_secrets(self):
        secret_names = (
            "BROWSEROS_AGENT_V2_KEY",
            "BROWSERCLAW_KEY",
            "VITE_PUBLIC_SENTRY_DSN",
            "VITE_PUBLIC_POSTHOG_KEY",
            "VITE_CLAW_POSTHOG_KEY",
            "SENTRY_AUTH_TOKEN",
        )
        for workflow_name in (
            "build-browseros.yml",
            "release-linux.yml",
            "release-windows.yml",
            "release-macos.yml",
        ):
            text = (WORKFLOW_DIR / workflow_name).read_text(encoding="utf-8")
            with self.subTest(workflow=workflow_name):
                for secret_name in secret_names:
                    self.assertNotIn(secret_name, text)

    def test_git_bootstrap_follows_candidate_checkout_and_verification(self):
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
        verify_index = next(
            index
            for index, step in enumerate(steps)
            if step.get("name") == "Verify exact candidate checkout"
        )

        self.assertEqual(verify_index, checkout_index + 1)
        self.assertEqual(bootstrap_index, verify_index + 1)
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
    RELEASES = {
        "release-browseros.yml": {
            "product": "browseros",
            "extension_secret": "BROWSEROS_AGENT_V2_KEY",
            "other_extension_secret": "BROWSERCLAW_KEY",
            "components": {"server", "agent"},
        },
        "release-browserclaw.yml": {
            "product": "browserclaw",
            "extension_secret": "BROWSERCLAW_KEY",
            "other_extension_secret": "BROWSEROS_AGENT_V2_KEY",
            "components": {"claw-server-rust", "browserclaw"},
        },
    }

    def load_workflow(self, workflow_name: str) -> dict[str, object]:
        path = WORKFLOW_DIR / workflow_name
        return yaml.safe_load(path.read_text(encoding="utf-8"))

    def named_step(
        self,
        workflow: dict[str, object],
        job_name: str,
        step_name: str,
    ) -> dict[str, object]:
        return next(
            step
            for step in workflow["jobs"][job_name]["steps"]
            if step.get("name") == step_name
        )

    def test_full_releases_have_no_optional_dispatch_surface(self):
        for workflow_name in self.RELEASES:
            with self.subTest(workflow=workflow_name):
                workflow = self.load_workflow(workflow_name)
                triggers = workflow.get("on", workflow.get(True))
                self.assertIsNone(triggers["workflow_dispatch"])
                self.assertFalse(workflow["concurrency"]["cancel-in-progress"])
                self.assertEqual(workflow["permissions"], {})

    def test_candidate_freezes_default_branch_and_uses_python_lifecycle(self):
        for workflow_name, config in self.RELEASES.items():
            with self.subTest(workflow=workflow_name):
                workflow = self.load_workflow(workflow_name)
                candidate = workflow["jobs"]["candidate"]
                checkout = next(
                    step
                    for step in candidate["steps"]
                    if str(step.get("uses", "")).startswith("actions/checkout@")
                )
                ensure = self.named_step(
                    workflow, "candidate", "Create or recover candidate"
                )
                upload = self.named_step(
                    workflow, "candidate", "Upload candidate record"
                )

                self.assertEqual(
                    candidate["concurrency"]["group"],
                    "release-component-allocation",
                )
                self.assertEqual(
                    candidate["permissions"],
                    {"contents": "write", "pull-requests": "write"},
                )
                self.assertEqual(checkout["with"]["ref"], "${{ github.sha }}")
                self.assertEqual(checkout["with"]["fetch-depth"], 0)
                for token in (
                    "release candidate ensure",
                    f"--product {config['product']}",
                    '--parent-sha "$GITHUB_SHA"',
                    "--default-branch",
                    "--dispatch-ref",
                    "--github-summary",
                ):
                    self.assertIn(token, ensure["run"])
                self.assertIn(
                    f"release-candidate-{config['product']}-",
                    candidate["outputs"]["candidate_artifact"],
                )
                self.assertTrue(upload["with"]["overwrite"])
                self.assertEqual(upload["with"]["retention-days"], 30)

    def test_candidate_products_own_only_their_versioned_components(self):
        from bos_build.release.components import components_for_candidate

        for config in self.RELEASES.values():
            components = {
                component.id
                for component in components_for_candidate(config["product"])
            }
            self.assertEqual(components, config["components"])
            self.assertNotIn("claw-onboard", components)

    def test_common_resources_are_recoverable_and_candidate_bound(self):
        for workflow_name, config in self.RELEASES.items():
            with self.subTest(workflow=workflow_name):
                workflow = self.load_workflow(workflow_name)
                common = workflow["jobs"]["common"]
                checkout = next(
                    step
                    for step in common["steps"]
                    if str(step.get("uses", "")).startswith("actions/checkout@")
                )
                recover = self.named_step(
                    workflow, "common", "Recover prepared common resources"
                )
                prepare = self.named_step(
                    workflow, "common", "Prepare or validate common resources"
                )
                upload = self.named_step(
                    workflow, "common", "Upload prepared common resources"
                )

                self.assertEqual(common["needs"], "candidate")
                self.assertEqual(
                    common["permissions"],
                    {"actions": "read", "contents": "read"},
                )
                self.assertEqual(
                    checkout["with"]["ref"],
                    "${{ needs.candidate.outputs.candidate_sha }}",
                )
                self.assertIn("actions/artifacts?name=$ARTIFACT_NAME", recover["run"])
                self.assertIn("gh run download", recover["run"])
                self.assertIn("release resources prepare", prepare["run"])
                self.assertIn('--candidate "$RUNNER_TEMP/candidate/candidate.json"', prepare["run"])
                self.assertIn(
                    config["extension_secret"],
                    prepare["env"],
                )
                self.assertNotIn(config["other_extension_secret"], prepare["env"])
                self.assertEqual(
                    upload["with"]["name"],
                    "${{ needs.candidate.outputs.common_artifact }}",
                )
                self.assertTrue(upload["with"]["overwrite"])
                self.assertEqual(upload["with"]["retention-days"], 30)

    def test_full_release_matrix_is_fixed_and_uses_source_mode(self):
        lanes = {
            "linux": ("release-linux.yml", None),
            "windows": ("release-windows.yml", None),
            "macos": ("release-macos.yml", "universal"),
        }
        for workflow_name, config in self.RELEASES.items():
            workflow = self.load_workflow(workflow_name)
            jobs = workflow["jobs"]
            for job_name, (called_workflow, arch) in lanes.items():
                with self.subTest(workflow=workflow_name, lane=job_name):
                    job = jobs[job_name]
                    self.assertNotIn("if", job)
                    self.assertEqual(set(job["needs"]), {"candidate", "common"})
                    self.assertEqual(
                        job["uses"], f"./.github/workflows/{called_workflow}"
                    )
                    self.assertEqual(job["with"]["products"], config["product"])
                    self.assertEqual(job["with"]["resource_mode"], "source")
                    self.assertIs(job["with"]["upload_to_r2"], True)
                    self.assertEqual(
                        job["with"]["candidate_sha"],
                        "${{ needs.candidate.outputs.candidate_sha }}",
                    )
                    self.assertEqual(
                        job["with"]["prepared_resources_artifact"],
                        "${{ needs.candidate.outputs.common_artifact }}",
                    )
                    if job_name == "windows":
                        self.assertIs(job["with"]["sign"], True)
                    if arch is not None:
                        self.assertEqual(job["with"]["arch"], arch)

                    serialized_secrets = yaml.safe_dump(job["secrets"])
                    self.assertNotEqual(job["secrets"], "inherit")
                    if job_name == "linux":
                        self.assertNotIn("ESIGNER_", serialized_secrets)
                        self.assertNotIn("MACOS_", serialized_secrets)
                        self.assertNotIn("SPARKLE_PRIVATE_KEY", serialized_secrets)
                    elif job_name == "windows":
                        self.assertIn("ESIGNER_USERNAME", serialized_secrets)
                        self.assertNotIn("MACOS_", serialized_secrets)
                    else:
                        self.assertIn("MACOS_CERTIFICATE_NAME", serialized_secrets)
                        self.assertNotIn("ESIGNER_", serialized_secrets)

    def test_gate_merge_and_finalization_are_strictly_ordered(self):
        for workflow_name, config in self.RELEASES.items():
            with self.subTest(workflow=workflow_name):
                workflow = self.load_workflow(workflow_name)
                jobs = workflow["jobs"]
                self.assertEqual(
                    jobs["gate"]["permissions"],
                    {"actions": "read", "contents": "read"},
                )
                self.assertEqual(
                    jobs["merge"]["permissions"],
                    {
                        "actions": "read",
                        "contents": "write",
                        "pull-requests": "write",
                    },
                )
                self.assertEqual(
                    jobs["finalize"]["permissions"],
                    {"actions": "read", "contents": "write"},
                )
                self.assertEqual(
                    set(jobs["gate"]["needs"]),
                    {"candidate", "linux", "windows", "macos"},
                )
                gate = self.named_step(
                    workflow, "gate", "Validate complete native matrix"
                )
                self.assertIn("release browser gate", gate["run"])
                self.assertIn('ne 3', gate["run"])
                lane_download = self.named_step(
                    workflow, "gate", "Download lane manifests"
                )
                self.assertEqual(
                    lane_download["with"]["pattern"],
                    f"lane-{config['product']}-*",
                )

                self.assertEqual(set(jobs["merge"]["needs"]), {"candidate", "gate"})
                merge = self.named_step(workflow, "merge", "Merge exact candidate")
                self.assertIn("release candidate merge", merge["run"])
                self.assertIn("--github-summary", merge["run"])
                merged_upload = self.named_step(
                    workflow, "merge", "Upload merged candidate record"
                )
                self.assertTrue(merged_upload["with"]["overwrite"])

                self.assertEqual(
                    set(jobs["finalize"]["needs"]),
                    {"candidate", "gate", "merge"},
                )
                finalize = self.named_step(
                    workflow, "finalize", "Create or refresh browser draft"
                )
                self.assertIn("release browser finalize", finalize["run"])
                self.assertIn("--preview-dir", finalize["run"])
                self.assertIn("--github-summary", finalize["run"])
                preview = self.named_step(
                    workflow, "finalize", "Upload finalization previews"
                )
                self.assertIn(
                    f"release-finalization-{config['product']}-",
                    preview["with"]["name"],
                )

    def test_all_component_allocators_share_one_preparation_lock(self):
        workflows = (
            "release-browseros.yml",
            "release-browserclaw.yml",
            "release-server.yml",
            "release-claw-server.yml",
            "release-claw-onboard.yml",
            "release-extensions.yml",
        )
        for workflow_name in workflows:
            workflow = self.load_workflow(workflow_name)
            job_name = "candidate" if "candidate" in workflow["jobs"] else "prepare"
            with self.subTest(workflow=workflow_name):
                self.assertEqual(
                    workflow["jobs"][job_name]["concurrency"]["group"],
                    "release-component-allocation",
                )

    def test_full_workflows_have_no_component_publication_choreography(self):
        removed = (
            "release-server.yml",
            "release-claw-server.yml",
            "release-claw-onboard.yml",
            "release-extensions.yml",
            "defer_finalize",
            "release.plan",
            "release publish",
            "release appcast",
            "release extensions",
            "browseros_server_version",
            "browserclaw_server_version",
            "browserclaw_onboard_version",
            "bundled_extensions_manifest_url",
        )
        for workflow_name in self.RELEASES:
            text = (WORKFLOW_DIR / workflow_name).read_text(encoding="utf-8")
            with self.subTest(workflow=workflow_name):
                for token in removed:
                    self.assertNotIn(token, text)

    def test_extension_build_secrets_exist_only_in_common_producer(self):
        extension_secrets = {
            "BROWSEROS_AGENT_V2_KEY",
            "BROWSERCLAW_KEY",
            "VITE_PUBLIC_SENTRY_DSN",
            "VITE_PUBLIC_POSTHOG_KEY",
            "VITE_CLAW_POSTHOG_KEY",
            "SENTRY_AUTH_TOKEN",
        }
        for workflow_name in self.RELEASES:
            workflow = self.load_workflow(workflow_name)
            common_env = self.named_step(
                workflow, "common", "Prepare or validate common resources"
            )["env"]
            for job_name, job in workflow["jobs"].items():
                if job_name == "common" or "steps" not in job:
                    continue
                serialized = yaml.safe_dump(job)
                with self.subTest(workflow=workflow_name, job=job_name):
                    for secret in extension_secrets:
                        if secret in common_env:
                            self.assertNotIn(secret, serialized)

    def test_macos_product_artifact_globs_do_not_overlap(self):
        workflow = self.load_workflow("release-macos.yml")
        upload_steps = {
            step["name"]: step
            for step in workflow["jobs"]["build"]["steps"]
            if str(step.get("uses", "")).startswith("actions/upload-artifact@")
        }
        browseros_path = upload_steps["Upload BrowserOS DMG artifact"]["with"]["path"]
        neo_path = upload_steps["Upload BrowserOS neo DMG artifact"]["with"]["path"]

        self.assertIn("/BrowserOS_*.dmg\n!", browseros_path)
        self.assertIn("/BrowserOS_neo_*.dmg", browseros_path)
        self.assertTrue(neo_path.endswith("/BrowserOS_neo_*.dmg"))

    def test_top_level_release_changes_trigger_build_system_tests(self):
        workflow = self.load_workflow("bos-build-tests.yml")
        triggers = workflow.get("on", workflow.get(True))
        paths = triggers["pull_request"]["paths"]
        self.assertIn(".github/workflows/release-browseros.yml", paths)
        self.assertIn(".github/workflows/release-browserclaw.yml", paths)


class NightlyWorkflowTest(unittest.TestCase):
    NIGHTLIES = {
        "nightly-browseros.yml": {
            "product": "browseros",
            "build_step": "Build BrowserOS (macOS arm64)",
            "extension_secret": "BROWSEROS_AGENT_V2_KEY",
            "server_secret": "BROWSEROS_CONFIG_URL",
            "tag": "nightly-browseros",
        },
        "nightly-browserclaw.yml": {
            "product": "browserclaw",
            "build_step": "Build BrowserOS neo (macOS arm64)",
            "extension_secret": "BROWSERCLAW_KEY",
            "server_secret": "CLAW_POSTHOG_KEY",
            "tag": "nightly-browserclaw",
        },
    }

    def load_workflow(self, workflow_name: str) -> dict[str, object]:
        path = WORKFLOW_DIR / workflow_name
        return yaml.safe_load(path.read_text(encoding="utf-8"))

    def test_nightlies_use_one_source_mode_build_without_inline_staging(self):
        removed = (
            "scripts/build/server.ts",
            "claw-server-rust-local.sh",
            "extract_artifact_zip",
            "Stage BrowserOS",
            "release candidate",
            "release component",
        )
        for workflow_name, config in self.NIGHTLIES.items():
            workflow = self.load_workflow(workflow_name)
            text = (WORKFLOW_DIR / workflow_name).read_text(encoding="utf-8")
            build = next(
                step
                for step in workflow["jobs"]["build"]["steps"]
                if step.get("name") == config["build_step"]
            )
            with self.subTest(workflow=workflow_name):
                self.assertEqual(text.count("uv run browseros build"), 1)
                for token in removed:
                    self.assertNotIn(token, text)
                for token in (
                    "--profile nightly-macos",
                    f"--product {config['product']}",
                    "--arch arm64",
                    '--source-sha "$(git rev-parse HEAD)"',
                ):
                    self.assertIn(token, build["run"])
                self.assertIn(config["extension_secret"], build["env"])
                self.assertIn(config["server_secret"], build["env"])
                self.assertIn("SPARKLE_PRIVATE_KEY", build["env"])
                self.assertIn("R2_SECRET_ACCESS_KEY", build["env"])

    def test_nightlies_preserve_browser_version_and_rolling_release_behavior(self):
        for workflow_name, config in self.NIGHTLIES.items():
            workflow = self.load_workflow(workflow_name)
            steps = workflow["jobs"]["build"]["steps"]
            text = (WORKFLOW_DIR / workflow_name).read_text(encoding="utf-8")
            rolling = next(
                step for step in steps if step.get("name", "").startswith("Update rolling")
            )
            with self.subTest(workflow=workflow_name):
                self.assertIn(config["tag"], rolling["run"])
                self.assertIn("gh release create", rolling["run"])
                self.assertIn("actions/upload-artifact@v7", text)
                self.assertNotIn("Cargo.lock", text)
                self.assertNotIn("bun.lock", text)
                self.assertNotIn("release candidate", text)

    def test_nightly_profile_prepares_common_and_server_once(self):
        from bos_build.core.planner import load_profile, plan

        profile = load_profile(
            REPO_ROOT
            / "packages/browseros/bos_build/profiles/nightly-macos.yaml"
        ).switches
        for product in ("browseros", "browserclaw"):
            switches = replace(profile, product=product).resolved()
            steps = plan(switches, "arm64", "macos")
            with self.subTest(product=product):
                self.assertEqual(steps.count("prepare_common_resources"), 1)
                self.assertEqual(steps.count("prepare_server_resources"), 1)
                self.assertNotIn("download_resources", steps)


class ReleaseDocumentationTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        docs = REPO_ROOT / "packages/browseros/bos_build"
        cls.readme = (docs / "README.md").read_text(encoding="utf-8")
        cls.release = (docs / "docs/release-ci.md").read_text(encoding="utf-8")
        cls.nightly = (docs / "docs/nightly-macos-ci.md").read_text(
            encoding="utf-8"
        )
        cls.warp = (docs / "docs/warpbuild-ci.md").read_text(encoding="utf-8")

    def test_release_runbook_covers_source_and_recovery_commands(self):
        for token in (
            "browseros release candidate ensure",
            "browseros release resources prepare",
            "--prepared-resources",
            "--resource-mode source",
            'gh run rerun "$RUN_ID" --failed',
            "browseros release browser finalize",
            "--resource-mode published",
        ):
            with self.subTest(token=token):
                self.assertIn(token, self.release)

    def test_runbooks_state_native_host_and_publication_boundaries(self):
        for token in (
            "One machine cannot produce the complete signed release matrix",
            "Linux x64",
            "Windows x64",
            "macOS arm64, universal plan",
            "Component OTA remains a separate operation",
        ):
            with self.subTest(token=token):
                self.assertIn(token, self.release)
        self.assertIn("resource_mode: source", self.nightly)
        self.assertIn("resource-mode=source", self.warp)

    def test_primary_docs_do_not_describe_retired_resource_staging(self):
        text = "\n".join((self.readme, self.release, self.nightly, self.warp))
        for token in (
            "bundle_local_extensions",
            "extensions_version",
            "include_servers",
            "Stage BrowserOS nightly resources",
            "claw-server-rust-local.sh",
        ):
            with self.subTest(token=token):
                self.assertNotIn(token, text)


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
