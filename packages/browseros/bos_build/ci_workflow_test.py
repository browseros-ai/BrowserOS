#!/usr/bin/env python3
"""Regression tests for the reusable Chromium build workflow."""

import os
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
                self.assertEqual(build_with["arch"], "x64")

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
                self.assertIs(server["with"]["publish_ota"], True)
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
                self.assertIs(extension["with"]["publish_alpha_feed"], True)
                self.assertEqual(extension["secrets"], "inherit")

    def test_full_release_matrix_is_fixed_and_uses_published_resources(self):
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
                    if arch is not None:
                        self.assertEqual(job["with"]["arch"], arch)
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
                for token in (
                    "release github create",
                    f"--product {config['product']}",
                    "--platforms all",
                    "--source-sha",
                    "--workflow-run-id",
                    "--target",
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
                    "defer_finalize",
                ):
                    self.assertNotIn(token, text)

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
        self.assertIn(
            '--chromium-src "${{ steps.chromium_workspace.outputs.chromium_src }}"',
            build["run"],
        )
        self.assertNotIn(
            '--chromium-src "${{ steps.inputs.outputs.chromium_src }}"',
            build["run"],
        )
        self.assertEqual(workspace_cleanup["if"], "always()")
        self.assertEqual(
            workspace_cleanup["env"]["MACOS_CHROMIUM_WORKSPACE_STATE_PATH"],
            "${{ steps.chromium_workspace.outputs.state_path }}",
        )
        self.assertIn("macos-chromium-workspace.sh", workspace_cleanup["run"])
        self.assertIn(" cleanup", workspace_cleanup["run"])
        self.assertEqual(keychain_cleanup["if"], "always()")


