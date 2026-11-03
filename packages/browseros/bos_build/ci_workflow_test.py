#!/usr/bin/env python3
"""Regression tests for the reusable Chromium build workflow."""

import json
import os
import re
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import yaml

from bos_build.release.feeds.spec import all_feeds
from bos_build.steps.source import provision


REPO_ROOT = Path(__file__).resolve().parents[3]
WORKFLOW_DIR = REPO_ROOT / ".github" / "workflows"
MACOS_SIGNING_HELPER = REPO_ROOT / ".github" / "scripts" / "macos-signing-keychain.sh"
MACOS_CHROMIUM_WORKSPACE_HELPER = (
    REPO_ROOT / ".github" / "scripts" / "macos-chromium-workspace.sh"
)
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


# Neo's browser draft gate, whitespace-normalized. Kept literal so the
# behavioral test below evaluates exactly what the workflow ships.
NEO_FINALIZE_CONDITION = (
    "${{ !cancelled() "
    "&& needs.preflight.result == 'success' "
    "&& needs.server.result == 'success' "
    "&& needs.extension.result == 'success' "
    "&& needs.components.result == 'success' "
    "&& needs.linux.result == (needs.preflight.outputs.build_linux == 'true' "
    "&& 'success' || 'skipped') "
    "&& needs.windows.result == (needs.preflight.outputs.build_windows == 'true' "
    "&& 'success' || 'skipped') "
    "&& needs.macos.result == (needs.preflight.outputs.build_macos == 'true' "
    "&& 'success' || 'skipped') }}"
)


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
        self.assertEqual(inputs["server-version"]["default"], "")
        self.assertEqual(inputs["extension-version"]["default"], "")
        self.assertEqual(inputs["onboarding-version"]["default"], "")
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
        self.assertIn("git rev-parse HEAD)", verify["run"])

    def test_source_lane_downloads_common_resources_and_emits_attestation(self):
        steps = self.build_steps()
        download = next(
            step
            for step in steps
            if step.get("name") == "Download prepared common resources"
        )
        build_step = next(
            step for step in steps if step.get("name") == "Build ${{ inputs.product }}"
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
        self.assertEqual(
            build_step["env"]["ONBOARDING_RESOURCE_VERSION"],
            "${{ inputs.onboarding-version }}",
        )
        self.assertNotIn("BROWSERCLAW_ONBOARD_RESOURCE_VERSION", build_step["env"])

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
                    {
                        "resource_mode",
                        "candidate_sha",
                        "prepared_resources_artifact",
                    }.isdisjoint(dispatch_inputs)
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
                self.assertEqual(
                    build_with["server-version"],
                    "${{ inputs.server_version || '' }}",
                )
                self.assertEqual(
                    build_with["extension-version"],
                    "${{ inputs.extension_version || '' }}",
                )
                self.assertEqual(
                    build_with["onboarding-version"],
                    "${{ inputs.onboarding_version || '' }}",
                )
                self.assertEqual(
                    build_with["arch"],
                    "${{ matrix.arch }}"
                    if workflow_name == "release-linux.yml"
                    else "x64",
                )

    def test_persistent_macos_lane_uses_one_source_build(self):
        workflow = self.load_workflow("release-macos.yml")
        triggers = workflow.get("on", workflow.get(True))
        inputs = triggers["workflow_call"]["inputs"]
        dispatch_inputs = triggers["workflow_dispatch"]["inputs"]
        steps = workflow["jobs"]["build"]["steps"]
        build_step = next(
            step for step in steps if step.get("name") == "Build selected products"
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
        stale_dmg_cleanup = next(
            step
            for step in steps
            if step.get("name") == "Remove stale selected-product DMGs"
        )

        self.assertEqual(inputs["arch"]["default"], "universal")
        self.assertEqual(inputs["resource_mode"]["default"], "published")
        self.assertTrue(
            {
                "resource_mode",
                "candidate_sha",
                "prepared_resources_artifact",
            }.isdisjoint(dispatch_inputs)
        )
        self.assertTrue(self.REMOVED_RESOURCE_INPUTS.isdisjoint(inputs))
        self.assertIn('git checkout --detach "$CANDIDATE_SHA"', sync["run"])
        self.assertIn('git checkout --detach "$TARGET_REF"', sync["run"])
        self.assertIn("git rev-parse HEAD)", sync["run"])
        self.assertEqual(download["uses"], "actions/download-artifact@v8")
        self.assertEqual(build_step["run"].count("uv run browseros build"), 1)
        self.assertIn("-name 'BrowserOS_*.dmg'", stale_dmg_cleanup["run"])
        self.assertIn("-name 'BrowserOS_neo_*.dmg'", stale_dmg_cleanup["run"])
        for value in (
            "--resource-mode",
            "--prepared-resources",
            "--lane-manifest",
        ):
            self.assertIn(value, build_step["run"])
        self.assertEqual(upload["with"]["if-no-files-found"], "error")
        self.assertEqual(
            build_step["env"]["ONBOARDING_RESOURCE_VERSION"],
            "${{ inputs.onboarding_version }}",
        )
        self.assertNotIn("BROWSERCLAW_ONBOARD_RESOURCE_VERSION", build_step["env"])

    def test_macos_release_sets_up_ci_keychain_before_build_and_cleans_up(self):
        workflow = self.load_workflow("release-macos.yml")
        triggers = workflow.get("on", workflow.get(True))
        secrets = triggers["workflow_call"]["secrets"]
        steps = workflow["jobs"]["build"]["steps"]
        setup_index = next(
            index
            for index, step in enumerate(steps)
            if step.get("name") == "Import macOS signing certificate"
        )
        build_index = next(
            index
            for index, step in enumerate(steps)
            if step.get("name") == "Build selected products"
        )
        cleanup_index = next(
            index
            for index, step in enumerate(steps)
            if step.get("name") == "Clean up macOS signing keychain"
        )
        upload_index = next(
            index
            for index, step in enumerate(steps)
            if step.get("name") == "Upload BrowserOS DMG artifact"
        )
        setup = steps[setup_index]
        build = steps[build_index]
        cleanup = steps[cleanup_index]

        self.assertIn("MACOS_CERTIFICATE_P12", secrets)
        self.assertIn("MACOS_CERTIFICATE_PWD", secrets)
        self.assertIn("PROD_MACOS_BROWSEROS_PASSKEY_PROFILE_B64", secrets)
        self.assertIn("PROD_MACOS_BROWSERCLAW_PASSKEY_PROFILE_B64", secrets)
        self.assertLess(setup_index, build_index)
        self.assertLess(build_index, cleanup_index)
        self.assertLess(cleanup_index, upload_index)
        self.assertEqual(setup["id"], "macos_signing")
        self.assertIn("macos-signing-keychain.sh setup", setup["run"])
        for name in (
            "MACOS_CERTIFICATE_NAME",
            "MACOS_CERTIFICATE_P12",
            "MACOS_CERTIFICATE_PWD",
            "MACOS_KEYCHAIN_PASSWORD",
            "PROD_MACOS_BROWSEROS_PASSKEY_PROFILE_B64",
            "PROD_MACOS_BROWSERCLAW_PASSKEY_PROFILE_B64",
        ):
            self.assertEqual(setup["env"][name], f"${{{{ secrets.{name} }}}}")
        self.assertEqual(
            build["env"]["MACOS_KEYCHAIN_PATH"],
            "${{ steps.macos_signing.outputs.keychain_path }}",
        )
        self.assertEqual(
            build["env"]["MACOS_CERTIFICATE_NAME"],
            "${{ steps.macos_signing.outputs.codesign_identity }}",
        )
        self.assertEqual(
            build["env"]["PROD_MACOS_BROWSEROS_PASSKEY_PROFILE_PATH"],
            "${{ steps.macos_signing.outputs.browseros_passkey_profile_path }}",
        )
        self.assertEqual(
            build["env"]["PROD_MACOS_BROWSERCLAW_PASSKEY_PROFILE_PATH"],
            "${{ steps.macos_signing.outputs.browserclaw_passkey_profile_path }}",
        )
        self.assertEqual(cleanup["if"], "always()")
        self.assertEqual(
            cleanup["env"]["MACOS_SIGNING_STATE_PATH"],
            "${{ steps.macos_signing.outputs.state_path }}",
        )
        self.assertIn("macos-signing-keychain.sh", cleanup["run"])
        self.assertIn(" cleanup", cleanup["run"])

    def test_reusable_browser_build_records_the_checked_out_source(self):
        workflow = self.load_workflow("build-browseros.yml")
        steps = workflow["jobs"]["build"]["steps"]
        source = next(
            step for step in steps if step.get("name") == "Record build source"
        )
        build = next(
            step for step in steps if step.get("name") == "Build ${{ inputs.product }}"
        )

        self.assertIn("git rev-parse HEAD", source["run"])
        self.assertEqual(
            build["env"]["BROWSEROS_BUILD_SOURCE_SHA"],
            "${{ steps.source.outputs.sha }}",
        )
        self.assertEqual(
            build["env"]["ONBOARDING_RESOURCE_VERSION"],
            "${{ inputs.onboarding-version }}",
        )
        self.assertNotIn("BROWSERCLAW_ONBOARD_RESOURCE_VERSION", build["env"])

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
            "Prepare chromium source and dependencies",
            "Clean cached product outputs and resources",
        ):
            with self.subTest(phase=phase):
                self.assertLess(bootstrap_index, indexes[phase])

    def test_source_ensure_explicitly_repairs_disposable_depot_tools_cache(self):
        steps = self.build_steps()

        for phase in (
            "Prepare chromium source and dependencies",
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

    def test_app_onboard_release_changes_trigger_build_system_tests(self):
        test_workflow = self.load_workflow("bos-build-tests.yml")
        triggers = test_workflow.get("on", test_workflow.get(True))
        pull_request_paths = triggers["pull_request"]["paths"]

        self.assertIn(
            ".github/workflows/release-app-onboard.yml",
            pull_request_paths,
        )

    def test_macos_signing_helper_changes_trigger_build_system_tests(self):
        test_workflow = self.load_workflow("bos-build-tests.yml")
        triggers = test_workflow.get("on", test_workflow.get(True))
        pull_request_paths = triggers["pull_request"]["paths"]

        self.assertIn(
            ".github/scripts/macos-signing-keychain.sh",
            pull_request_paths,
        )

    def test_macos_chromium_workspace_helper_changes_trigger_build_system_tests(self):
        test_workflow = self.load_workflow("bos-build-tests.yml")
        triggers = test_workflow.get("on", test_workflow.get(True))
        pull_request_paths = triggers["pull_request"]["paths"]

        self.assertIn(
            ".github/scripts/macos-chromium-workspace.sh",
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
            "server_workflow": "release-server.yml",
            "extension": "agent",
            "onboarding_manifest": "apps/app-onboard/package.json",
        },
        "release-browserclaw.yml": {
            "product": "browserclaw",
            "server_workflow": "release-claw-server.yml",
            "extension": "browserclaw",
            "onboarding_manifest": "apps/claw-onboard/package.json",
        },
    }

    # Neo's narrower dispatch surface; each default reproduces the full
    # release, and x64-only macOS is absent because release-macos.yml cannot
    # build it.
    NEO_DISPATCH_INPUTS = {
        "platforms": ("choice", ["all", "macos", "linux", "windows"], "all"),
        "macos_arch": ("choice", ["universal", "arm64"], "universal"),
        "linux_arch": ("choice", ["all", "x64", "arm64"], "all"),
        "publication": ("choice", ["alpha", "draft-only"], "alpha"),
    }
    NEO_ALPHA = "${{ needs.preflight.outputs.publication == 'alpha' }}"
    NEO_DRAFT_ONLY = "${{ needs.preflight.outputs.publication == 'draft-only' }}"

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

    def test_full_release_dispatch_surfaces_default_to_the_full_release(self):
        for workflow_name in self.RELEASES:
            with self.subTest(workflow=workflow_name):
                workflow = self.load_workflow(workflow_name)
                triggers = workflow.get("on", workflow.get(True))
                if workflow_name == "release-browseros.yml":
                    self.assertIsNone(triggers["workflow_dispatch"])
                else:
                    inputs = triggers["workflow_dispatch"]["inputs"]
                    self.assertEqual(
                        {
                            name: (spec["type"], spec["options"], spec["default"])
                            for name, spec in inputs.items()
                        },
                        self.NEO_DISPATCH_INPUTS,
                    )
                self.assertFalse(workflow["concurrency"]["cancel-in-progress"])
                self.assertEqual(workflow["concurrency"]["queue"], "max")
                self.assertEqual(workflow["permissions"], {})

    def test_preflight_freezes_the_default_branch_dispatch_sha(self):
        for workflow_name, config in self.RELEASES.items():
            with self.subTest(workflow=workflow_name):
                workflow = self.load_workflow(workflow_name)
                preflight = workflow["jobs"]["preflight"]
                checkout = next(
                    step
                    for step in preflight["steps"]
                    if str(step.get("uses", "")).startswith("actions/checkout@")
                )
                resolve = self.named_step(
                    workflow, "preflight", "Resolve release source and version"
                )
                self.assertEqual(
                    preflight["permissions"],
                    {"contents": "read"},
                )
                self.assertEqual(checkout["with"]["ref"], "${{ github.sha }}")
                self.assertEqual(checkout["with"]["fetch-depth"], 0)
                for token in (
                    'expected_ref="refs/heads/$DEFAULT_BRANCH"',
                    'source_sha="$(git rev-parse HEAD)"',
                    'test "$source_sha" = "$GITHUB_SHA"',
                    "bump_version.py --mode none",
                    config["onboarding_manifest"],
                ):
                    self.assertIn(token, resolve["run"])

    def test_components_publish_alpha_in_strict_order(self):
        for workflow_name, config in self.RELEASES.items():
            with self.subTest(workflow=workflow_name):
                workflow = self.load_workflow(workflow_name)
                jobs = workflow["jobs"]
                server = jobs["server"]
                extension = jobs["extension"]

                self.assertEqual(server["needs"], "preflight")
                self.assertEqual(
                    server["uses"],
                    f"./.github/workflows/{config['server_workflow']}",
                )
                self.assertIn(
                    server["with"]["ref"],
                    "${{ needs.preflight.outputs.source_sha }}",
                )
                if workflow_name == "release-browseros.yml":
                    self.assertIs(server["with"]["publish_ota"], True)
                    self.assertNotIn("defer_finalize", server["with"])
                else:
                    # draft-only defers finalize, which is what suppresses the
                    # tag, latest alias, public release, OTA and reflection.
                    self.assertEqual(server["with"]["publish_ota"], self.NEO_ALPHA)
                    self.assertEqual(
                        server["with"]["defer_finalize"], self.NEO_DRAFT_ONLY
                    )
                for pinned in ("version", "mode", "state_owner"):
                    self.assertNotIn(pinned, server["with"])
                self.assertEqual(server["secrets"], "inherit")

                self.assertEqual(set(extension["needs"]), {"preflight", "server"})
                self.assertEqual(
                    extension["uses"],
                    "./.github/workflows/release-extensions.yml",
                )
                self.assertEqual(extension["with"]["extension"], config["extension"])
                self.assertEqual(
                    extension["with"]["branch"],
                    "${{ needs.preflight.outputs.source_sha }}",
                )
                if workflow_name == "release-browseros.yml":
                    self.assertIs(extension["with"]["publish_alpha_feed"], True)
                    self.assertNotIn("defer_finalize", extension["with"])
                else:
                    self.assertEqual(
                        extension["with"]["publish_alpha_feed"], self.NEO_ALPHA
                    )
                    self.assertEqual(
                        extension["with"]["defer_finalize"], self.NEO_DRAFT_ONLY
                    )
                for pinned in ("version", "mode", "state_owner"):
                    self.assertNotIn(pinned, extension["with"])
                self.assertEqual(extension["secrets"], "inherit")

    def test_native_lanes_are_selected_and_use_published_resources(self):
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
                    if workflow_name == "release-browseros.yml":
                        self.assertNotIn("if", job)
                    else:
                        # No status function: the implicit success() still
                        # requires preflight and both exact components.
                        self.assertEqual(
                            job["if"],
                            f"needs.preflight.outputs.build_{job_name} == 'true'",
                        )
                    self.assertEqual(
                        set(job["needs"]),
                        {"preflight", "server", "extension", "components"},
                    )
                    self.assertEqual(
                        job["uses"], f"./.github/workflows/{called_workflow}"
                    )
                    self.assertEqual(job["with"]["products"], config["product"])
                    self.assertEqual(job["with"]["resource_mode"], "published")
                    self.assertIs(job["with"]["upload_to_r2"], True)
                    self.assertEqual(
                        job["with"]["ref"],
                        "${{ needs.preflight.outputs.source_sha }}",
                    )
                    self.assertEqual(
                        job["with"]["server_version"],
                        "${{ needs.server.outputs.version }}",
                    )
                    self.assertEqual(
                        job["with"]["extension_version"],
                        "${{ needs.extension.outputs.version }}",
                    )
                    self.assertEqual(
                        job["with"]["onboarding_version"],
                        "${{ needs.preflight.outputs.onboarding_version }}",
                    )
                    self.assertNotIn("candidate_sha", job["with"])
                    self.assertNotIn("prepared_resources_artifact", job["with"])
                    if job_name == "windows":
                        self.assertIs(job["with"]["sign"], True)
                    if arch is not None and workflow_name == "release-browseros.yml":
                        self.assertEqual(job["with"]["arch"], arch)
                        self.assertNotIn("notify", job["with"])
                    elif arch is not None:
                        self.assertEqual(
                            job["with"]["arch"],
                            "${{ needs.preflight.outputs.macos_arch }}",
                        )
                        self.assertEqual(job["with"]["notify"], self.NEO_ALPHA)
                    self.assertEqual(job["secrets"], "inherit")

    def test_browser_draft_waits_for_every_publication_and_native_lane(self):
        for workflow_name, config in self.RELEASES.items():
            with self.subTest(workflow=workflow_name):
                workflow = self.load_workflow(workflow_name)
                jobs = workflow["jobs"]
                self.assertEqual(
                    set(jobs["finalize"]["needs"]),
                    {
                        "preflight",
                        "server",
                        "extension",
                        "components",
                        "linux",
                        "windows",
                        "macos",
                    },
                )
                self.assertEqual(jobs["finalize"]["permissions"], {"contents": "write"})
                finalize = self.named_step(
                    workflow, "finalize", "Create or refresh browser draft"
                )
                if workflow_name == "release-browseros.yml":
                    self.assertNotIn("if", jobs["finalize"])
                    selection = ("--platforms all", "--macos-arch universal")
                else:
                    self.assertEqual(
                        " ".join(jobs["finalize"]["if"].split()),
                        NEO_FINALIZE_CONDITION,
                    )
                    selection = (
                        '--platforms "$PLATFORMS"',
                        '--macos-arch "$MACOS_ARCH"',
                    )
                    self.assertEqual(
                        finalize["env"]["PLATFORMS"],
                        "${{ needs.preflight.outputs.platforms }}",
                    )
                    self.assertEqual(
                        finalize["env"]["MACOS_ARCH"],
                        "${{ needs.preflight.outputs.macos_arch }}",
                    )
                    self.assertNotIn("inputs.", finalize["run"])
                for token in (
                    "release github create",
                    f"--product {config['product']}",
                    *selection,
                    '--source-sha "$SOURCE_SHA"',
                    '--workflow-run-id "$GITHUB_RUN_ID"',
                    '--target "$SOURCE_SHA"',
                ):
                    self.assertIn(token, finalize["run"])
                self.assertNotIn("--workflow-run-attempt", finalize["run"])

    def test_standalone_component_allocators_share_one_preparation_lock(self):
        workflows = (
            "release-server.yml",
            "release-claw-server.yml",
            "release-claw-onboard.yml",
            "release-app-onboard.yml",
            "release-extensions.yml",
        )
        for workflow_name in workflows:
            workflow = self.load_workflow(workflow_name)
            with self.subTest(workflow=workflow_name):
                self.assertEqual(
                    workflow["jobs"]["prepare"]["concurrency"]["group"],
                    "release-component-allocation",
                )
                self.assertEqual(
                    workflow["jobs"]["prepare"]["concurrency"]["queue"],
                    "max",
                )

    def test_component_workflows_inspect_private_drafts_with_release_cli(self):
        for workflow_name in (
            "release-server.yml",
            "release-claw-server.yml",
            "release-claw-onboard.yml",
            "release-app-onboard.yml",
            "release-extensions.yml",
        ):
            text = (WORKFLOW_DIR / workflow_name).read_text(encoding="utf-8")
            with self.subTest(workflow=workflow_name):
                self.assertNotIn("releases/tags/", text)
                self.assertGreaterEqual(
                    text.count("--json isDraft,targetCommitish,assets"),
                    2,
                )

    def test_release_critical_concurrency_groups_retain_pending_runs(self):
        for workflow_name in (
            "release-browseros.yml",
            "release-browserclaw.yml",
            "release-server.yml",
            "release-claw-server.yml",
            "release-claw-onboard.yml",
            "release-app-onboard.yml",
            "release-extensions.yml",
            "release-extension-feeds.yml",
            "release-linux.yml",
            "release-windows.yml",
        ):
            workflow = self.load_workflow(workflow_name)
            concurrency = workflow["concurrency"]
            with self.subTest(workflow=workflow_name):
                self.assertFalse(concurrency["cancel-in-progress"])
                self.assertEqual(concurrency["queue"], "max")

    def test_component_reflection_waits_for_live_alpha_publication(self):
        for workflow_name in ("release-server.yml", "release-claw-server.yml"):
            workflow = self.load_workflow(workflow_name)
            reflect = workflow["jobs"]["reflect-version"]
            with self.subTest(workflow=workflow_name):
                self.assertEqual(
                    set(reflect["needs"]),
                    {"prepare", "finalize", "publish-ota"},
                )
                self.assertIn("needs.publish-ota.result == 'success'", reflect["if"])

        extension = self.load_workflow("release-extensions.yml")
        reflect = extension["jobs"]["reflect-version"]
        self.assertEqual(
            set(reflect["needs"]),
            {"prepare", "finalize", "publish_alpha"},
        )
        self.assertIn("needs.publish_alpha.result == 'success'", reflect["if"])

    def test_version_reflection_targets_the_configured_source_repository(self):
        # BrowserOS-internal mirrors this repository one way, so its releases
        # must reflect versions into the repository it mirrors, not itself.
        token = "${{ secrets.RELEASE_REFLECT_TOKEN || github.token }}"
        default_branch = "${{ steps.source.outputs.default_branch }}"
        for workflow_name in (
            "release-server.yml",
            "release-claw-server.yml",
            "release-extensions.yml",
            "release-app-onboard.yml",
            "release-claw-onboard.yml",
        ):
            workflow = self.load_workflow(workflow_name)
            reflect = workflow["jobs"]["reflect-version"]
            source, checkout = reflect["steps"][:2]
            publish = next(
                step
                for step in reflect["steps"]
                if "merge-release-pr.sh" in step.get("run", "")
            )
            with self.subTest(workflow=workflow_name):
                self.assertEqual(
                    reflect["env"]["GH_REPO"],
                    "${{ vars.RELEASE_REFLECT_REPO || github.repository }}",
                )
                self.assertEqual(source["id"], "source")
                self.assertEqual(source["env"]["GH_TOKEN"], token)
                self.assertIn('gh api "repos/$GH_REPO"', source["run"])
                self.assertTrue(checkout["uses"].startswith("actions/checkout@"))
                self.assertEqual(
                    checkout["with"],
                    {
                        "fetch-depth": 0,
                        "repository": "${{ env.GH_REPO }}",
                        "ref": default_branch,
                        "token": token,
                    },
                )
                self.assertEqual(publish["env"]["GH_TOKEN"], token)
                self.assertEqual(publish["env"]["DEFAULT_BRANCH"], default_branch)
                self.assertNotIn("--repo", publish["run"])

    def test_reusable_components_have_one_validated_state_owner(self):
        for workflow_name in (
            "release-server.yml",
            "release-claw-server.yml",
            "release-extensions.yml",
        ):
            workflow = self.load_workflow(workflow_name)
            triggers = workflow.get("on", workflow.get(True))
            call_inputs = triggers["workflow_call"]["inputs"]
            dispatch_inputs = triggers["workflow_dispatch"]["inputs"]
            validate = self.named_step(workflow, "prepare", "Validate lifecycle inputs")
            reflect = workflow["jobs"]["reflect-version"]
            with self.subTest(workflow=workflow_name):
                self.assertEqual(call_inputs["state_owner"]["default"], "component")
                self.assertNotIn("state_owner", dispatch_inputs)
                self.assertEqual(
                    validate["env"]["STATE_OWNER"],
                    "${{ inputs.state_owner || 'component' }}",
                )
                self.assertIn("component|suite", validate["run"])
                self.assertIn("inputs.state_owner != 'suite'", reflect["if"])

    def test_server_latest_aliases_reconcile_monotonically_inside_retained_lock(self):
        expected_groups = {
            "release-server.yml": "release-server",
            "release-claw-server.yml": "release-claw-server-rust",
        }
        for workflow_name, group in expected_groups.items():
            workflow = self.load_workflow(workflow_name)
            latest = self.named_step(
                workflow, "finalize", "Copy versioned objects to latest"
            )
            script = latest["run"]
            with self.subTest(workflow=workflow_name):
                self.assertEqual(workflow["concurrency"]["group"], group)
                self.assertFalse(workflow["concurrency"]["cancel-in-progress"])
                self.assertNotIn("if", latest)
                self.assertIn("current_version", script)
                self.assertIn("sort -V", script)
                self.assertIn("already newer", script)
                self.assertIn("already matches version, source, and checksum", script)
                self.assertIn("conflicts at version", script)
                self.assertIn(
                    'test("^(0|[1-9][0-9]*)[.](0|[1-9][0-9]*)[.](0|[1-9][0-9]*)$"))',
                    script,
                )
                self.assertIn("copy-object", script)
                self.assertLess(
                    script.index("already newer"), script.index("copy-object")
                )

    def test_server_latest_alias_behavior_covers_missing_older_same_and_newer(self):
        fake_aws = r"""#!/usr/bin/env python3
import hashlib
import json
import os
import sys
from pathlib import Path


def option(name):
    return sys.argv[sys.argv.index(name) + 1]


operation = sys.argv[2]
key = option("--key")
state = Path(os.environ["FAKE_AWS_STATE"])
targets = ("darwin-arm64", "darwin-x64", "linux-arm64", "linux-x64", "windows-x64")
target = next(value for value in targets if key.endswith(f"-{value}.zip"))
component = "artifacts/server" if key.startswith("artifacts/server/") else "claw-server-rust/prod-resources"
marker = state / target

if operation == "copy-object":
    marker.touch()
    with (state / "copies").open("a", encoding="utf-8") as stream:
        stream.write(key + "\n")
    print("{}")
    raise SystemExit(0)

if operation != "head-object":
    raise SystemExit(f"unexpected fake aws operation: {operation}")

is_latest = "/latest/" in key
if is_latest and not marker.exists() and os.environ["FAKE_DEST_VERSION"] == "missing":
    print("An error occurred (404) when calling HeadObject: Not Found", file=sys.stderr)
    raise SystemExit(44)

if is_latest and not marker.exists():
    version = os.environ["FAKE_DEST_VERSION"]
    release_sha = os.environ["FAKE_DEST_SOURCE"]
    checksum = os.environ["FAKE_DEST_CHECKSUM"]
    if checksum == "exact":
        checksum = hashlib.sha256(target.encode()).hexdigest()
else:
    version = os.environ["VERSION"]
    release_sha = os.environ["RELEASE_SHA"]
    checksum = hashlib.sha256(target.encode()).hexdigest()
print(json.dumps({
    "ContentLength": 100,
    "Metadata": {
        "component": component,
        "target": target,
        "version": version,
        "release-sha": release_sha,
        "sha256": checksum,
    },
}))
"""
        cases = (
            ("missing", "9" * 40, "f" * 64, 5, True),
            ("0.0.1", "9" * 40, "f" * 64, 5, True),
            ("1.2.3", "1" * 40, "exact", 0, True),
            ("9.0.0", "9" * 40, "f" * 64, 0, True),
            ("1.2.3", "9" * 40, "exact", 0, False),
            ("1.2.3", "1" * 40, "f" * 64, 0, False),
        )
        for workflow_name in ("release-server.yml", "release-claw-server.yml"):
            script = self.named_step(
                self.load_workflow(workflow_name),
                "finalize",
                "Copy versioned objects to latest",
            )["run"]
            for (
                destination_version,
                destination_source,
                destination_checksum,
                copy_count,
                succeeds,
            ) in cases:
                with (
                    self.subTest(
                        workflow=workflow_name,
                        destination_version=destination_version,
                        destination_source=destination_source,
                    ),
                    tempfile.TemporaryDirectory() as temp_dir,
                ):
                    temp_root = Path(temp_dir)
                    fake_bin = temp_root / "bin"
                    fake_bin.mkdir()
                    aws = fake_bin / "aws"
                    aws.write_text(fake_aws, encoding="utf-8")
                    aws.chmod(0o755)
                    state = temp_root / "state"
                    state.mkdir()
                    environment = {
                        **os.environ,
                        "PATH": f"{fake_bin}{os.pathsep}{os.environ['PATH']}",
                        "FAKE_AWS_STATE": str(state),
                        "FAKE_DEST_VERSION": destination_version,
                        "FAKE_DEST_SOURCE": destination_source,
                        "FAKE_DEST_CHECKSUM": destination_checksum,
                        "R2_ENDPOINT": "https://r2.invalid",
                        "R2_BUCKET": "bucket",
                        "RELEASE_SHA": "1" * 40,
                        "VERSION": "1.2.3",
                    }
                    result = subprocess.run(
                        [git_bash_path(), "-c", script],
                        cwd=REPO_ROOT,
                        env=environment,
                        capture_output=True,
                        text=True,
                    )
                    self.assertEqual(
                        result.returncode == 0,
                        succeeds,
                        msg=result.stdout + result.stderr,
                    )
                    copies = state / "copies"
                    copied = (
                        copies.read_text(encoding="utf-8").splitlines()
                        if copies.exists()
                        else []
                    )
                    self.assertEqual(len(copied), copy_count)

    def test_server_ota_hands_assembled_snapshot_to_suite_owner(self):
        workflow = self.load_workflow("publish-server-ota.yml")
        triggers = workflow.get("on", workflow.get(True))
        inputs = triggers["workflow_call"]["inputs"]
        validate = self.named_step(
            workflow, "validate", "Validate product snapshot ownership"
        )
        persist = self.named_step(workflow, "publish", "Persist alpha appcast snapshot")
        publish = self.named_step(
            workflow, "publish", "Publish committed alpha appcast"
        )
        upload = self.named_step(
            workflow, "publish", "Hand alpha appcast to release suite"
        )

        self.assertEqual(inputs["state_owner"]["default"], "component")
        self.assertEqual(
            validate["env"]["STATE_OWNER"],
            "${{ inputs.state_owner || 'component' }}",
        )
        self.assertIn("component|suite", validate["run"])
        self.assertEqual(persist["if"], "inputs.state_owner != 'suite'")
        self.assertEqual(publish["if"], "inputs.state_owner != 'suite'")
        self.assertEqual(upload["if"], "inputs.state_owner == 'suite'")
        self.assertTrue(str(upload["uses"]).startswith("actions/upload-artifact@"))
        self.assertEqual(
            upload["with"]["name"],
            "server-ota-snapshot-${{ inputs.product }}",
        )
        self.assertEqual(upload["with"]["path"], "${{ inputs.snapshot_path }}")

    def test_extension_version_is_resolved_once(self):
        workflow = self.load_workflow("release-extensions.yml")
        resolve = self.named_step(workflow, "prepare", "Resolve extension release")[
            "run"
        ]
        self.assertEqual(
            resolve.count('uv run browseros release component resolve "${args[@]}"'),
            1,
        )

    def test_component_resolvers_include_immutable_r2_allocations(self):
        jobs = (
            ("release-server.yml", "prepare", "Resolve release"),
            ("release-claw-server.yml", "prepare", "Resolve release"),
            ("release-claw-onboard.yml", "prepare", "Resolve release"),
            ("release-app-onboard.yml", "prepare", "Resolve release"),
            ("release-extensions.yml", "prepare", "Resolve extension release"),
        )
        for workflow_name, job_name, step_name in jobs:
            workflow = self.load_workflow(workflow_name)
            step = self.named_step(workflow, job_name, step_name)
            with self.subTest(workflow=workflow_name):
                self.assertIn("--r2-allocations", step["run"])
                for name in (
                    "R2_ACCOUNT_ID",
                    "R2_ACCESS_KEY_ID",
                    "R2_SECRET_ACCESS_KEY",
                    "R2_BUCKET",
                ):
                    self.assertEqual(step["env"][name], f"${{{{ secrets.{name} }}}}")

    def test_feed_snapshot_writers_share_a_retained_queue(self):
        jobs = (
            ("publish-server-ota.yml", "publish"),
            ("release-extensions.yml", "publish_alpha"),
        )
        for workflow_name, job_name in jobs:
            workflow = self.load_workflow(workflow_name)
            concurrency = workflow["jobs"][job_name]["concurrency"]
            with self.subTest(workflow=workflow_name):
                self.assertEqual(concurrency["group"], "release-feed-snapshots")
                self.assertFalse(concurrency["cancel-in-progress"])
                self.assertEqual(concurrency["queue"], "max")

    def test_full_workflows_have_no_retired_source_candidate_lifecycle(self):
        for workflow_name in self.RELEASES:
            text = (WORKFLOW_DIR / workflow_name).read_text(encoding="utf-8")
            with self.subTest(workflow=workflow_name):
                for token in (
                    "release candidate",
                    "prepared_resources_artifact",
                    "candidate_sha",
                    "resource_mode: source",
                ):
                    self.assertNotIn(token, text)
                deferrals = text.count("defer_finalize")
                if workflow_name == "release-browseros.yml":
                    self.assertEqual(deferrals, 0)
                else:
                    self.assertEqual(
                        text.count(f"defer_finalize: {self.NEO_DRAFT_ONLY}"),
                        deferrals,
                    )
                    self.assertEqual(deferrals, 2)

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
        for workflow_name in (
            "release-browseros.yml",
            "release-browserclaw.yml",
            "release-server.yml",
            "release-claw-server.yml",
            "release-docker.yml",
            "release-extensions.yml",
            "release-macos.yml",
        ):
            self.assertIn(f".github/workflows/{workflow_name}", paths)

    def test_macos_runner_queue_retains_pending_builds(self):
        workflow = self.load_workflow("release-macos.yml")
        self.assertNotIn("concurrency", workflow)
        self.assertEqual(
            workflow["jobs"]["build"]["concurrency"],
            {
                "group": "macos-build",
                "cancel-in-progress": False,
                "queue": "max",
            },
        )

    def test_macos_release_builds_inside_disposable_chromium_workspace(self):
        workflow = self.load_workflow("release-macos.yml")
        steps = workflow["jobs"]["build"]["steps"]
        indexes = {
            step.get("name"): index
            for index, step in enumerate(steps)
            if "name" in step
        }
        setup = steps[indexes["Setup disposable Chromium workspace"]]
        build = steps[indexes["Build selected products"]]
        workspace_cleanup = steps[indexes["Clean up disposable Chromium workspace"]]
        keychain_cleanup = steps[indexes["Clean up macOS signing keychain"]]

        self.assertLess(
            indexes["Setup disposable Chromium workspace"],
            indexes["Build selected products"],
        )
        self.assertLess(
            indexes["Build selected products"],
            indexes["Clean up disposable Chromium workspace"],
        )
        self.assertLess(
            indexes["Clean up disposable Chromium workspace"],
            indexes["Clean up macOS signing keychain"],
        )
        self.assertEqual(setup["id"], "chromium_workspace")
        self.assertEqual(setup["shell"], "bash")
        self.assertEqual(
            setup["working-directory"],
            "${{ steps.inputs.outputs.browseros_repo }}",
        )
        self.assertIn("macos-chromium-workspace.sh setup", setup["run"])
        self.assertIn("${{ steps.inputs.outputs.chromium_src }}", setup["run"])
        self.assertIn("${{ steps.inputs.outputs.products }}", setup["run"])
        self.assertIn(
            '--chromium-src "$chromium_src"',
            build["run"],
        )
        for product in ("browseros", "browserclaw"):
            self.assertEqual(
                build["env"][f"{product.upper()}_CHROMIUM_SRC"],
                "${{ steps.chromium_workspace.outputs." + product + "_chromium_src }}",
            )
        self.assertIn("--profile release-ci", build["run"])
        self.assertIn("browseros source clean-outputs", build["run"])
        self.assertNotIn("--modules clean", build["run"])
        self.assertNotIn(
            '--chromium-src "${{ steps.inputs.outputs.chromium_src }}"',
            build["run"],
        )
        self.assertEqual(workspace_cleanup["if"], "always()")
        self.assertNotIn("MACOS_CHROMIUM_WORKSPACE_STATE_PATH", workspace_cleanup.get("env", {}))
        self.assertIn("macos-chromium-workspace.sh", workspace_cleanup["run"])
        self.assertIn(" cleanup", workspace_cleanup["run"])
        self.assertEqual(keychain_cleanup["if"], "always()")


class LinuxReleaseSelectionTest(unittest.TestCase):
    """Execute the shipped planner before any expensive Chromium runner starts."""

    @classmethod
    def setUpClass(cls):
        cls.workflow = yaml.safe_load(
            (WORKFLOW_DIR / "release-linux.yml").read_text(encoding="utf-8")
        )

    def run_plan(
        self, products: str, arch: str, mode: str = "published", *, pinned: bool = False,
        resource_versions: dict[str, str] | None = None,
    ):
        step = self.workflow["jobs"]["plan"]["steps"][0]
        self.assertNotIn("${{", step["run"])
        with tempfile.TemporaryDirectory() as temp_dir:
            output = Path(temp_dir) / "output"
            output.touch()
            result = subprocess.run(
                [git_bash_path(), "-c", step["run"]],
                env={
                    **os.environ,
                    "GITHUB_OUTPUT": str(output),
                    "PRODUCTS": products,
                    "ARCH": arch,
                    "RESOURCE_MODE": mode,
                    "SERVER_VERSION": "0.0.68" if pinned else "",
                    "EXTENSION_VERSION": "0.2.30" if pinned else "",
                    "ONBOARDING_VERSION": "0.1.0" if pinned else "",
                    **(resource_versions or {}),
                },
                capture_output=True,
                text=True,
            )
            outputs = dict(
                line.split("=", 1)
                for line in output.read_text(encoding="utf-8").splitlines()
            )
        return result, outputs

    def test_planner_expands_only_selected_products_and_architectures(self):
        cases = (
            ("browserclaw", "x64", [{"product": "browserclaw", "arch": "x64"}]),
            ("browserclaw", "arm64", [{"product": "browserclaw", "arch": "arm64"}]),
            ("browserclaw", "all", [
                {"product": "browserclaw", "arch": "x64"},
                {"product": "browserclaw", "arch": "arm64"},
            ]),
            ("all", "arm64", [
                {"product": "browseros", "arch": "arm64"},
                {"product": "browserclaw", "arch": "arm64"},
            ]),
        )
        for products, arch, expected in cases:
            with self.subTest(products=products, arch=arch):
                result, outputs = self.run_plan(products, arch, pinned=arch == "all")
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(json.loads(outputs["matrix"]), expected)

    def test_unsupported_selections_fail_before_emitting_a_matrix(self):
        for products, arch, mode in (
            ("browserclaw", "arm64;true", "published"),
            ("browserclaw", "", "published"),
            ("browserclaw;true", "arm64", "published"),
            ("browserclaw", "arm64", "source"),
            ("browserclaw", "all", "source"),
            ("all", "x64", "source"),
            ("browserclaw", "x64", "typo"),
            ("browserclaw", "all", "published"),
        ):
            with self.subTest(products=products, arch=arch, mode=mode):
                result, outputs = self.run_plan(products, arch, mode)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("::error::", result.stdout)
                self.assertEqual(outputs, {})

        result, outputs = self.run_plan("browserclaw", "x64", "source")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(outputs["matrix"]), [
            {"product": "browserclaw", "arch": "x64"},
        ])

        result, outputs = self.run_plan("all", "all", pinned=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(outputs, {})

    def test_dual_architectures_reject_mutable_resource_selectors(self):
        for name in ("SERVER_VERSION", "EXTENSION_VERSION", "ONBOARDING_VERSION"):
            for alias in ("latest", "alpha", "../latest"):
                with self.subTest(name=name, alias=alias):
                    result, outputs = self.run_plan(
                        "browserclaw", "all", pinned=True,
                        resource_versions={name: alias},
                    )
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn("::error::", result.stdout)
                    self.assertEqual(outputs, {})

    def test_architecture_selection_and_metadata_ownership_are_explicit(self):
        triggers = self.workflow.get("on", self.workflow.get(True))
        dispatch = triggers["workflow_dispatch"]["inputs"]["arch"]
        self.assertEqual(dispatch["options"], ["x64", "arm64", "all"])
        self.assertEqual(dispatch["default"], "x64")
        self.assertEqual(triggers["workflow_call"]["inputs"]["arch"]["default"], "x64")
        build = self.workflow["jobs"]["build"]
        # Architectures build on separate VMs; release.json uses an R2 CAS.
        self.assertNotIn("max-parallel", build["strategy"])
        self.assertEqual(build["with"]["arch"], "${{ matrix.arch }}")


class NeoReleaseSelectionTest(unittest.TestCase):
    """Behavior of Neo's lane selection, draft gate and quiet test builds."""

    @classmethod
    def setUpClass(cls):
        cls.neo = yaml.safe_load(
            (WORKFLOW_DIR / "release-browserclaw.yml").read_text(encoding="utf-8")
        )
        cls.macos = yaml.safe_load(
            (WORKFLOW_DIR / "release-macos.yml").read_text(encoding="utf-8")
        )

    def step(self, workflow, job_name: str, step_name: str) -> dict[str, object]:
        return next(
            step
            for step in workflow["jobs"][job_name]["steps"]
            if step.get("name") == step_name
        )

    def run_selection(
        self,
        platforms: str,
        macos_arch: str,
        publication: str,
        linux_arch: str | None = None,
    ):
        if linux_arch is None:
            triggers = self.neo.get("on", self.neo.get(True))
            linux_arch = str(
                triggers["workflow_dispatch"]["inputs"]["linux_arch"]["default"]
            )
        step = self.step(self.neo, "preflight", "Validate release selection")
        self.assertEqual(
            step["env"],
            {
                "MACOS_ARCH": "${{ inputs.macos_arch }}",
                "LINUX_ARCH": "${{ inputs.linux_arch }}",
                "PLATFORMS": "${{ inputs.platforms }}",
                "PUBLICATION": "${{ inputs.publication }}",
            },
        )
        self.assertNotIn("${{", step["run"])
        with tempfile.TemporaryDirectory() as temp_dir:
            output = Path(temp_dir) / "output"
            output.touch()
            result = subprocess.run(
                [git_bash_path(), "-c", step["run"]],
                env={
                    **os.environ,
                    "GITHUB_OUTPUT": str(output),
                    "MACOS_ARCH": macos_arch,
                    "LINUX_ARCH": linux_arch,
                    "PLATFORMS": platforms,
                    "PUBLICATION": publication,
                },
                capture_output=True,
                text=True,
            )
            outputs = dict(
                line.split("=", 1)
                for line in output.read_text(encoding="utf-8").splitlines()
            )
        return result, outputs

    def test_preflight_emits_full_release_selection_by_default(self):
        result, outputs = self.run_selection("all", "universal", "alpha")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            outputs,
            {
                "build_linux": "true",
                "build_macos": "true",
                "build_windows": "true",
                "macos_arch": "universal",
                "linux_arch": "all",
                "platforms": "all",
                "publication": "alpha",
            },
        )

    def test_preflight_selects_only_the_macos_arm64_lane(self):
        result, outputs = self.run_selection("macos", "arm64", "draft-only")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            outputs,
            {
                "build_linux": "false",
                "build_macos": "true",
                "build_windows": "false",
                "macos_arch": "arm64",
                "linux_arch": "all",
                "platforms": "macos",
                "publication": "draft-only",
            },
        )

    def test_preflight_selects_linux_arm64_and_forwards_the_same_contract(self):
        for arch in ("arm64", "all"):
            result, outputs = self.run_selection(
                "linux", "universal", "draft-only", arch
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(outputs["linux_arch"], arch)
            self.assertEqual(outputs["build_linux"], "true")
            self.assertEqual(outputs["build_macos"], "false")
            self.assertEqual(outputs["build_windows"], "false")

        linux = self.neo["jobs"]["linux"]
        self.assertEqual(
            linux["with"]["arch"], "${{ needs.preflight.outputs.linux_arch }}"
        )
        finalize = self.step(self.neo, "finalize", "Create or refresh browser draft")
        self.assertEqual(
            finalize["env"]["LINUX_ARCH"], "${{ needs.preflight.outputs.linux_arch }}"
        )
        self.assertIn('--linux-arch "$LINUX_ARCH"', finalize["run"])

    def test_preflight_rejects_unused_or_invalid_linux_architecture(self):
        for platforms, arch in (
            ("macos", "arm64"),
            ("windows", "arm64"),
            ("linux", "bad;true"),
        ):
            result, outputs = self.run_selection(platforms, "universal", "alpha", arch)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("::error::", result.stdout)
            self.assertEqual(outputs, {})

    def test_preflight_rejects_unsupported_or_meaningless_requests(self):
        cases = (
            ("macos", "x64", "alpha"),
            ("linux", "arm64", "alpha"),
            ("macos;true", "arm64", "alpha"),
            ("all", "universal", "prod"),
            ("", "universal", "alpha"),
        )
        for platforms, macos_arch, publication in cases:
            with self.subTest(platforms=platforms, macos_arch=macos_arch):
                result, outputs = self.run_selection(
                    platforms, macos_arch, publication
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("::error::", result.stdout)
                self.assertEqual(outputs, {})

    def finalize_runs(
        self,
        selected: set[str],
        lanes: dict[str, str],
        *,
        cancelled: bool = False,
        components: str = "success",
    ) -> bool:
        """Evaluate the shipped finalize `if` for one simulated run.

        The expression only uses !cancelled(), &&, ||, ==, string literals,
        parentheses and needs lookups, whose GitHub semantics match Python's
        not/and/or/==. The whitelist below fails if that grammar ever grows.
        """
        expression = " ".join(self.neo["jobs"]["finalize"]["if"].split())
        self.assertEqual(expression, NEO_FINALIZE_CONDITION)
        self.assertTrue(expression.startswith("${{ ") and expression.endswith(" }}"))
        body = expression[4:-3]
        body = body.replace("!cancelled()", "(not cancelled)")
        body = body.replace("&&", " and ").replace("||", " or ")
        body = re.sub(
            r"needs\.([a-z_]+)\.outputs\.([a-z_]+)",
            r"needs['\1']['outputs']['\2']",
            body,
        )
        body = re.sub(r"needs\.([a-z_]+)\.result", r"needs['\1']['result']", body)
        leftover = re.sub(r"'[^']*'|\b(?:not|and|or|cancelled|needs)\b", "", body)
        self.assertRegex(leftover, r"^[\s()\[\]=]*$")

        needs = {
            "preflight": {
                "result": "success",
                "outputs": {
                    f"build_{lane}": "true" if lane in selected else "false"
                    for lane in ("linux", "windows", "macos")
                },
            },
            "server": {"result": "success"},
            "extension": {"result": "success"},
            "components": {"result": components},
            **{lane: {"result": result} for lane, result in lanes.items()},
        }
        return eval(  # noqa: S307 - repository-owned expression, whitelisted above
            body, {"__builtins__": {}}, {"cancelled": cancelled, "needs": needs}
        )

    def test_finalize_requires_every_lane_of_the_default_full_release(self):
        full = {"linux", "windows", "macos"}
        succeeded = {"linux": "success", "windows": "success", "macos": "success"}
        self.assertTrue(self.finalize_runs(full, succeeded))
        for lane, result in (
            ("windows", "failure"),
            ("linux", "cancelled"),
            ("macos", "skipped"),
        ):
            with self.subTest(lane=lane, result=result):
                self.assertFalse(
                    self.finalize_runs(full, {**succeeded, lane: result})
                )

    def test_finalize_accepts_skipped_unselected_lanes_for_macos_arm64(self):
        lanes = {"linux": "skipped", "windows": "skipped", "macos": "success"}
        self.assertTrue(self.finalize_runs({"macos"}, lanes))

    def test_finalize_rejects_failed_selected_or_unexpected_lanes(self):
        base = {"linux": "skipped", "windows": "skipped", "macos": "success"}
        cases = (
            ({"macos": "failure"}, {}),
            ({"macos": "cancelled"}, {}),
            ({"macos": "skipped"}, {}),
            ({"linux": "success"}, {}),
            ({"windows": "failure"}, {}),
            ({}, {"components": "failure"}),
            ({}, {"cancelled": True}),
        )
        for lane_overrides, run_overrides in cases:
            with self.subTest(lanes=lane_overrides, run=run_overrides):
                self.assertFalse(
                    self.finalize_runs(
                        {"macos"}, {**base, **lane_overrides}, **run_overrides
                    )
                )

    def test_macos_notify_defaults_on_for_dispatch_and_call(self):
        triggers = self.macos.get("on", self.macos.get(True))
        for trigger in ("workflow_dispatch", "workflow_call"):
            with self.subTest(trigger=trigger):
                notify = triggers[trigger]["inputs"]["notify"]
                self.assertEqual(notify["type"], "boolean")
                self.assertIs(notify["default"], True)

        ping = self.step(self.macos, "build", "Slack failure ping")
        self.assertEqual(ping["if"], "failure() && inputs.notify")

    def test_quiet_macos_build_blanks_the_webhook_before_bos_build_runs(self):
        build = self.step(self.macos, "build", "Build selected products")
        script = build["run"]
        self.assertEqual(build["env"]["SLACK_NOTIFY"], "${{ inputs.notify }}")
        self.assertNotIn("SLACK_WEBHOOK_URL", build["env"])
        guard = 'if [ "$SLACK_NOTIFY" = "false" ]; then'
        self.assertIn(guard, script)
        self.assertIn('export SLACK_WEBHOOK_URL=""', script)
        self.assertLess(script.index(guard), script.index("uv run browseros"))

    def test_blank_webhook_wins_over_persistent_checkout_dotenv(self):
        from bos_build.lib import env as env_module
        from bos_build.lib.notify import Notifier

        with tempfile.TemporaryDirectory() as temp_dir:
            package_root = Path(temp_dir) / "packages" / "browseros"
            package_root.mkdir(parents=True)
            (package_root / ".env").write_text(
                "SLACK_WEBHOOK_URL=https://hooks.slack.invalid/T000\n",
                encoding="utf-8",
            )
            with mock.patch(
                "bos_build.lib.paths.get_package_root", return_value=package_root
            ):
                with mock.patch.dict(os.environ, {"SLACK_WEBHOOK_URL": ""}):
                    env_module._load_dotenv_file()
                    self.assertFalse(Notifier().enabled)
                # Control: without the blank override the dotenv webhook loads.
                with mock.patch.dict(os.environ, {}, clear=False):
                    os.environ.pop("SLACK_WEBHOOK_URL", None)
                    env_module._load_dotenv_file()
                    self.assertTrue(Notifier().enabled)


FAKE_GH = r"""#!/usr/bin/env python3
import json
import os
import subprocess
import sys
from pathlib import Path

state_path = Path(os.environ["FAKE_GH_STATE"])
state = json.loads(state_path.read_text())
args = sys.argv[1:]


def option(name, default=None):
    return args[args.index(name) + 1] if name in args else default


def record(entry):
    with open(os.environ["FAKE_GH_WRITES"], "a", encoding="utf-8") as stream:
        stream.write(json.dumps(entry) + "\n")


if args[:1] != ["release"]:
    raise SystemExit(f"unexpected fake gh call: {args}")
command, tag = args[1], args[2]
release = state["releases"].get(tag)

if command == "view":
    if release is None:
        print("release not found", file=sys.stderr)
        raise SystemExit(1)
    if "--json" not in args:
        print(f"title:\t{tag}")
        raise SystemExit(0)
    if release.get("jsonFails"):
        print("HTTP 502: transient metadata failure", file=sys.stderr)
        raise SystemExit(1)
    document = {
        "isDraft": release["isDraft"],
        "targetCommitish": release["targetCommitish"],
        "assets": [{"name": name} for name in sorted(release["assets"])],
    }
    query = option("--jq")
    if query is None:
        print(json.dumps(document))
    else:
        result = subprocess.run(
            ["jq", "-r", query], input=json.dumps(document),
            capture_output=True, text=True, check=True,
        )
        sys.stdout.write(result.stdout)
elif command == "download":
    name = option("--pattern")
    if release is None or name not in release["assets"]:
        raise SystemExit(1)
    target = Path(option("--dir"))
    target.mkdir(parents=True, exist_ok=True)
    (target / name).write_text(release["assets"][name], encoding="utf-8")
else:
    # Every other release subcommand mutates GitHub state.
    record({"command": command, "tag": tag, "args": args[3:],
            "draft": None if release is None else release["isDraft"]})
    if command == "create":
        state["releases"][tag] = {
            "isDraft": "--draft" in args,
            "targetCommitish": option("--target", ""),
            "assets": {},
        }
    elif command == "upload":
        if release is None:
            raise SystemExit(1)
        for path in (value for value in args[3:] if not value.startswith("--")):
            release["assets"][Path(path).name] = Path(path).read_text(encoding="utf-8")
    state_path.write_text(json.dumps(state))
"""


@unittest.skipIf(os.name == "nt", "component release scripts run on Linux runners")
class DeferredComponentAttachmentTest(unittest.TestCase):
    """Deferred component builds may write only to source-bound private drafts.

    These run the shipped attachment scripts against a fake `gh` that keeps
    release state and records every mutating call, so "zero public writes" is
    observed rather than inferred from the script text.
    """

    SOURCE = "1" * 40
    OTHER = "2" * 40
    SERVER_TAG = "claw-server/v1.2.3"
    SERVER_TARGETS = ("darwin-arm64", "darwin-x64", "linux-arm64", "linux-x64", "windows-x64")
    EXTENSION_TAG = "ext-browserclaw/v1.2.3.0"
    CRX = "browserclaw-1.2.3.0.crx"

    @classmethod
    def setUpClass(cls):
        probe = subprocess.run(
            [git_bash_path(), "-c", "mapfile -t probe < /dev/null"],
            capture_output=True,
        )
        if probe.returncode != 0:
            raise unittest.SkipTest("needs bash >= 4 (mapfile), as on Linux runners")

    def script(self, workflow_name: str, job: str, step_name: str) -> dict:
        workflow = yaml.safe_load((WORKFLOW_DIR / workflow_name).read_text(encoding="utf-8"))
        return next(
            step for step in workflow["jobs"][job]["steps"] if step.get("name") == step_name
        )

    def run_attachment(self, step, *, files, releases, env):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            fake_bin = root / "bin"
            fake_bin.mkdir()
            gh = fake_bin / "gh"
            gh.write_text(FAKE_GH, encoding="utf-8")
            gh.chmod(0o755)
            work = root / "work"
            for relative, content in files.items():
                path = work / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(content, encoding="utf-8")
            state = root / "state.json"
            state.write_text(json.dumps({"releases": releases}), encoding="utf-8")
            writes = root / "writes.jsonl"
            writes.touch()
            result = subprocess.run(
                [git_bash_path(), "-c", step["run"]],
                cwd=work,
                env={
                    **os.environ,
                    "PATH": f"{fake_bin}{os.pathsep}{os.environ['PATH']}",
                    "FAKE_GH_STATE": str(state),
                    "FAKE_GH_WRITES": str(writes),
                    "GH_TOKEN": "fake",
                    "RUNNER_TEMP": str(root / "runner-temp"),
                    "GIT_CEILING_DIRECTORIES": str(root),
                    **env,
                },
                capture_output=True,
                text=True,
            )
            calls = [
                json.loads(line)
                for line in writes.read_text(encoding="utf-8").splitlines()
            ]
        return result, calls

    def server_zip(self, target: str) -> str:
        return f"browseros-claw-server-rust-resources-{target}.zip"

    def run_server(
        self,
        release,
        *,
        defer="true",
        reservation="create",
        drop=None,
        alter=None,
        json_fails=False,
    ):
        step = self.script(
            "release-claw-server.yml", "publish-versioned", "Attach zips to private draft"
        )
        self.assertEqual(step["env"]["DEFER_FINALIZE"], "${{ inputs.defer_finalize }}")
        self.assertEqual(
            step["env"]["RESERVATION"], "${{ needs.prepare.outputs.reservation }}"
        )
        files = {
            f"dist/claw-server/{self.server_zip(target)}": f"canonical {target}"
            for target in self.SERVER_TARGETS
        }
        releases = {}
        if release is not None:
            draft, target = release
            assets = {Path(path).name: content for path, content in files.items()}
            if drop:
                del assets[self.server_zip(drop)]
            if alter:
                assets[self.server_zip(alter)] = "different bytes"
            releases[self.SERVER_TAG] = {
                "isDraft": draft,
                "targetCommitish": target,
                "assets": assets,
                "jsonFails": json_fails,
            }
        return self.run_attachment(
            step,
            files=files,
            releases=releases,
            env={
                "DEFER_FINALIZE": defer,
                "RELEASE_SHA": self.SOURCE,
                "RELEASE_TAG": self.SERVER_TAG,
                "RESERVATION": reservation,
                "VERSION": "1.2.3",
            },
        )

    def test_deferred_server_reuses_matching_public_assets_read_only(self):
        result, calls = self.run_server((False, self.SOURCE), reservation="tag")

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("read-only", result.stdout)
        self.assertEqual(calls, [])

    def test_deferred_server_rejects_missing_or_conflicting_public_assets(self):
        for change in ({"drop": "linux-x64"}, {"alter": "darwin-arm64"}):
            with self.subTest(**change):
                result, calls = self.run_server(
                    (False, self.SOURCE), reservation="tag", **change
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("::error::Published", result.stdout)
                self.assertEqual(calls, [])

    def test_deferred_server_rejects_a_foreign_draft(self):
        result, calls = self.run_server((True, self.OTHER), reservation="reuse")

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("is not a private draft", result.stdout)
        self.assertEqual(calls, [])

    def test_deferred_server_writes_only_to_its_private_draft(self):
        cases = (
            ("source-bound draft", (True, self.SOURCE), "reuse", ["upload"]),
            ("tag-verified draft", (True, self.OTHER), "tag", ["upload"]),
            ("new draft", None, "create", ["create", "upload"]),
        )
        for label, release, reservation, commands in cases:
            with self.subTest(label):
                result, calls = self.run_server(release, reservation=reservation)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual([call["command"] for call in calls], commands)
                upload = calls[-1]
                self.assertIn("--clobber", upload["args"])
                self.assertIn(upload["draft"], (True, None))
                if release is None:
                    self.assertIn("--draft", calls[0]["args"])

    def test_deferred_server_fails_closed_when_metadata_is_unreadable(self):
        # Plain existence succeeds in every case, so only the strict deferred
        # metadata read stands between these releases and `upload --clobber`.
        cases = (
            ("public, JSON read fails", (False, self.SOURCE), True),
            ("private draft, JSON read fails", (True, self.SOURCE), True),
            ("draft state missing", (None, self.SOURCE), False),
        )
        for label, release, json_fails in cases:
            with self.subTest(label):
                result, calls = self.run_server(
                    release, reservation="tag", json_fails=json_fails
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(calls, [])

    def test_alpha_server_attachment_is_unchanged(self):
        result, calls = self.run_server(
            (False, self.SOURCE), defer="", reservation="tag", alter="linux-x64"
        )

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual([call["command"] for call in calls], ["upload"])

    def run_extension(self, release, *, defer="true", asset=None, json_fails=False):
        step = self.script("release-extensions.yml", "build", "Attach CRX assets")
        self.assertEqual(step["env"]["DEFER_FINALIZE"], "${{ inputs.defer_finalize }}")
        built = f"packages/browseros/build/extensions/dist/{self.CRX}"
        releases = {}
        if release is not None:
            draft, target = release
            releases[self.EXTENSION_TAG] = {
                "isDraft": draft,
                "targetCommitish": target,
                "assets": {} if asset is None else {self.CRX: asset},
                "jsonFails": json_fails,
            }
        return self.run_attachment(
            step,
            files={built: "canonical crx"},
            releases=releases,
            env={
                "DEFER_FINALIZE": defer,
                "NAMES": "browserclaw",
                "RELEASE_SHA": self.SOURCE,
                "VERSION": "1.2.3.0",
            },
        )

    def test_deferred_extension_never_writes_to_a_public_release(self):
        cases = (
            ("matching", "canonical crx", 0),
            ("missing", None, 1),
            ("conflicting", "different bytes", 1),
        )
        for label, asset, failed in cases:
            with self.subTest(label):
                result, calls = self.run_extension((False, self.SOURCE), asset=asset)
                self.assertEqual(result.returncode != 0, bool(failed), result.stdout)
                self.assertEqual(calls, [])

    def test_deferred_extension_adds_assets_only_to_its_private_draft(self):
        result, calls = self.run_extension((True, self.OTHER))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("is not a private draft", result.stdout)
        self.assertEqual(calls, [])

        result, calls = self.run_extension((True, self.SOURCE))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(
            [(call["command"], call["draft"]) for call in calls], [("upload", True)]
        )

    def test_deferred_extension_fails_closed_when_metadata_is_unreadable(self):
        for draft in (False, True):
            with self.subTest(draft=draft):
                result, calls = self.run_extension(
                    (draft, self.SOURCE), json_fails=True
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(calls, [])

    def test_alpha_extension_attachment_is_unchanged(self):
        result, calls = self.run_extension((False, self.SOURCE), defer="")

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual([call["command"] for call in calls], ["upload"])


class ProductNightlyWorkflowTest(unittest.TestCase):
    PRODUCTS = {
        "browseros": (
            "BrowserOS",
            "server",
            "agent",
            "server_version",
            "agent_version",
            "app_onboarding_version",
            "appcast-server.alpha.xml",
        ),
        "browserclaw": (
            "BrowserOS neo",
            "claw-server",
            "browserclaw",
            "claw_server_version",
            "browserclaw_version",
            "onboarding_version",
            "appcast-claw-server.alpha.xml",
        ),
    }

    def load_workflow(self, workflow_name: str) -> dict[str, object]:
        path = WORKFLOW_DIR / workflow_name
        return yaml.safe_load(path.read_text(encoding="utf-8"))

    def named_step(
        self, workflow: dict[str, object], job_name: str, step_name: str
    ) -> dict[str, object]:
        return next(
            step
            for step in workflow["jobs"][job_name]["steps"]
            if step.get("name") == step_name
        )

    def test_manual_product_entries_replace_family_dispatch(self):
        self.assertFalse((WORKFLOW_DIR / "nightly.yml").exists())
        self.assertFalse((WORKFLOW_DIR / "nightly-macos-product.yml").exists())
        for product, (
            label,
            _,
            _,
            server,
            extension,
            onboard,
            _,
        ) in self.PRODUCTS.items():
            with self.subTest(product=product):
                workflow = self.load_workflow(f"nightly-macos-{product}.yml")
                self.assertEqual(workflow["name"], f"Nightly: {label} (macOS arm64)")
                self.assertEqual(
                    workflow.get("on", workflow.get(True)), {"workflow_dispatch": None}
                )
                self.assertEqual(workflow["permissions"], {})
                self.assertEqual(
                    workflow["concurrency"],
                    {
                        "group": "release-suite",
                        "cancel-in-progress": False,
                        "queue": "max",
                    },
                )
                transaction = workflow["jobs"]["transaction"]
                self.assertEqual(
                    transaction["concurrency"],
                    {
                        "group": "release-component-allocation",
                        "cancel-in-progress": False,
                        "queue": "max",
                    },
                )
                self.assertEqual(
                    transaction["steps"][0]["with"]["ref"], "${{ github.sha }}"
                )
                self.assertEqual(
                    set(transaction["outputs"]),
                    {
                        "state_ref",
                        "source_sha",
                        "reservation_sha",
                        "browser_version",
                        server,
                        extension,
                        onboard,
                    },
                )
                reconcile = self.named_step(
                    workflow, "transaction", "Reconcile product transaction"
                )["run"]
                for token in (
                    '"$DEFAULT_BRANCH" != "main"',
                    '"$SOURCE_REF" != "refs/heads/main"',
                    'test "$source_sha" = "$GITHUB_SHA"',
                    "release suite reconcile",
                    f"--product {product}",
                    "--mode nightly",
                    "Re-run failed jobs",
                    "Only an open transaction can start builds",
                ):
                    self.assertIn(token, reconcile)

    def test_each_graph_prepares_builds_and_finalizes_only_its_product(self):
        for product, (
            _,
            server,
            extension,
            server_pin,
            extension_pin,
            onboarding_pin,
            snapshot,
        ) in self.PRODUCTS.items():
            with self.subTest(product=product):
                workflow = self.load_workflow(f"nightly-macos-{product}.yml")
                jobs = workflow["jobs"]
                build_name = f"build-{product}"
                ota_name = "server-ota" if product == "browseros" else "claw-server-ota"
                self.assertEqual(
                    set(jobs),
                    {
                        "transaction",
                        f"prepare-{server}",
                        f"prepare-{extension}",
                        "verify-components",
                        build_name,
                        f"finalize-{server}",
                        f"finalize-{extension}",
                        ota_name,
                        "publish",
                        "notify-failure",
                    },
                )
                for component, workflow_name, pin in (
                    (server, f"release-{server}.yml", server_pin),
                    (extension, "release-extensions.yml", extension_pin),
                ):
                    for phase in ("prepare", "finalize"):
                        job = jobs[f"{phase}-{component}"]
                        self.assertEqual(
                            job["uses"], f"./.github/workflows/{workflow_name}"
                        )
                        self.assertEqual(job["with"]["state_owner"], "suite")
                        self.assertEqual(
                            job["with"]["version"],
                            f"${{{{ needs.transaction.outputs.{pin} }}}}",
                        )
                        source_input = "branch" if component == extension else "ref"
                        self.assertEqual(
                            job["with"][source_input],
                            "${{ needs.transaction.outputs.source_sha }}",
                        )
                        if phase == "prepare":
                            self.assertEqual(job["needs"], "transaction")
                            self.assertEqual(job["with"]["mode"], "build")
                            self.assertIs(job["with"]["defer_finalize"], True)
                        else:
                            self.assertEqual(
                                set(job["needs"]), {"transaction", build_name}
                            )
                            self.assertEqual(job["with"]["mode"], "finalize")
                        if component == extension:
                            self.assertEqual(job["with"]["extension"], extension)
                            self.assertIs(job["with"]["publish_alpha_feed"], False)
                        else:
                            self.assertIs(job["with"]["publish_ota"], False)
                self.assertEqual(
                    set(jobs["verify-components"]["needs"]),
                    {
                        "transaction",
                        f"prepare-{server}",
                        f"prepare-{extension}",
                    },
                )
                verify = self.named_step(
                    workflow,
                    "verify-components",
                    "Verify source and version identities",
                )["run"]
                self.assertIn('test "$SERVER_SHA" = "$SOURCE_SHA"', verify)
                self.assertIn('test "$EXTENSION_SHA" = "$SOURCE_SHA"', verify)
                self.assertIn(f"needs.transaction.outputs.{server_pin}", verify)
                self.assertIn(f"needs.transaction.outputs.{extension_pin}", verify)
                build = jobs[build_name]
                self.assertEqual(
                    set(build["needs"]), {"transaction", "verify-components"}
                )
                self.assertEqual(
                    build["uses"],
                    "./.github/workflows/reusable-build-macos-nightly.yml",
                )
                self.assertEqual(build["with"]["product"], product)
                for input_name, output_name in (
                    ("source_sha", "source_sha"),
                    ("reservation_sha", "reservation_sha"),
                    ("state_ref", "state_ref"),
                    ("browser_version", "browser_version"),
                    ("server_version", server_pin),
                    ("extension_version", extension_pin),
                    ("onboarding_version", onboarding_pin),
                ):
                    self.assertEqual(
                        build["with"][input_name],
                        f"${{{{ needs.transaction.outputs.{output_name} }}}}",
                    )
                self.assertNotIn("state_sha", build["with"])
                ota = jobs[ota_name]
                self.assertEqual(
                    ota["uses"], "./.github/workflows/publish-server-ota.yml"
                )
                self.assertEqual(
                    set(ota["needs"]), {"transaction", f"finalize-{server}"}
                )
                self.assertEqual(ota["with"]["product"], product)
                self.assertEqual(
                    ota["with"]["snapshot_path"], f"updates/server/{snapshot}"
                )
                self.assertEqual(ota["with"]["state_owner"], "suite")
                self.assertEqual(
                    ota["permissions"], {"contents": "write", "pull-requests": "write"}
                )
                self.assertEqual(
                    set(jobs["publish"]["needs"]),
                    {
                        "transaction",
                        build_name,
                        f"finalize-{server}",
                        f"finalize-{extension}",
                        ota_name,
                    },
                )
                self.assertEqual(
                    set(jobs["notify-failure"]["needs"]), set(jobs) - {"notify-failure"}
                )

    def test_one_locked_job_merges_then_publishes_scoped_artifacts_and_feeds(self):
        for product, (_, _, extension, _, _, _, snapshot) in self.PRODUCTS.items():
            with self.subTest(product=product):
                workflow = self.load_workflow(f"nightly-macos-{product}.yml")
                publish = workflow["jobs"]["publish"]
                self.assertEqual(
                    publish["concurrency"],
                    {
                        "group": "release-feed-snapshots",
                        "cancel-in-progress": False,
                        "queue": "max",
                    },
                )
                self.assertEqual(
                    publish["permissions"],
                    {"contents": "write", "pull-requests": "write"},
                )
                self.assertEqual(
                    publish["steps"][0]["with"]["ref"],
                    "${{ needs.transaction.outputs.source_sha }}",
                )
                names = (
                    "Recover product transaction",
                    "Compose snapshots from current main",
                    "Render shared extension feeds with the selected product pin",
                    "Reconcile and merge product state",
                    "Verify merged transaction and restore current committed feeds",
                    "Publish immutable signed browser artifact",
                    "Publish current committed product feeds",
                    "Reconcile product rolling nightly release",
                )
                steps = [self.named_step(workflow, "publish", name) for name in names]
                indexes = [publish["steps"].index(step) for step in steps]
                self.assertEqual(indexes, sorted(indexes))
                (
                    recover,
                    baseline,
                    render,
                    merge,
                    committed,
                    artifact,
                    feeds,
                    rolling,
                ) = steps
                for step in (recover, merge, committed):
                    self.assertIn(f"--product {product}", step["run"])
                    self.assertIn('--source-sha "$SOURCE_SHA"', step["run"])
                for step in (baseline, render, merge):
                    self.assertEqual(
                        step["if"], "steps.recovery.outputs.state == 'open'"
                    )
                self.assertIn("open|merged)", recover["run"])
                self.assertIn(".reservation_sha", recover["run"])
                self.assertIn(
                    'git restore --source="$BASE_SHA" --worktree', baseline["run"]
                )
                self.assertIn(
                    'git merge-base --is-ancestor "$SOURCE_SHA" "$BASE_SHA"',
                    baseline["run"],
                )
                self.assertNotIn("git checkout", baseline["run"])
                self.assertEqual(render["run"].count("--set"), 1)
                self.assertIn(f'--set "{extension}=$EXTENSION_VERSION"', render["run"])
                self.assertIn(
                    '--baseline-root "$GITHUB_WORKSPACE/updates"', render["run"]
                )
                self.assertIn('--state-base-sha "$BASE_SHA"', merge["run"])
                self.assertIn('--state-root "$GITHUB_WORKSPACE"', merge["run"])
                self.assertIn(f'products: ["{product}"]', merge["run"])
                self.assertIn("release suite merge", merge["run"])
                for token in (
                    'test "$(jq -r \'.state\' "$record")" = "merged"',
                    'test "$(jq -r \'.merge_sha\' "$record")" = "$merge_sha"',
                    'git merge-base --is-ancestor "$merge_sha" "$current_main"',
                    'git restore --source="$current_main" --worktree',
                ):
                    self.assertIn(token, committed["run"])
                self.assertNotIn('git restore --source="$merge_sha"', committed["run"])
                own_paths = {
                    "updates/extensions/bundled-manifest.xml",
                    "updates/extensions/extensions.alpha.json",
                    "updates/extensions/update-manifest.alpha.xml",
                    f"updates/server/{snapshot}",
                }
                for step in (baseline, committed):
                    for path in own_paths:
                        self.assertIn(path, step["run"])
                    sibling = (
                        "appcast-claw-server.alpha.xml"
                        if product == "browseros"
                        else "appcast-server.alpha.xml"
                    )
                    self.assertNotIn(sibling, step["run"])
                self.assertEqual(artifact["run"].count("publish-browser-artifact"), 1)
                self.assertIn(f"--product {product}", artifact["run"])
                self.assertIn(
                    f'--artifact-root "$RUNNER_TEMP/nightly-{product}"', artifact["run"]
                )
                for path in own_paths:
                    self.assertIn(path.removeprefix("updates/"), feeds["run"])
                self.assertNotIn("--allow-downgrade", render["run"] + feeds["run"])
                self.assertEqual(rolling["run"].count("reconcile-rolling-release"), 1)
                self.assertIn(f"--tag nightly-{product}", rolling["run"])
                self.assertIn('--source-sha "$SOURCE_SHA"', rolling["run"])
                self.assertIn('--browser-version "$VERSION"', rolling["run"])
                self.assertNotIn("gh release delete", rolling["run"])
                downloads = [
                    step
                    for step in publish["steps"]
                    if str(step.get("uses", "")).startswith(
                        "actions/download-artifact@"
                    )
                ]
                self.assertEqual(len(downloads), 2)
                self.assertEqual(
                    downloads[0]["with"]["name"],
                    f"${{{{ needs.build-{product}.outputs.artifact_name }}}}",
                )
                self.assertEqual(
                    downloads[1]["with"]["name"], f"server-ota-snapshot-{product}"
                )
                self.assertEqual(
                    downloads[1]["if"], "steps.recovery.outputs.state == 'open'"
                )

    def test_product_nightly_changes_trigger_build_system_tests(self):
        workflow = self.load_workflow("bos-build-tests.yml")
        paths = workflow.get("on", workflow.get(True))["pull_request"]["paths"]
        for filename in (
            "nightly-macos-browseros.yml",
            "nightly-macos-browserclaw.yml",
            "reusable-build-macos-nightly.yml",
        ):
            self.assertIn(f".github/workflows/{filename}", paths)

        release_checks = self.load_workflow("test.yml")
        release_paths = release_checks.get("on", release_checks.get(True))["pull_request"]["paths"]
        self.assertIn(".github/workflows/reusable-build-macos-nightly.yml", release_paths)

    def test_internal_builder_uses_reservation_and_frozen_artifact_source(self):
        workflow = self.load_workflow("reusable-build-macos-nightly.yml")
        triggers = workflow.get("on", workflow.get(True))
        self.assertEqual(workflow["name"], "Reusable: Build macOS nightly")
        self.assertEqual(set(triggers), {"workflow_call"})
        self.assertNotIn("workflow_dispatch", triggers)
        self.assertEqual(
            set(triggers["workflow_call"]["inputs"]),
            {
                "product",
                "state_ref",
                "source_sha",
                "reservation_sha",
                "browser_version",
                "server_version",
                "extension_version",
                "onboarding_version",
            },
        )
        job = workflow["jobs"]["build"]
        self.assertEqual(
            job["concurrency"],
            {"group": "macos-build", "cancel-in-progress": False, "queue": "max"},
        )
        sync = self.named_step(
            workflow, "build", "Sync build repo to transaction reservation"
        )
        build = self.named_step(workflow, "build", "Build signed nightly")
        text = (WORKFLOW_DIR / "reusable-build-macos-nightly.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn("refs/pull/[1-9][0-9]*/head", sync["run"])
        self.assertIn('"+$STATE_REF:$state_remote_ref"', sync["run"])
        self.assertIn('git checkout --detach "$RESERVATION_SHA"', sync["run"])
        self.assertIn(
            'test "$(git rev-parse "${RESERVATION_SHA}^")" = "$SOURCE_SHA"',
            sync["run"],
        )
        self.assertIn(
            'git merge-base --is-ancestor "$RESERVATION_SHA" "$state_remote_ref"',
            sync["run"],
        )
        self.assertNotIn("STATE_SHA", text)
        self.assertEqual(
            build["env"]["BROWSEROS_BUILD_RESERVATION_SHA"],
            "${{ inputs.reservation_sha }}",
        )
        self.assertEqual(build["env"]["BROWSEROS_DEFER_R2_UPLOAD"], "1")
        self.assertEqual(
            build["env"]["ONBOARDING_RESOURCE_VERSION"],
            "${{ inputs.onboarding_version }}",
        )
        self.assertIn(
            "PROD_MACOS_BROWSEROS_PASSKEY_PROFILE_B64",
            triggers["workflow_call"]["secrets"],
        )
        self.assertIn(
            "PROD_MACOS_BROWSERCLAW_PASSKEY_PROFILE_B64",
            triggers["workflow_call"]["secrets"],
        )
        setup = self.named_step(workflow, "build", "Import macOS signing certificate")
        self.assertEqual(
            setup["env"]["PROD_MACOS_BROWSEROS_PASSKEY_PROFILE_B64"],
            "${{ secrets.PROD_MACOS_BROWSEROS_PASSKEY_PROFILE_B64 }}",
        )
        self.assertEqual(
            setup["env"]["PROD_MACOS_BROWSERCLAW_PASSKEY_PROFILE_B64"],
            "${{ secrets.PROD_MACOS_BROWSERCLAW_PASSKEY_PROFILE_B64 }}",
        )
        self.assertEqual(
            build["env"]["PROD_MACOS_BROWSEROS_PASSKEY_PROFILE_PATH"],
            "${{ steps.macos_signing.outputs.browseros_passkey_profile_path }}",
        )
        self.assertEqual(
            build["env"]["PROD_MACOS_BROWSERCLAW_PASSKEY_PROFILE_PATH"],
            "${{ steps.macos_signing.outputs.browserclaw_passkey_profile_path }}",
        )
        self.assertNotIn("BROWSERCLAW_ONBOARD_RESOURCE_VERSION", build["env"])
        for secret in (
            "R2_ACCESS_KEY_ID",
            "R2_ACCOUNT_ID",
            "R2_BUCKET",
            "R2_SECRET_ACCESS_KEY",
        ):
            self.assertNotIn(secret, triggers["workflow_call"].get("secrets", {}))
            self.assertNotIn(secret, build["env"])
        self.assertEqual(
            build["env"]["BROWSEROS_BUILD_SOURCE_SHA"],
            "${{ inputs.source_sha }}",
        )
        for token in (
            "--profile nightly-macos",
            '--product "$PRODUCT"',
            "--arch arm64",
            "--resource-mode published",
        ):
            self.assertIn(token, build["run"])
        self.assertIn("browseros source clean-outputs", build["run"])
        self.assertNotIn("--modules clean", build["run"])
        workspace_setup = self.named_step(workflow, "build", "Setup disposable Chromium workspace")
        self.assertIn("${{ inputs.product }}", workspace_setup["run"])
        self.assertIn("Clean up disposable Chromium workspace", text)
        self.assertIn("Clean up macOS signing keychain", text)
        self.assertIn("actions/upload-artifact@v7", text)
        self.assertIn("release.json", text)
        self.assertNotIn("gh release create", text)

@unittest.skipIf(os.name == "nt", "macOS workspace helper shell tests run on POSIX")
class MacOSChromiumWorkspaceHelperTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.runner_temp = self.root / "runner"
        self.runner_temp.mkdir()
        self.bin_dir = self.root / "bin"
        self.bin_dir.mkdir()
        self.github_env = self.root / "github_env"
        self.github_output = self.root / "github_output"
        self.git_log = self.root / "git.log"
        self.git_clean_log = self.root / "git-clean.log"
        self.cp_log = self.root / "cp.log"
        self.version_file = self.root / "CHROMIUM_VERSION"
        self.version_file.write_text(
            "MAJOR=1\nMINOR=2\nBUILD=3\nPATCH=4\n",
            encoding="utf-8",
        )
        self.base_root = self.root / "chromium-base"
        self.base_src = self.base_root / "src"
        self.base_src.mkdir(parents=True)
        (self.base_root / ".gclient").write_text("solutions = []\n")
        (self.base_src / ".git").mkdir()
        self.base_root_resolved = self.base_root.resolve()
        self.base_src_resolved = self.base_src.resolve()
        self.head = "a" * 40
        self._write_fake_uv()
        self._write_fake_git()
        self._write_fake_cp()
        depot_tools = self.base_root / "depot_tools"
        (depot_tools / ".git").mkdir(parents=True)
        gclient = depot_tools / "gclient"
        gclient.write_text("#!/usr/bin/env bash\nset -euo pipefail\nprintf '%s\\n' \"$*\" >> \"$GCLIENT_LOG\"\n")
        gclient.chmod(0o755)

    def tearDown(self):
        self.tmp.cleanup()

    def _write_fake_uv(self):
        import sys

        # Exercise the shell-to-CLI handoff on Linux CI, replacing only the
        # macOS platform/device facts and external git/gclient/cp executables.
        uv = self.bin_dir / "uv"
        uv.write_text(
            f"#!{sys.executable}\n"
            f"import sys\nsys.path.insert(0, {str(REPO_ROOT / 'packages/browseros')!r})\n"
            """import os
from pathlib import Path
from types import SimpleNamespace
from bos_build.cli.source import app
from bos_build.steps.source import workspace
workspace.sys = SimpleNamespace(platform='darwin')
if os.environ.get('STAT_SPLIT_WORKSPACE_PARENT') == '1':
    real_stat = Path.stat
    def split_stat(path, *args, **kwargs):
        result = real_stat(path, *args, **kwargs)
        if path.name == 'browseros-ci-apfs-workspaces':
            values = list(result)
            values[2] += 1
            return os.stat_result(values)
        return result
    Path.stat = split_stat
app(args=sys.argv[sys.argv.index('source') + 1:])
"""
        )
        uv.chmod(0o755)

    def _write_fake_git(self):
        git = self.bin_dir / "git"
        git.write_text(
            """#!/usr/bin/env bash
set -euo pipefail
printf '%s\\n' "$*" >> "$GIT_LOG"
repo=""
if [ "${1:-}" = "-C" ]; then
  repo="$2"
  shift 2
fi
cmd="${1:-}"
shift || true
repo="${repo:-$PWD}"
repo_was_cleaned() {
  [ -f "$GIT_CLEAN_LOG" ] || return 1
  while IFS= read -r cleaned_repo; do
    [ "$cleaned_repo" = "$repo" ] && return 0
  done < "$GIT_CLEAN_LOG"
  return 1
}
case "$cmd" in
  rev-parse)
    target="${@: -1}"
    case "$target" in
      HEAD)
        if [ -f "$GIT_CHECKOUT_HEAD" ]; then
          cat "$GIT_CHECKOUT_HEAD"
        else
          printf '%s\\n' "${GIT_HEAD:-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa}"
        fi
        ;;
      --is-shallow-repository) printf 'false\\n' ;;
      refs/tags/*)
        printf '%s\\n' "${GIT_PIN_HEAD:-${GIT_HEAD:-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa}}"
        ;;
      *)
        printf '%s\\n' "${GIT_HEAD:-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa}"
        ;;
    esac
    ;;
  status)
    repo_was_cleaned && exit 0
    if [ -n "${GIT_DIRTY_REPO:-}" ]; then
      if [ "$repo" = "$GIT_DIRTY_REPO" ]; then
        printf '%b' "${GIT_DIRTY_STATUS:- M nested-change\\n}"
      fi
    else
      printf '%b' "${GIT_STATUS:-}"
    fi
    ;;
  clean)
    printf '%s\\n' "$repo" >> "$GIT_CLEAN_LOG"
    ;;
  checkout)
    printf '%s\\n' "${GIT_PIN_HEAD:-$GIT_HEAD}" > "$GIT_CHECKOUT_HEAD"
    ;;
esac
"""
        )
        git.chmod(0o755)

    def _write_fake_cp(self):
        cp = self.bin_dir / "cp"
        cp.write_text(
            """#!/usr/bin/env bash
set -euo pipefail
printf '%s\\n' "$*" >> "$CP_LOG"
case "${1:-}" in
  -c)
    [ "${CP_FAIL_PROBE:-}" != "1" ] || exit 65
    /bin/cp "$2" "$3"
    ;;
  -cR)
    if [ "${CP_FAIL_CLONE:-}" = "1" ] && [[ "$3" == *"-browserclaw/"* ]]; then
      printf 'partial copy\\n' > "$(dirname "$3")/partial-copy"
      exit 66
    fi
    /bin/cp -R "$2" "$3"
    ;;
  *)
    printf 'unexpected cp invocation: %s\\n' "$*" >&2
    exit 64
    ;;
esac
"""
        )
        cp.chmod(0o755)

    def _env(self, **overrides):
        env = os.environ.copy()
        env.update(
            {
                "BROWSEROS_CHROMIUM_VERSION_FILE": str(self.version_file),
                "CP_LOG": str(self.cp_log),
                "GIT_HEAD": self.head,
                "GIT_CHECKOUT_HEAD": str(self.root / "git-checkout-head"),
                "GCLIENT_LOG": str(self.root / "gclient.log"),
                "GIT_CLEAN_LOG": str(self.git_clean_log),
                "GIT_LOG": str(self.git_log),
                "GITHUB_ENV": str(self.github_env),
                "GITHUB_OUTPUT": str(self.github_output),
                "GITHUB_RUN_ATTEMPT": "4",
                "GITHUB_RUN_ID": "123",
                "PATH": f"{self.bin_dir}{os.pathsep}{env['PATH']}",
                "RUNNER_TEMP": str(self.runner_temp),
            }
        )
        env.update(overrides)
        return env

    def _run_helper(self, command, *args, **env_overrides):
        return subprocess.run(
            ["bash", str(MACOS_CHROMIUM_WORKSPACE_HELPER), command, *map(str, args)],
            capture_output=True,
            env=self._env(**env_overrides),
            text=True,
        )

    def _outputs(self):
        values = {}
        if not self.github_output.exists():
            return values
        for line in self.github_output.read_text(encoding="utf-8").splitlines():
            name, value = line.split("=", 1)
            values[name] = value
        return values

    def _workspace_parent(self):
        return self.base_root_resolved.parent / "browseros-ci-apfs-workspaces"

    def _workspace_root(self, tag="123-4", product="browseros"):
        return self._workspace_parent() / f"browseros-ci-chromium-{tag}-{product}"

    def _state_path(self, tag="123-4", product="browseros"):
        return self.runner_temp / f"browseros-ci-chromium-workspace-{tag}-{product}.env"

    def _write_workspace_marker(self, workspace_root, tag="old-1"):
        marker = workspace_root / ".browseros-workspace-state.env"
        marker.write_text(
            "\n".join(
                (
                    f"workspace_parent={self._workspace_parent()}",
                    f"workspace_root={workspace_root}",
                    f"workspace_src={workspace_root / 'src'}",
                    f"base_root={self.base_root_resolved}",
                    f"base_src={self.base_src_resolved}",
                    f"base_head={self.head}",
                    "chromium_version=1.2.3.4",
                    f"run_tag={tag}",
                    "product=browseros",
                )
            )
            + "\n",
            encoding="utf-8",
        )

    def test_setup_clones_gclient_root_and_cleanup_removes_workspace_only(self):
        result = self._run_helper("setup", self.base_src)
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)

        outputs = self._outputs()
        workspace_root = Path(outputs["workspace_root"])
        workspace_src = Path(outputs["chromium_src"])
        state_path = Path(outputs["state_path"])

        self.assertEqual(workspace_src, workspace_root / "src")
        self.assertEqual(outputs["base_src"], str(self.base_src_resolved))
        self.assertEqual(outputs["base_head"], self.head)
        self.assertTrue((workspace_root / ".gclient").is_file())
        self.assertTrue((workspace_src / ".git").is_dir())
        self.assertTrue((workspace_root / ".browseros-workspace-state.env").is_file())
        self.assertTrue(state_path.is_file())
        self.assertTrue(self.base_src.is_dir())

        env_lines = self.github_env.read_text(encoding="utf-8").splitlines()
        self.assertIn(f"CHROMIUM_SRC={workspace_src}", env_lines)
        self.assertEqual(outputs["browseros_state_path"], str(state_path))
        self.assertEqual(outputs["browseros_chromium_src"], str(workspace_src))
        cp_lines = self.cp_log.read_text(encoding="utf-8").splitlines()
        self.assertTrue(any(line.startswith("-c ") for line in cp_lines))
        self.assertTrue(any(line.startswith("-cR ") for line in cp_lines))
        self.assertFalse(any(line.startswith("-R ") for line in cp_lines))
        locks = list((workspace_root / ".browseros-build-locks").iterdir())
        self.assertEqual(len(locks), 1)
        lock_inode = locks[0].stat().st_ino

        cleanup = self._run_helper(
            "cleanup",
            MACOS_CHROMIUM_WORKSPACE_STATE_PATH=str(state_path),
        )
        self.assertEqual(cleanup.returncode, 0, cleanup.stderr + cleanup.stdout)
        self.assertEqual(
            {path.name for path in workspace_root.iterdir()},
            {".browseros-build-locks", ".browseros-workspace-state.env"},
        )
        self.assertFalse(state_path.exists())
        self.assertTrue(self.base_src.is_dir())
        self.assertEqual(locks[0].stat().st_ino, lock_inode)
        self.assertEqual(self._run_helper("cleanup").returncode, 0)
        self.assertEqual(locks[0].stat().st_ino, lock_inode)

    def test_setup_reaps_only_marked_stale_owned_workspaces(self):
        parent = self._workspace_parent()
        parent.mkdir()
        stale = self._workspace_root("old-1")
        stale.mkdir()
        (stale / "src").mkdir()
        self._write_workspace_marker(stale, tag="old-1")
        unmarked = self._workspace_root("unmarked")
        unmarked.mkdir()

        result = self._run_helper("setup", self.base_src)

        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertFalse((stale / "src").exists())
        self.assertTrue((stale / ".browseros-workspace-state.env").exists())
        self.assertTrue(unmarked.exists())
        self.assertTrue(Path(self._outputs()["workspace_root"]).exists())
        self.assertTrue(self.base_root.exists())

    def test_setup_reaps_stale_workspaces_before_repairing_dirty_base(self):
        parent = self._workspace_parent()
        parent.mkdir()
        stale = self._workspace_root("old-1")
        stale.mkdir()
        (stale / "src").mkdir()
        self._write_workspace_marker(stale, tag="old-1")

        result = self._run_helper(
            "setup",
            self.base_src,
            GIT_STATUS=" M chrome/app/generated_resources.grd\n",
        )

        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertFalse((stale / "src").exists())
        self.assertTrue(self._workspace_root().exists())
        self.assertTrue(self.base_root.exists())

    def test_cleanup_ignores_unsafe_state_target(self):
        state_path = self._state_path()
        state_path.write_text(
            "\n".join(
                (
                    f"workspace_parent={self.root}",
                    "workspace_root=/",
                    "workspace_src=/src",
                    f"base_root={self.base_root}",
                    f"base_src={self.base_src}",
                    f"base_head={self.head}",
                    "chromium_version=1.2.3.4",
                    "run_tag=old-1",
                )
            )
            + "\n",
            encoding="utf-8",
        )

        result = self._run_helper("cleanup")

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Workspace cleanup failed", result.stderr + result.stdout)
        self.assertTrue(state_path.exists())
        self.assertTrue(self.base_root.exists())

    def test_setup_does_not_use_unowned_state_override(self):
        outside_state = self.root / "outside.env"

        result = self._run_helper(
            "setup",
            self.base_src,
            MACOS_CHROMIUM_WORKSPACE_STATE_PATH=str(outside_state),
        )

        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertFalse(outside_state.exists())
        self.assertTrue(self._state_path().exists())

    def test_setup_fails_when_workspace_parent_is_not_on_base_volume(self):
        result = self._run_helper(
            "setup",
            self.base_src,
            STAT_SPLIT_WORKSPACE_PARENT="1",
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn(
            "must share an APFS volume",
            result.stderr + result.stdout,
        )
        self.assertFalse(self._workspace_root().exists())

    def test_setup_fails_without_full_copy_fallback_when_apfs_clone_fails(self):
        result = self._run_helper("setup", self.base_src, "all", CP_FAIL_CLONE="1")

        self.assertNotEqual(result.returncode, 0)
        for product in ("browseros", "browserclaw"):
            root = self._workspace_root(product=product)
            self.assertEqual(
                {path.name for path in root.iterdir()},
                {".browseros-build-locks", ".browseros-workspace-state.env"},
            )
            self.assertFalse(self._state_path(product=product).exists())
        self.assertEqual(self._outputs(), {})
        cp_lines = self.cp_log.read_text(encoding="utf-8").splitlines()
        self.assertTrue(any(line.startswith("-cR ") for line in cp_lines))
        self.assertFalse(any(line.startswith("-R ") for line in cp_lines))

    def test_setup_refreshes_base_when_not_at_pinned_chromium_tag(self):
        result = self._run_helper("setup", self.base_src, GIT_PIN_HEAD="b" * 40)

        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertEqual(self._outputs()["base_head"], "b" * 40)
        self.assertIn(
            "checkout --force --detach refs/tags/1.2.3.4", self.git_log.read_text()
        )
        self.assertTrue(self._workspace_root().exists())

    def test_setup_repairs_base_with_tracked_changes(self):
        result = self._run_helper(
            "setup",
            self.base_src,
            GIT_STATUS=" M chrome/app/generated_resources.grd\n",
        )

        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertTrue(self._workspace_root().exists())

    def test_setup_repairs_base_with_untracked_changes(self):
        result = self._run_helper(
            "setup",
            self.base_src,
            GIT_STATUS="?? chrome/browser/browseros/generated_resources.grd\n",
        )

        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertTrue(self._workspace_root().exists())

    def test_setup_repairs_nested_gclient_repo_with_changes(self):
        nested_repo = self.base_src / "third_party" / "v8"
        nested_repo.mkdir(parents=True)
        (nested_repo / ".git").mkdir()
        (self.base_root / ".gclient_entries").write_text(
            "entries = {'src/third_party/v8': 'fixture'}\n"
        )

        result = self._run_helper(
            "setup",
            self.base_src,
            GIT_DIRTY_REPO=str(nested_repo.resolve()),
            GIT_DIRTY_STATUS=" M src/builtins/generated.cc\n",
        )

        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertTrue(self._workspace_root().exists())
        self.assertIn(str(nested_repo.resolve()), self.git_clean_log.read_text())

    def test_setup_removes_browseros_output_dirs_from_base(self):
        out_dir = self.base_src / "out" / "Default_browseros_arm64"
        out_dir.mkdir(parents=True)

        result = self._run_helper("setup", self.base_src, "all")

        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertFalse(out_dir.exists())
        for product in ("browseros", "browserclaw"):
            self.assertTrue(self._workspace_root(product=product).exists())
            self.assertIn(f"{product}_chromium_src", self._outputs())
        self.assertEqual(len((self.root / "gclient.log").read_text().splitlines()), 1)


@unittest.skipIf(os.name == "nt", "macOS signing helper shell tests run on POSIX")
class MacOSSigningKeychainHelperTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.runner_temp = self.root / "runner"
        self.runner_temp.mkdir()
        self.bin_dir = self.root / "bin"
        self.bin_dir.mkdir()
        self.security_log = self.root / "security.log"
        self.codesign_log = self.root / "codesign.log"
        self.github_env = self.root / "github_env"
        self.github_output = self.root / "github_output"
        self.original_keychain = self.root / "login.keychain-db"
        self.original_default = self.original_keychain
        self.identity_sha1 = "0123456789abcdef0123456789abcdef01234567"
        self._write_fake_security()
        self._write_fake_codesign()

    def tearDown(self):
        self.tmp.cleanup()

    def _write_fake_security(self):
        security = self.bin_dir / "security"
        security.write_text(
            """#!/usr/bin/env bash
set -euo pipefail
printf '%s\\n' "$*" >> "$SECURITY_LOG"
cmd="$1"
shift || true
last="${@: -1}"
case "$cmd" in
  list-keychains)
    if [[ " $* " == *" -s "* ]]; then
      exit 0
    fi
    printf '"%s"\\n' "$ORIGINAL_KEYCHAIN"
    if [ -n "${EXTRA_KEYCHAIN:-}" ]; then
      printf '"%s"\\n' "$EXTRA_KEYCHAIN"
    fi
    ;;
  default-keychain)
    if [[ " $* " == *" -s "* ]]; then
      exit 0
    fi
    printf '"%s"\\n' "$ORIGINAL_DEFAULT_KEYCHAIN"
    ;;
  create-keychain)
    touch "$last"
    ;;
  delete-keychain)
    rm -f "$last"
    ;;
  import)
    if [ "${SECURITY_FAIL_IMPORT:-}" = "1" ]; then
      exit 42
    fi
    ;;
  find-identity)
    if [ -n "${SECURITY_FIND_IDENTITIES:-}" ]; then
      printf '%b' "$SECURITY_FIND_IDENTITIES"
    else
      printf '  1) %s "%s"\\n' "${SECURITY_IDENTITY_SHA1:-0123456789abcdef0123456789abcdef01234567}" "$MACOS_CERTIFICATE_NAME"
      if [ -n "${SECURITY_EXTRA_IDENTITY_SHA1:-}" ]; then
        printf '  2) %s "%s"\\n' "$SECURITY_EXTRA_IDENTITY_SHA1" "${SECURITY_EXTRA_IDENTITY_NAME:-$MACOS_CERTIFICATE_NAME}"
      fi
    fi
    ;;
  unlock-keychain|set-keychain-settings|set-key-partition-list|show-keychain-info|lock-keychain)
    ;;
esac
"""
        )
        security.chmod(0o755)

    def _write_fake_codesign(self):
        codesign = self.bin_dir / "codesign"
        codesign.write_text(
            """#!/usr/bin/env bash
set -euo pipefail
printf '%s\\n' "$*" >> "$CODESIGN_LOG"
signing_identity=""
args=("$@")
for ((i = 0; i < ${#args[@]}; i++)); do
  if [ "${args[$i]}" = "--sign" ]; then
    signing_identity="${args[$((i + 1))]}"
  fi
done
if [ "${CODESIGN_FAIL_SIGN:-}" = "1" ] && [ -n "$signing_identity" ]; then
  exit 45
fi
if [ "${CODESIGN_FAIL_VERIFY:-}" = "1" ] && [ "${1:-}" = "--verify" ]; then
  exit 46
fi
if [ "${CODESIGN_REJECT_COMMON_NAME:-}" = "1" ] && [ "$signing_identity" = "$MACOS_CERTIFICATE_NAME" ]; then
  printf '%s: ambiguous\\n' "$signing_identity" >&2
  exit 47
fi
"""
        )
        codesign.chmod(0o755)

    def _env(self, **overrides):
        env = os.environ.copy()
        env.update(
            {
                "CODESIGN_LOG": str(self.codesign_log),
                "GITHUB_ENV": str(self.github_env),
                "GITHUB_OUTPUT": str(self.github_output),
                "GITHUB_RUN_ATTEMPT": "4",
                "GITHUB_RUN_ID": "123",
                "MACOS_CERTIFICATE_NAME": "Developer ID Application",
                "MACOS_CERTIFICATE_P12": "ZmFrZS1wMTI=",
                "MACOS_CERTIFICATE_PWD": "p12-password",
                "MACOS_KEYCHAIN_PASSWORD": "keychain-password",
                "ORIGINAL_DEFAULT_KEYCHAIN": str(self.original_default),
                "ORIGINAL_KEYCHAIN": str(self.original_keychain),
                "PATH": f"{self.bin_dir}{os.pathsep}{env['PATH']}",
                "RUNNER_TEMP": str(self.runner_temp),
                "SECURITY_LOG": str(self.security_log),
                "SECURITY_IDENTITY_SHA1": self.identity_sha1,
            }
        )
        env.update(overrides)
        return env

    def _run_helper(self, command, **env_overrides):
        return subprocess.run(
            ["bash", str(MACOS_SIGNING_HELPER), command],
            capture_output=True,
            env=self._env(**env_overrides),
            text=True,
        )

    def _outputs(self):
        values = {}
        for line in self.github_output.read_text(encoding="utf-8").splitlines():
            name, value = line.split("=", 1)
            values[name] = value
        return values

    def test_setup_imports_p12_and_cleanup_restores_keychain_state(self):
        result = self._run_helper("setup")
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        outputs = self._outputs()
        keychain_path = Path(outputs["keychain_path"])
        state_path = Path(outputs["state_path"])
        cert_path = self.runner_temp / "browseros-signing-cert-123-4.p12"
        smoke_path = self.runner_temp / "browseros-ci-codesign-smoke-123-4"

        self.assertEqual(outputs["codesign_identity"], self.identity_sha1)
        self.assertEqual(outputs["browseros_passkey_profile_path"], "")
        self.assertEqual(outputs["browserclaw_passkey_profile_path"], "")
        self.assertTrue(keychain_path.exists())
        self.assertTrue(state_path.exists())
        self.assertFalse(cert_path.exists())
        env_lines = self.github_env.read_text(encoding="utf-8").splitlines()
        self.assertIn(f"MACOS_CERTIFICATE_NAME={self.identity_sha1}", env_lines)
        self.assertIn(f"MACOS_KEYCHAIN_PATH={keychain_path}", env_lines)
        self.assertFalse(smoke_path.exists())

        cleanup = self._run_helper(
            "cleanup",
            MACOS_SIGNING_STATE_PATH=str(state_path),
        )
        self.assertEqual(cleanup.returncode, 0, cleanup.stderr + cleanup.stdout)

        log = self.security_log.read_text(encoding="utf-8")
        self.assertIn(f"import {self.runner_temp}", log)
        self.assertIn("set-key-partition-list", log)
        self.assertIn(f"find-identity -v -p codesigning {keychain_path}", log)
        self.assertIn(
            f"list-keychains -d user -s {keychain_path} {self.original_keychain}",
            log,
        )
        self.assertIn(
            f"default-keychain -d user -s {self.original_keychain}",
            log,
        )
        codesign_log = self.codesign_log.read_text(encoding="utf-8")
        self.assertIn(
            f"--sign {self.identity_sha1} --force --timestamp=none --keychain {keychain_path} {smoke_path}",
            codesign_log,
        )
        self.assertIn(f"--verify --verbose=2 {smoke_path}", codesign_log)
        self.assertFalse(keychain_path.exists())
        self.assertFalse(state_path.exists())

    def test_setup_decodes_both_passkey_profiles_and_cleanup_removes_them(self):
        result = self._run_helper(
            "setup",
            PROD_MACOS_BROWSEROS_PASSKEY_PROFILE_B64="cHJvZmlsZS1ieXRlcw==",
            PROD_MACOS_BROWSERCLAW_PASSKEY_PROFILE_B64="Y2xhdy1wcm9maWxlLWJ5dGVz",
        )
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        outputs = self._outputs()
        browseros_profile_path = Path(outputs["browseros_passkey_profile_path"])
        browserclaw_profile_path = Path(outputs["browserclaw_passkey_profile_path"])
        self.assertEqual(browseros_profile_path.read_bytes(), b"profile-bytes")
        self.assertEqual(browserclaw_profile_path.read_bytes(), b"claw-profile-bytes")
        self.assertEqual(browseros_profile_path.stat().st_mode & 0o777, 0o600)
        self.assertEqual(browserclaw_profile_path.stat().st_mode & 0o777, 0o600)
        env_lines = self.github_env.read_text(encoding="utf-8").splitlines()
        self.assertIn(
            f"PROD_MACOS_BROWSEROS_PASSKEY_PROFILE_PATH={browseros_profile_path}",
            env_lines,
        )
        self.assertIn(
            f"PROD_MACOS_BROWSERCLAW_PASSKEY_PROFILE_PATH={browserclaw_profile_path}",
            env_lines,
        )

        cleanup = self._run_helper(
            "cleanup",
            MACOS_SIGNING_STATE_PATH=outputs["state_path"],
        )
        self.assertEqual(cleanup.returncode, 0, cleanup.stderr + cleanup.stdout)
        self.assertFalse(browseros_profile_path.exists())
        self.assertFalse(browserclaw_profile_path.exists())

    def test_setup_uses_fingerprint_when_common_name_is_duplicated(self):
        result = self._run_helper("setup", CODESIGN_REJECT_COMMON_NAME="1")
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        outputs = self._outputs()
        self.assertEqual(outputs["codesign_identity"], self.identity_sha1)
        self.assertNotIn("ambiguous", result.stderr + result.stdout)

        cleanup = self._run_helper(
            "cleanup",
            MACOS_SIGNING_STATE_PATH=outputs["state_path"],
        )
        self.assertEqual(cleanup.returncode, 0, cleanup.stderr + cleanup.stdout)

    def test_setup_rejects_missing_matching_identity(self):
        result = self._run_helper(
            "setup",
            SECURITY_FIND_IDENTITIES=(
                f'  1) {self.identity_sha1} "Developer ID Application Extra"\\n'
            ),
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn(
            "does not expose the configured macOS signing identity",
            result.stderr + result.stdout,
        )
        keychain_path = self.runner_temp / "browseros-ci-signing-123-4.keychain-db"
        cert_path = self.runner_temp / "browseros-signing-cert-123-4.p12"
        state_path = self.runner_temp / "browseros-ci-signing-keychain-state.env"
        self.assertFalse(keychain_path.exists())
        self.assertFalse(cert_path.exists())
        self.assertFalse(state_path.exists())

    def test_setup_rejects_multiple_matching_identities(self):
        result = self._run_helper(
            "setup",
            SECURITY_EXTRA_IDENTITY_SHA1="fedcba9876543210fedcba9876543210fedcba98",
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn(
            "multiple matching macOS signing identities",
            result.stderr + result.stdout,
        )
        keychain_path = self.runner_temp / "browseros-ci-signing-123-4.keychain-db"
        state_path = self.runner_temp / "browseros-ci-signing-keychain-state.env"
        self.assertFalse(keychain_path.exists())
        self.assertFalse(state_path.exists())

    def test_setup_failure_cleans_smoke_target(self):
        result = self._run_helper("setup", CODESIGN_FAIL_SIGN="1")

        self.assertNotEqual(result.returncode, 0)
        keychain_path = self.runner_temp / "browseros-ci-signing-123-4.keychain-db"
        cert_path = self.runner_temp / "browseros-signing-cert-123-4.p12"
        smoke_path = self.runner_temp / "browseros-ci-codesign-smoke-123-4"
        state_path = self.runner_temp / "browseros-ci-signing-keychain-state.env"
        self.assertFalse(keychain_path.exists())
        self.assertFalse(cert_path.exists())
        self.assertFalse(smoke_path.exists())
        self.assertFalse(state_path.exists())

    def test_setup_failure_cleans_partial_keychain_and_certificate(self):
        result = self._run_helper("setup", SECURITY_FAIL_IMPORT="1")

        self.assertNotEqual(result.returncode, 0)
        keychain_path = self.runner_temp / "browseros-ci-signing-123-4.keychain-db"
        cert_path = self.runner_temp / "browseros-signing-cert-123-4.p12"
        state_path = self.runner_temp / "browseros-ci-signing-keychain-state.env"
        log = self.security_log.read_text(encoding="utf-8")

        self.assertIn(f"lock-keychain {keychain_path}", log)
        self.assertIn(f"delete-keychain {keychain_path}", log)
        self.assertFalse(keychain_path.exists())
        self.assertFalse(cert_path.exists())
        self.assertFalse(state_path.exists())

    def test_setup_rejects_unowned_state_path(self):
        unowned_state_path = self.root / "outside-state.env"

        result = self._run_helper(
            "setup",
            MACOS_SIGNING_STATE_PATH=str(unowned_state_path),
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn(
            "Unexpected macOS signing state path",
            result.stdout + result.stderr,
        )
        self.assertFalse(unowned_state_path.exists())

    def test_setup_first_cleans_previous_recorded_state(self):
        old_keychain_path = self.runner_temp / "browseros-ci-signing-old-1.keychain-db"
        old_cert_path = self.runner_temp / "browseros-signing-cert-old-1.p12"
        old_originals_path = (
            self.runner_temp / "browseros-ci-original-keychains-old-1.txt"
        )
        old_state_path = self.runner_temp / "browseros-ci-signing-keychain-state.env"
        old_original = self.root / "old-login.keychain-db"
        old_keychain_path.write_text("old-keychain")
        old_cert_path.write_text("old-cert")
        old_originals_path.write_text(f"{old_original}\n", encoding="utf-8")
        old_state_path.write_text(
            "\n".join(
                (
                    f"cert_path={old_cert_path}",
                    f"keychain_path={old_keychain_path}",
                    f"original_default_keychain={old_original}",
                    f"original_keychains_file={old_originals_path}",
                )
            )
            + "\n",
            encoding="utf-8",
        )

        result = self._run_helper("setup")

        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        log = self.security_log.read_text(encoding="utf-8")
        self.assertIn(f"lock-keychain {old_keychain_path}", log)
        self.assertIn(f"delete-keychain {old_keychain_path}", log)
        self.assertIn(f"default-keychain -d user -s {old_original}", log)
        self.assertFalse(old_keychain_path.exists())
        self.assertFalse(old_cert_path.exists())
        self.assertFalse(old_originals_path.exists())
        self.assertTrue(old_state_path.exists())

    def test_setup_filters_stale_ci_keychains_from_restored_search_list(self):
        stale_keychain = self.runner_temp / "browseros-ci-signing-stale-1.keychain-db"
        stale_keychain.write_text("stale")

        result = self._run_helper("setup", EXTRA_KEYCHAIN=str(stale_keychain))
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        outputs = self._outputs()
        cleanup = self._run_helper(
            "cleanup",
            MACOS_SIGNING_STATE_PATH=outputs["state_path"],
        )

        self.assertEqual(cleanup.returncode, 0, cleanup.stderr + cleanup.stdout)
        log = self.security_log.read_text(encoding="utf-8")
        self.assertIn(f"delete-keychain {stale_keychain}", log)
        self.assertNotIn(
            f"list-keychains -d user -s {outputs['keychain_path']} {self.original_keychain} {stale_keychain}",
            log,
        )
        self.assertNotIn(
            f"list-keychains -d user -s {self.original_keychain} {stale_keychain}",
            log,
        )
        self.assertFalse(stale_keychain.exists())


class ReleaseDocumentationTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        docs = REPO_ROOT / "packages/browseros/bos_build"
        cls.readme = (docs / "README.md").read_text(encoding="utf-8")
        cls.release = (docs / "docs/release-ci.md").read_text(encoding="utf-8")
        cls.nightly = (docs / "docs/nightly-macos-ci.md").read_text(encoding="utf-8")
        cls.warp = (docs / "docs/warpbuild-ci.md").read_text(encoding="utf-8")

    def test_release_runbook_covers_ordered_published_release(self):
        for token in (
            "publish server release, latest resources, and alpha OTA",
            "publish extension release, versioned CRX, and alpha/bundled feeds",
            "resource_mode: published",
            'gh run rerun "$RUN_ID" --failed',
            "--resource-mode published",
        ):
            with self.subTest(token=token):
                self.assertIn(token, self.release)

    def test_readme_lists_both_independent_onboarding_bundles(self):
        for token in (
            "apps/app-onboard/package.json",
            "release-app-onboard.yml",
            "app-onboard/v*",
            "apps/claw-onboard/package.json",
            "release-claw-onboard.yml",
            "claw-onboard/v*",
        ):
            with self.subTest(token=token):
                self.assertIn(token, self.readme)

    def test_runbooks_state_native_host_and_publication_boundaries(self):
        for token in (
            "Linux x64",
            "Windows x64",
            "macOS universal",
            "It does not publish the\nbrowser appcast",
        ):
            with self.subTest(token=token):
                self.assertIn(token, self.release)
        self.assertIn("resource_mode: published", self.nightly)
        self.assertIn("--resource-mode published", self.nightly)
        self.assertIn("`queue: max`", self.nightly)
        self.assertIn("Published mode deliberately retains", self.warp)

    def test_nightly_runbook_covers_product_transactions_and_family_cutover(self):
        for token in (
            ".github/workflows/nightly-macos-browseros.yml",
            ".github/workflows/nightly-macos-browserclaw.yml",
            ".github/workflows/reusable-build-macos-nightly.yml",
            "nightly-<product>-<source-sha>",
            "Source SHA",
            "Reservation SHA",
            "State SHA",
            "Merge SHA",
            "refs/pull/<PR_NUMBER>/head",
            "created as a draft",
            "Open, closed, and merged canonical suite",
            "--baseline-root",
            "--state-base-sha",
            "Re-run failed jobs",
            "conditionally creates",
            "before exposing any mutable feeds",
            "superseded/no-op",
            "partial draft resumes",
            "--allow-downgrade",
            "release-suite",
            "Historical runs",
            "omitting",
            "--product",
            "Historical standalone extension-feed runs",
            "current committed feed files",
            "Rewriting `main` history remains unsupported",
        ):
            with self.subTest(token=token):
                self.assertIn(token, self.nightly)
        self.assertLess(
            self.nightly.index(
                "conditionally create or verify its immutable DMG and receipt"
            ),
            self.nightly.index(
                "publish current committed feeds and reconcile its rolling prerelease"
            ),
        )
        self.assertIn("Signed nightlies use independent product transactions", self.release)
        for token in (
            "always check out `reservation_sha`",
            "new whole-run invocation that finds the transaction already\nmerged fails closed",
        ):
            with self.subTest(token=token):
                self.assertIn(token, self.release)

    def test_primary_docs_do_not_describe_retired_release_behavior(self):
        text = "\n".join((self.readme, self.release, self.nightly, self.warp))
        for token in (
            "bundle_local_extensions",
            "extensions_version",
            "include_servers",
            "Stage BrowserOS nightly resources",
            "claw-server-rust-local.sh",
            "BROWSEROS_NIGHTLY_REF",
            "build the exact transaction-branch head",
            "verify it still resolves to the recorded state\nSHA",
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
            "`browseros source ensure --reset --repair-cached-depot-tools`"
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


class RepairUpdateFeedWorkflowTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        path = WORKFLOW_DIR / "repair-update-feed.yml"
        cls.workflow = yaml.safe_load(path.read_text(encoding="utf-8"))

    def test_dispatch_only_selects_registered_appcasts(self):
        triggers = self.workflow.get("on", self.workflow.get(True))
        inputs = triggers["workflow_dispatch"]["inputs"]
        self.assertEqual(
            inputs["feed"]["options"],
            [feed.key for feed in all_feeds() if feed.kind in ("browser", "server")],
        )
        self.assertFalse(inputs["publish"]["default"])
        self.assertFalse(inputs["repair_invalid_live"]["default"])
        self.assertNotIn("allow_downgrade", inputs)

    def test_uses_tracked_default_branch_and_guarded_publisher(self):
        self.assertEqual(
            self.workflow["concurrency"],
            {
                "group": "release-feed-snapshots",
                "cancel-in-progress": False,
                "queue": "max",
            },
        )
        job = self.workflow["jobs"]["repair"]
        self.assertEqual(job["permissions"], {"contents": "read"})
        checkout = next(
            step
            for step in job["steps"]
            if str(step.get("uses", "")).startswith("actions/checkout@")
        )
        self.assertEqual(
            checkout["with"]["ref"],
            "${{ github.event.repository.default_branch || 'main' }}",
        )
        publish = next(
            step
            for step in job["steps"]
            if step.get("name") == "Publish tracked appcast"
        )["run"]
        self.assertIn('args=("$FEED")', publish)
        self.assertIn("args+=(--repair-invalid-live)", publish)
        self.assertIn("args+=(--publish)", publish)
        self.assertIn('release feeds publish-local "${args[@]}"', publish)
        self.assertNotIn("--allow-downgrade", publish)



class DockerImageWorkflowTest(unittest.TestCase):
    def setUp(self):
        self.workflow = yaml.safe_load(
            (WORKFLOW_DIR / "release-docker.yml").read_text(encoding="utf-8")
        )
        self.jobs = self.workflow["jobs"]

    def test_only_jobs_that_push_images_can_write_packages(self):
        # verify pushes the image it tested by digest; publish names it.
        self.assertEqual(self.workflow["permissions"], {})
        for name, job in self.jobs.items():
            self.assertEqual(
                job["permissions"].get("packages"),
                "write" if name in ("verify", "publish") else None,
            )

    def test_server_builds_on_warpbuild_with_a_cargo_cache_and_a_queue_watchdog(self):
        server = self.jobs["server"]
        self.assertEqual(server["runs-on"], "warp-custom-browseros-ubuntu-2204-x64-32x")
        self.assertIn("timeout-minutes", server)
        cache = next(step for step in server["steps"]
                     if step.get("uses", "").startswith("Swatinem/rust-cache@"))
        self.assertEqual(cache["with"]["cache-provider"], "warpbuild")
        self.assertIn("~/.cargo/build/browseros", cache["with"]["cache-directories"])
        self.assertEqual(cache["with"]["shared-key"], "neo-docker-${{ matrix.rust_target }}")
        # Without the environment hash the key ignores Cargo.lock and never refreshes.
        self.assertNotIn("add-rust-environment-hash-key", cache["with"])
        watchdog = self.jobs["queue-watchdog"]["steps"][0]["env"]
        self.assertEqual(watchdog["RUNNER_LABEL"], server["runs-on"])
        self.assertEqual(watchdog["CONCURRENCY_GROUP"], self.workflow["concurrency"]["group"])

    def test_verify_checks_the_image_runs_this_commits_server_before_smoke(self):
        steps = self.jobs["verify"]["steps"]
        names = [step.get("name") for step in steps]
        check = names.index("Check the image runs this commit's server")
        self.assertLess(names.index("Build native image"), check)
        self.assertLess(check, next(i for i, step in enumerate(steps) if step.get("id") == "smoke"))
        run = steps[check]["run"]
        self.assertIn('sha256sum "$RUNNER_TEMP/neo-servers/$ARCH/browseros-claw-server-rs"', run)
        self.assertIn("find /usr/lib /opt -type f -name browseros-claw-server", run)

    def test_publish_tags_the_images_verify_tested_without_building(self):
        steps = self.jobs["verify"]["steps"]
        names = [step.get("name") for step in steps]
        smoke = next(i for i, step in enumerate(steps) if step.get("id") == "smoke")
        push = names.index("Push the tested image by digest")
        self.assertLess(smoke, push)
        self.assertEqual(steps[push]["if"], "inputs.push")
        self.assertIn("--push-by-digest", steps[push]["run"])
        self.assertLess(push, names.index("Check the pushed image is the one tested"))

        publish = self.jobs["publish"]
        self.assertEqual(publish["needs"], "verify")
        self.assertNotIn("environment", publish)
        uses = [step.get("uses", "") for step in publish["steps"]]
        self.assertFalse(any(use.startswith("docker/setup-qemu-action") for use in uses))
        tag = next(step for step in publish["steps"]
                   if step.get("name") == "Tag the tested amd64 and arm64 images")
        self.assertIn('--from-digests "$RUNNER_TEMP/neo-digests"', tag["run"])
        self.assertNotIn("--server-dir", tag["run"])

    def test_native_smoke_checks_on_both_architectures_gate_publication(self):
        verify = self.jobs["verify"]
        self.assertEqual(verify["runs-on"], "${{ matrix.runner }}")
        self.assertEqual(verify["strategy"]["matrix"]["include"], [
            {"arch": "amd64", "runner": "ubuntu-latest"},
            {"arch": "arm64", "runner": "ubuntu-24.04-arm"},
        ])
        self.assertIn("verify", self.jobs["publish"]["needs"])
        steps = {step.get("name"): step for step in verify["steps"]}
        self.assertNotIn("--push", steps["Build native image"]["run"])
        smoke = next(step for step in verify["steps"] if step.get("id") == "smoke")
        self.assertIn("for hosted in 1 0", smoke["run"])
        self.assertIn("for extensions_disabled in false true", smoke["run"])
        self.assertIn("< packages/browseros/docker/neo/check_extensions.py", smoke["run"])
        self.assertIn('captcha_smoke.py "http://127.0.0.1:$port" "$hosted" "$KEYED"', smoke["run"])
        self.assertEqual(self.jobs["publish"]["if"], "inputs.push")

    def test_keyed_jobs_scope_secrets_to_preparation_and_always_clean_files(self):
        # Only verify builds an image, so only verify holds provider keys.
        job = self.jobs["verify"]
        self.assertEqual(job["environment"], "neo-captcha")
        self.assertNotIn("CAPSOLVER_API_KEY", job.get("env", {}))
        self.assertNotIn("TWOCAPTCHA_API_KEY", job.get("env", {}))
        cleanup = job["steps"][-1]
        self.assertEqual(cleanup["if"], "always()")
        self.assertIn('rm -f "$RUNNER_TEMP/neo-capsolver-key" "$RUNNER_TEMP/neo-2captcha-key"', cleanup["run"])
        for name, other in self.jobs.items():
            if name != "verify":
                self.assertNotIn("CAPSOLVER_API_KEY", str(other))
                self.assertNotIn("TWOCAPTCHA_API_KEY", str(other))

    def test_server_override_preserves_production_telemetry_configuration(self):
        build = next(step for step in self.jobs["server"]["steps"]
                     if step.get("name") == "Build this commit's server")
        self.assertEqual(build["env"]["CLAW_POSTHOG_KEY"], "${{ secrets.CLAW_POSTHOG_KEY }}")
        self.assertEqual(build["env"]["CLAW_SENTRY_DSN"], "${{ secrets.CLAW_SENTRY_DSN }}")
        self.assertIn('if [ -z "$CLAW_POSTHOG_KEY" ]', build["run"])

    def test_inputs_reach_shell_only_through_environment(self):
        for job in self.jobs.values():
            for step in job["steps"]:
                self.assertNotIn("${{ inputs.", step.get("run", ""))


if __name__ == "__main__":
    unittest.main()
