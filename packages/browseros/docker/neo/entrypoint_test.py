"""Run entrypoint.sh functions under bash, with the container's commands stubbed."""

import json
import re
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
        self.assertIn("--browseros-disable-zoom", argv)

    def test_neo_args_still_come_last(self):
        _, argv = self.launch("--lang=de --disable-browseros-features=")

        self.assertEqual(argv[-2:], ["--lang=de", "--disable-browseros-features="])
        self.assertLess(argv.index("--browseros-disable-zoom"), len(argv) - 2)

    def test_every_launch_first_drops_saved_zoom_levels(self):
        # The supervisor loop relaunches Neo through start_neo after it closes.
        calls, _ = self.launch("")

        self.assertEqual(calls, ["forget_zoom_levels", "spawn_as_neo"])


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