class FamilyNightlyWorkflowTest(unittest.TestCase):
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

    def test_one_entrypoint_freezes_main_into_stable_suite_identity(self):
        self.assertTrue((WORKFLOW_DIR / "nightly.yml").is_file())
        self.assertFalse((WORKFLOW_DIR / "nightly-browseros.yml").exists())
        self.assertFalse((WORKFLOW_DIR / "nightly-browserclaw.yml").exists())
        self.assertFalse(
            (WORKFLOW_DIR / "reserve-nightly-browser-version.yml").exists()
        )

        workflow = self.load_workflow("nightly.yml")
        triggers = workflow.get("on", workflow.get(True))
        transaction = workflow["jobs"]["transaction"]
        checkout = next(
            step
            for step in transaction["steps"]
            if str(step.get("uses", "")).startswith("actions/checkout@")
        )
        reconcile = self.named_step(
            workflow, "transaction", "Reconcile family transaction"
        )
        text = (WORKFLOW_DIR / "nightly.yml").read_text(encoding="utf-8")

        self.assertIsNone(triggers["workflow_dispatch"])
        self.assertNotIn("schedule", triggers)
        self.assertEqual(workflow["permissions"], {})
        self.assertEqual(
            workflow["concurrency"],
            {
                "group": "release-suite",
                "cancel-in-progress": False,
                "queue": "max",
            },
        )
        self.assertEqual(checkout["with"]["ref"], "${{ github.sha }}")
        self.assertEqual(
            transaction["concurrency"],
            {
                "group": "release-component-allocation",
                "cancel-in-progress": False,
                "queue": "max",
            },
        )
        self.assertEqual(
            transaction["outputs"]["app_onboarding_version"],
            "${{ steps.transaction.outputs.app_onboarding_version }}",
        )
        for token in (
            '"$DEFAULT_BRANCH" != "main"',
            '"$SOURCE_REF" != "refs/heads/main"',
            'source_sha="$(git rev-parse HEAD)"',
            'test "$source_sha" = "$GITHUB_SHA"',
            "release suite reconcile",
            "--mode nightly",
            '--source-sha "$source_sha"',
            'if [ "$(jq -r \'.state\' "$RUNNER_TEMP/release-suite.json")" != "open" ]; then',
            "Only an open transaction can start builds",
            "Re-run failed jobs",
        ):
            self.assertIn(token, reconcile["run"])
        self.assertNotIn("GITHUB_RUN_ATTEMPT", text)

    def test_exact_family_components_prepare_before_both_browser_builds(self):
        workflow = self.load_workflow("nightly.yml")
        jobs = workflow["jobs"]
        components = {
            "prepare-server": (
                "release-server.yml",
                "server_version",
                None,
            ),
            "prepare-claw-server": (
                "release-claw-server.yml",
                "claw_server_version",
                None,
            ),
            "prepare-agent": (
                "release-extensions.yml",
                "agent_version",
                "agent",
            ),
            "prepare-browserclaw": (
                "release-extensions.yml",
                "browserclaw_version",
                "browserclaw",
            ),
        }
        for job_name, (workflow_name, output_name, extension) in components.items():
            job = jobs[job_name]
            with self.subTest(job=job_name):
                self.assertEqual(job["needs"], "transaction")
                self.assertEqual(job["uses"], f"./.github/workflows/{workflow_name}")
                self.assertEqual(job["with"]["mode"], "build")
                self.assertIs(job["with"]["defer_finalize"], True)
                self.assertEqual(job["with"]["state_owner"], "suite")
                self.assertEqual(
                    job["with"]["version"],
                    f"${{{{ needs.transaction.outputs.{output_name} }}}}",
                )
                source_input = "branch" if extension else "ref"
                self.assertEqual(
                    job["with"][source_input],
                    "${{ needs.transaction.outputs.source_sha }}",
                )
                if extension:
                    self.assertEqual(job["with"]["extension"], extension)
                    self.assertIs(job["with"]["publish_alpha_feed"], False)
                else:
                    self.assertIs(job["with"]["publish_ota"], False)

        verifier = jobs["verify-components"]
        self.assertEqual(set(verifier["needs"]), {"transaction", *components.keys()})
        for build_name in ("build-browseros", "build-browserclaw"):
            build = jobs[build_name]
            self.assertEqual(set(build["needs"]), {"transaction", "verify-components"})
            self.assertEqual(
                build["uses"], "./.github/workflows/nightly-macos-product.yml"
            )
            self.assertEqual(
                build["with"]["source_sha"],
                "${{ needs.transaction.outputs.source_sha }}",
            )
            self.assertEqual(
                build["with"]["reservation_sha"],
                "${{ needs.transaction.outputs.reservation_sha }}",
            )
            self.assertNotIn("state_sha", build["with"])
            self.assertEqual(
                build["with"]["state_ref"],
                "${{ needs.transaction.outputs.state_ref }}",
            )
            self.assertEqual(
                build["with"]["browser_version"],
                "${{ needs.transaction.outputs.browser_version }}",
            )

        self.assertEqual(jobs["build-browseros"]["with"]["product"], "browseros")
        self.assertEqual(jobs["build-browserclaw"]["with"]["product"], "browserclaw")
        self.assertEqual(
            jobs["build-browseros"]["with"]["server_version"],
            "${{ needs.transaction.outputs.server_version }}",
        )
        self.assertEqual(
            jobs["build-browserclaw"]["with"]["server_version"],
            "${{ needs.transaction.outputs.claw_server_version }}",
        )
        self.assertEqual(
            jobs["build-browseros"]["with"]["onboarding_version"],
            "${{ needs.transaction.outputs.app_onboarding_version }}",
        )
        self.assertEqual(
            jobs["build-browserclaw"]["with"]["onboarding_version"],
            "${{ needs.transaction.outputs.onboarding_version }}",
        )

    def test_public_finalization_and_state_merge_follow_both_builds(self):
        workflow = self.load_workflow("nightly.yml")
        jobs = workflow["jobs"]
        finalizers = {
            "finalize-server": "release-server.yml",
            "finalize-claw-server": "release-claw-server.yml",
            "finalize-agent": "release-extensions.yml",
            "finalize-browserclaw": "release-extensions.yml",
        }
        for job_name, workflow_name in finalizers.items():
            job = jobs[job_name]
            with self.subTest(job=job_name):
                self.assertEqual(
                    set(job["needs"]),
                    {"transaction", "build-browseros", "build-browserclaw"},
                )
                self.assertEqual(job["uses"], f"./.github/workflows/{workflow_name}")
                self.assertEqual(job["with"]["mode"], "finalize")
                self.assertEqual(job["with"]["state_owner"], "suite")

        for job_name, product in (
            ("server-ota", "browseros"),
            ("claw-server-ota", "browserclaw"),
        ):
            job = jobs[job_name]
            self.assertEqual(job["uses"], "./.github/workflows/publish-server-ota.yml")
            self.assertEqual(job["with"]["product"], product)
            self.assertEqual(job["with"]["state_owner"], "suite")
            self.assertEqual(job["permissions"], {"contents": "read"})

        reconcile = jobs["reconcile-state"]
        self.assertTrue(set(finalizers).issubset(set(reconcile["needs"])))
        self.assertTrue(
            {"server-ota", "claw-server-ota"}.issubset(set(reconcile["needs"]))
        )
        render = self.named_step(
            workflow, "reconcile-state", "Render shared extension feeds"
        )
        merge = self.named_step(
            workflow, "reconcile-state", "Reconcile and merge family state"
        )
        self.assertIn('--set "agent=$AGENT_VERSION"', render["run"])
        self.assertIn('--set "browserclaw=$BROWSERCLAW_VERSION"', render["run"])
        self.assertEqual(render["run"].count("release extensions"), 1)
        self.assertEqual(merge["run"].count("release suite reconcile"), 1)
        self.assertEqual(merge["run"].count("release suite merge"), 1)
        self.assertIn('--state-root "$GITHUB_WORKSPACE"', merge["run"])

        publish = jobs["publish"]
        self.assertEqual(
            set(publish["needs"]),
            {"transaction", "reconcile-state", "build-browseros", "build-browserclaw"},
        )
        self.assertEqual(
            publish["concurrency"],
            {
                "group": "release-feed-snapshots",
                "cancel-in-progress": False,
                "queue": "max",
            },
        )
        self.assertEqual(publish["permissions"]["pull-requests"], "read")
        checkout = next(
            step
            for step in publish["steps"]
            if str(step.get("uses", "")).startswith("actions/checkout@")
        )
        self.assertEqual(
            checkout["with"]["ref"],
            "${{ needs.reconcile-state.outputs.merge_sha }}",
        )
        tracked = self.named_step(
            workflow, "publish", "Publish committed family feeds"
        )["run"]
        for path in (
            "server/appcast-server.alpha.xml",
            "server/appcast-claw-server.alpha.xml",
            "extensions/update-manifest.alpha.xml",
            "extensions/extensions.alpha.json",
            "extensions/bundled-manifest.xml",
        ):
            self.assertIn(path, tracked)
        self.assertNotIn("--allow-downgrade", tracked)
        self.assertNotIn(
            "--allow-downgrade",
            self.named_step(
                workflow, "reconcile-state", "Render shared extension feeds"
            )["run"],
        )
        immutable = self.named_step(
            workflow, "publish", "Publish immutable signed browser artifacts"
        )["run"]
        self.assertEqual(immutable.count("publish-browser-artifact"), 2)
        self.assertIn("--product browseros", immutable)
        self.assertIn("--product browserclaw", immutable)
        recover = self.named_step(
            workflow, "publish", "Recover merged transaction record"
        )
        self.assertEqual(
            recover["env"]["EXPECTED_MERGE_SHA"],
            "${{ needs.reconcile-state.outputs.merge_sha }}",
        )
        for assertion in (
            "jq -r '.state'",
            "jq -r '.merge_sha'",
            'git rev-parse HEAD)" = "$EXPECTED_MERGE_SHA"',
        ):
            self.assertIn(assertion, recover["run"])
        publish_steps = publish["steps"]
        recover_index = publish_steps.index(recover)
        download_indexes = [
            index
            for index, step in enumerate(publish_steps)
            if str(step.get("uses", "")).startswith("actions/download-artifact@")
        ]
        immutable_index = publish_steps.index(
            self.named_step(
                workflow, "publish", "Publish immutable signed browser artifacts"
            )
        )
        feed_index = publish_steps.index(
            self.named_step(workflow, "publish", "Publish committed family feeds")
        )
        rolling_index = publish_steps.index(
            self.named_step(workflow, "publish", "Reconcile rolling nightly releases")
        )
        self.assertEqual(len(download_indexes), 2)
        self.assertTrue(all(recover_index < index for index in download_indexes))
        self.assertTrue(all(index < immutable_index for index in download_indexes))
        self.assertLess(immutable_index, feed_index)
        self.assertLess(feed_index, rolling_index)

    def test_rolling_publication_is_source_and_checksum_aware(self):
        workflow = self.load_workflow("nightly.yml")
        rolling_step = self.named_step(
            workflow, "publish", "Reconcile rolling nightly releases"
        )
        self.assertEqual(rolling_step["working-directory"], "packages/browseros")
        rolling = rolling_step["run"]
        for token in (
            "nightly-browseros",
            "nightly-browserclaw",
            "release suite reconcile-rolling-release",
            '--source-sha "$SOURCE_SHA"',
            '--browser-version "$VERSION"',
            '--repo "$GITHUB_REPOSITORY"',
        ):
            self.assertIn(token, rolling)
        self.assertEqual(rolling.count("reconcile-rolling-release"), 2)
        self.assertNotIn("gh release delete", rolling)
        self.assertNotIn("gh release create", rolling)

    def test_internal_builder_uses_reservation_and_frozen_artifact_source(self):
        workflow = self.load_workflow("nightly-macos-product.yml")
        triggers = workflow.get("on", workflow.get(True))
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
        text = (WORKFLOW_DIR / "nightly-macos-product.yml").read_text(encoding="utf-8")
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
        setup = self.named_step(
            workflow, "build", "Import macOS signing certificate"
        )
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
        self.assertIn("Clean up disposable Chromium workspace", text)
        self.assertIn("Clean up macOS signing keychain", text)
        self.assertIn("actions/upload-artifact@v7", text)
        self.assertIn("release.json", text)
        self.assertNotIn("gh release create", text)

    def test_family_nightly_changes_trigger_build_system_tests(self):
        workflow = self.load_workflow("bos-build-tests.yml")
        triggers = workflow.get("on", workflow.get(True))
        paths = triggers["pull_request"]["paths"]
        self.assertIn(".github/workflows/nightly.yml", paths)
        self.assertIn(".github/workflows/nightly-macos-product.yml", paths)
        self.assertNotIn(".github/workflows/nightly-browseros.yml", paths)
        self.assertNotIn(".github/workflows/nightly-browserclaw.yml", paths)
        self.assertNotIn(".github/workflows/reserve-nightly-browser-version.yml", paths)


