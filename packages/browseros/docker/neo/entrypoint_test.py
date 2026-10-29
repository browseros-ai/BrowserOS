"""Run entrypoint.sh functions under bash, with the container's commands stubbed."""

import json
import re
import shlex
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = (Path(__file__).parent / "entrypoint.sh").read_text()


def shell_function(name: str) -> str:
    """The definition of one top-level function in entrypoint.sh."""
    match = re.search(rf"^{name}\(\) {{\n.*?^}}\n", SCRIPT, re.MULTILINE | re.DOTALL)
    assert match, f"entrypoint.sh has no function {name}"
    return match.group(0)


def run_bash(script: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["bash", "-c", "set -euo pipefail\n" + script],
        capture_output=True, text=True, check=True,
    )


class StartNeoTest(unittest.TestCase):
    def launch(self, neo_args: str) -> tuple[list[str], list[str]]:
        """start_neo's calls in order, and the browser's argv, with the image's defaults."""
        with tempfile.TemporaryDirectory() as directory:
            calls, argv = Path(directory) / "calls", Path(directory) / "argv"
            run_bash(
                f"forget_zoom_levels() {{ echo forget_zoom_levels >> '{calls}'; }}\n"
                f"spawn_as_neo() {{ echo spawn_as_neo >> '{calls}'; "
                f"printf '%s\\n' \"$@\" > '{argv}' & }}\n"
                "sandbox_flags=(--no-sandbox)\n"
                "browseros_extension_flags=(--disable-browseros-extensions)\n"
                "gpu_flags=(--ignore-gpu-blocklist --disable-gpu-compositing)\n"
                "PROFILE=/data/profile CDP_PORT=9110 SERVER_PORT=9210\n"
                "HIDDEN_BROWSEROS_FEATURES=NewTabPage\n"
                f"NEO_ARGS='{neo_args}'\n"
                + shell_function("start_neo")
                + "start_neo\nwait\n"
            )
            return calls.read_text().split(), argv.read_text().splitlines()

    def test_neo_starts_with_zoom_off(self):
        _, argv = self.launch("")

        self.assertEqual(argv[0], "browserclaw")
        self.assertIn("--disable-pinch", argv)

    def test_neo_args_still_come_last(self):
        _, argv = self.launch("--lang=de --disable-browseros-features=")

        self.assertEqual(argv[-2:], ["--lang=de", "--disable-browseros-features="])
        self.assertLess(argv.index("--disable-pinch"), len(argv) - 2)

    def test_every_launch_first_drops_saved_zoom_levels(self):
        # The supervisor loop relaunches Neo through start_neo after it closes.
        calls, _ = self.launch("")

        self.assertEqual(calls, ["forget_zoom_levels", "spawn_as_neo"])


@unittest.skipUnless(shutil.which("jq"), "needs jq, as the image has")
class ApplyPoliciesTest(unittest.TestCase):
    def apply(
        self,
        hosted_mode: str,
        stale_hosted_policy: bool = False,
        downloads: str = "/data/downloads",
    ) -> dict[str, dict]:
        """The managed policy files apply_policies leaves, by name, for one mode."""
        managed = Path(self.enterContext(tempfile.TemporaryDirectory()))
        if stale_hosted_policy:
            (managed / "neo-hosted.json").write_text('{"PasswordManagerEnabled": false}')
        run_bash(
            "sandbox_flags=(--no-sandbox)\n"
            f"BROWSERCLAW_HOSTED_MODE={hosted_mode}\n"
            f"DOWNLOADS={shlex.quote(downloads)}\n"
            + shell_function("apply_policies").replace("/etc/chromium/policies/managed", str(managed))
            + "apply_policies\n"
        )
        return {path.name: json.loads(path.read_text()) for path in managed.iterdir()}

    def test_hosted_mode_turns_off_password_and_address_prompts(self):
        policies = self.apply("1")

        self.assertEqual(
            policies["neo-hosted.json"],
            {
                "PasswordManagerEnabled": False,
                # Chromium checks a submitted login for leaks even with saving off.
                "PasswordLeakDetectionEnabled": False,
                "AutofillAddressEnabled": False,
            },
        )

    def test_desktop_mode_keeps_saving_prompts(self):
        # The policy directory outlives `docker restart`, so a file written in hosted mode is
        # dropped once the container is recreated with hosted mode off.
        policies = self.apply("0", stale_hosted_policy=True)

        self.assertNotIn("neo-hosted.json", policies)

    def test_card_saving_stays_off_in_both_modes(self):
        for mode in ("0", "1"):
            self.assertEqual(
                self.apply(mode)["neo-payments.json"], {"AutofillCreditCardEnabled": False}
            )

    def test_downloads_use_the_shared_folder_without_save_as_prompts(self):
        for mode in ("0", "1"):
            directory = '/workspace/shared files/"downloads"'
            policies = self.apply(mode, downloads=directory)

            self.assertEqual(
                policies["neo-downloads.json"],
                {"DownloadDirectory": directory, "PromptForDownloadLocation": False},
            )


@unittest.skipUnless(shutil.which("jq"), "needs jq, as the image has")
class ForgetZoomLevelsTest(unittest.TestCase):
    def forget(self, preferences: str | None) -> tuple[subprocess.CompletedProcess, Path]:
        directory = Path(self.enterContext(tempfile.TemporaryDirectory()))
        prefs = directory / "Default/Preferences"
        if preferences is not None:
            prefs.parent.mkdir()
            prefs.write_text(preferences)
        result = run_bash(
            # Outside the container there is no neo user to give files to.
            "chown() { :; }\n"
            "log() { printf '[neo] %s\\n' \"$*\"; }\n"
            f"PROFILE='{directory}'\n"
            + shell_function("merge_json")
            + shell_function("forget_zoom_levels")
            + "forget_zoom_levels\n"
        )
        return result, prefs

    def test_drops_only_zoom_levels(self):
        saved = {
            "partition": {
                "per_host_zoom_levels": {"x": {"example.com": {"zoom_level": 1.22}}},
                "default_zoom_level": {"x": 1.22},
                "other": True,
            },
            "browser": {"window_placement": {"maximized": True}},
        }

        _, prefs = self.forget(json.dumps(saved))

        self.assertEqual(
            json.loads(prefs.read_text()),
            {"partition": {"other": True}, "browser": {"window_placement": {"maximized": True}}},
        )

    def test_a_new_profile_gets_no_preferences_file(self):
        _, prefs = self.forget(None)

        self.assertFalse(prefs.exists())

    def test_unreadable_preferences_are_kept_and_startup_continues(self):
        result, prefs = self.forget("{not json")

        self.assertEqual(prefs.read_text(), "{not json")
        self.assertIn("Could not clear saved zoom levels", result.stdout)


if __name__ == "__main__":
    unittest.main()