@unittest.skipIf(os.name == "nt", "macOS signing helper shell tests run on POSIX")
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
        self._write_fake_uname()
        self._write_fake_stat()
        self._write_fake_git()
        self._write_fake_cp()

    def tearDown(self):
        self.tmp.cleanup()

    def _write_fake_uname(self):
        uname = self.bin_dir / "uname"
        uname.write_text(
            """#!/usr/bin/env bash
set -euo pipefail
printf '%s\\n' "${UNAME_VALUE:-Darwin}"
"""
        )
        uname.chmod(0o755)

    def _write_fake_stat(self):
        stat = self.bin_dir / "stat"
        stat.write_text(
            """#!/usr/bin/env bash
set -euo pipefail
path="${@: -1}"
if [ "${STAT_SPLIT_WORKSPACE_PARENT:-}" = "1" ] && [[ "$path" == *"browseros-ci-apfs-workspaces"* ]]; then
  printf '222\\n'
else
  printf '111\\n'
fi
"""
        )
        stat.chmod(0o755)

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
repo="${repo:-.}"
repo_was_cleaned() {
  [ -f "$GIT_CLEAN_LOG" ] || return 1
  while IFS= read -r cleaned_repo; do
    [ "$cleaned_repo" = "$repo" ] && return 0
  done < "$GIT_CLEAN_LOG"
  return 1
}
case "$cmd" in
  rev-parse)
    target="${1:-}"
    case "$target" in
      HEAD)
        printf '%s\\n' "${GIT_HEAD:-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa}"
        ;;
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
    [ "${CP_FAIL_CLONE:-}" != "1" ] || exit 66
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

    def _workspace_root(self, tag="123-4"):
        return self._workspace_parent() / f"browseros-ci-chromium-{tag}"

    def _state_path(self, tag="123-4"):
        return self.runner_temp / f"browseros-ci-chromium-workspace-{tag}.env"

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
        self.assertIn(
            f"MACOS_CHROMIUM_WORKSPACE_STATE_PATH={state_path}",
            env_lines,
        )
        cp_lines = self.cp_log.read_text(encoding="utf-8").splitlines()
        self.assertTrue(any(line.startswith("-c ") for line in cp_lines))
        self.assertTrue(any(line.startswith("-cR ") for line in cp_lines))
        self.assertFalse(any(line.startswith("-R ") for line in cp_lines))

        cleanup = self._run_helper(
            "cleanup",
            MACOS_CHROMIUM_WORKSPACE_STATE_PATH=str(state_path),
        )
        self.assertEqual(cleanup.returncode, 0, cleanup.stderr + cleanup.stdout)
        self.assertFalse(workspace_root.exists())
        self.assertFalse(state_path.exists())
        self.assertTrue(self.base_src.is_dir())

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
        self.assertFalse(stale.exists())
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
        self.assertFalse(stale.exists())
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

        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertIn("Ignoring unexpected Chromium workspace path", result.stderr)
        self.assertFalse(state_path.exists())
        self.assertTrue(self.base_root.exists())

    def test_setup_rejects_unowned_state_path(self):
        outside_state = self.root / "outside.env"

        result = self._run_helper(
            "setup",
            self.base_src,
            MACOS_CHROMIUM_WORKSPACE_STATE_PATH=str(outside_state),
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn(
            "Unexpected macOS Chromium workspace state path",
            result.stderr + result.stdout,
        )
        self.assertFalse(outside_state.exists())

    def test_setup_fails_when_workspace_parent_is_not_on_base_volume(self):
        result = self._run_helper(
            "setup",
            self.base_src,
            STAT_SPLIT_WORKSPACE_PARENT="1",
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn(
            "workspace parent is not on the base checkout volume",
            result.stderr + result.stdout,
        )
        self.assertFalse(self._workspace_root().exists())

    def test_setup_fails_without_full_copy_fallback_when_apfs_clone_fails(self):
        result = self._run_helper("setup", self.base_src, CP_FAIL_CLONE="1")

        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self._workspace_root().exists())
        self.assertFalse(self._state_path().exists())
        cp_lines = self.cp_log.read_text(encoding="utf-8").splitlines()
        self.assertTrue(any(line.startswith("-cR ") for line in cp_lines))
        self.assertFalse(any(line.startswith("-R ") for line in cp_lines))

    def test_setup_fails_when_base_is_not_at_pinned_chromium_tag(self):
        result = self._run_helper("setup", self.base_src, GIT_PIN_HEAD="b" * 40)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("does not match pinned", result.stderr + result.stdout)
        self.assertFalse(self._workspace_root().exists())

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

        result = self._run_helper(
            "setup",
            self.base_src,
            GIT_DIRTY_REPO=str(nested_repo.resolve()),
            GIT_DIRTY_STATUS=" M src/builtins/generated.cc\n",
        )

        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertTrue(self._workspace_root().exists())

    def test_setup_removes_browseros_output_dirs_from_base(self):
        out_dir = self.base_src / "out" / "Default_browseros_arm64"
        out_dir.mkdir(parents=True)

        result = self._run_helper("setup", self.base_src)

        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertFalse(out_dir.exists())
        self.assertTrue(self._workspace_root().exists())


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

    def test_nightly_runbook_covers_family_transaction_and_retry_identities(self):
        for token in (
            ".github/workflows/nightly.yml",
            "nightly-<source-sha>",
            "Source SHA",
            "Reservation SHA",
            "State SHA",
            "Merge SHA",
            "refs/pull/<PR_NUMBER>/head",
            "created as a draft",
            "Open, closed, and\nmerged canonical suite records",
            "superseded/no-op",
            "--allow-downgrade",
            "Re-run failed\njobs",
            "conditionally creates",
            "existing alias is older or missing",
            "before exposing any mutable feeds",
            "release record and its live tag must resolve to the same source",
            "partial draft resumes",
            "Rewriting `main` history is unsupported",
            "audit those effects and explicitly remove",
        ):
            with self.subTest(token=token):
                self.assertIn(token, self.nightly)
        self.assertLess(
            self.nightly.index(
                "conditionally create or verify both immutable signed browser artifacts"
            ),
            self.nightly.index(
                "publish the committed feeds and reconcile both rolling prereleases"
            ),
        )
        self.assertIn("first production slice migrates the nightly", self.release)
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


if __name__ == "__main__":
    unittest.main()
