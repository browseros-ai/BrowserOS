"""Run entrypoint.sh functions under bash, with the container's commands stubbed."""

import json
import os
import re
import shlex
import shutil
import socket
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from typing import NamedTuple

HERE = Path(__file__).parent
SCRIPT = (HERE / "entrypoint.sh").read_text()
HEALTHCHECK = (HERE / "healthcheck.sh").read_text()
# The image's /bin/sh is dash.
SH = shutil.which("dash") or "sh"

# start_display's processes before claw-sdk images existed. claw-server images keep them.
TODAY_XVNC = [
    "Xvnc", ":1", "-geometry", "1600x1000", "-depth", "24", "-SecurityTypes", "None",
    "-localhost", "-rfbport", "5900", "-AlwaysShared", "-nolisten", "tcp",
]
VNCCONFIG = ["vncconfig", "-nowin"]
OPENBOX = ["openbox", "--config-file", "/etc/neo/openbox-rc.xml"]
TODAY_WEBSOCKIFY = ["websockify", "--web", "/usr/share/novnc", "6080", "localhost:5900"]
CLIPBOARD_OFF = ["-SendCutText=0", "-SendPrimary=0"]


def shell_function(name: str) -> str:
    """The definition of one top-level function in entrypoint.sh."""
    match = re.search(rf"^{name}\(\) {{\n.*?^}}\n", SCRIPT, re.MULTILINE | re.DOTALL)
    assert match, f"entrypoint.sh has no function {name}"
    return match.group(0)


def run_bash(script: str, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["bash", "-c", "set -euo pipefail\n" + script],
        capture_output=True, text=True, check=check,
    )


def identified_by(identify: str) -> str:
    """Bash that runs server_policy with `identify` standing in for `neo-server-info kind`."""
    return (
        f'neo-server-info() {{ [ "$1" = kind ] && {identify}; }}\n'
        "log() { printf '[neo] %s\\n' \"$*\"; }\n"
        'die() { log "$*" >&2; exit 1; }\n'
        + shell_function("server_policy")
        + "server_policy\n"
    )


def server_is(kind: str) -> str:
    return identified_by(f"echo {kind}")


def start_neo_spawning(spawn: str) -> str:
    """Bash that defines start_neo with the image's launch inputs and zoom step stubbed, its
    spawn_as_neo running `spawn` and then leaving a pid, as the real spawn does. PROFILE and
    BROWSERCLAW_HOSTED_MODE come from the caller."""
    return (
        "forget_zoom_levels() { :; }\n"
        f"spawn_as_neo() {{\n{spawn}\n  : &\n}}\n"
        "BACK_FORWARD_CACHE=0 sandbox_flags=(--no-sandbox)\n"
        "browseros_extension_flags=(--disable-browseros-extensions)\n"
        "gpu_flags=(--disable-gpu-compositing)\n"
        "CDP_PORT=9110 SERVER_PORT=9210 HIDDEN_BROWSEROS_FEATURES=NewTabPage NEO_ARGS=\n"
        + shell_function("start_neo")
    )


class StartNeoTest(unittest.TestCase):
    def launch(
        self, neo_args: str, kind: str = "claw-server", hosted_mode: str = "1",
        start_count: int = 1,
    ) -> tuple[list[str], list[str]]:
        """Capture launch calls and final argv without letting restarts mutate caller flags."""
        with tempfile.TemporaryDirectory() as directory:
            calls, argv = Path(directory) / "calls", Path(directory) / "argv"
            run_bash(
                server_is(kind)
                + f"forget_zoom_levels() {{ echo forget_zoom_levels >> '{calls}'; }}\n"
                f"clear_sessions() {{ echo clear_sessions >> '{calls}'; }}\n"
                f"spawn_as_neo() {{ echo spawn_as_neo >> '{calls}'; "
                f"printf '%s\\n' \"$@\" > '{argv}' & }}\n"
                "sandbox_flags=(--no-sandbox)\n"
                "browseros_extension_flags=(--disable-browseros-extensions)\n"
                "gpu_flags=(--ignore-gpu-blocklist --disable-gpu-compositing)\n"
                "PROFILE=/data/profile CDP_PORT=9110 SERVER_PORT=9210\n"
                "HIDDEN_BROWSEROS_FEATURES=NewTabPage\n"
                f"BROWSERCLAW_HOSTED_MODE={hosted_mode}\n"
                f"readonly NEO_ARGS={shlex.quote(neo_args)}\n"
                + shell_function("start_neo")
                + "start_neo\nwait\n" * start_count
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

    def test_neo_args_keep_the_existing_whitespace_split(self):
        for mode in ("0", "1"):
            with self.subTest(mode=mode):
                _, argv = self.launch('--lang="de DE"\t\n--disable-gpu', hosted_mode=mode)

                self.assertEqual(argv[-3:], ['--lang="de', 'DE"', '--disable-gpu'])

    def test_every_launch_first_drops_saved_zoom_levels_and_saved_sessions(self):
        # The supervisor loop relaunches Neo through start_neo after it closes.
        calls, _ = self.launch("")

        self.assertEqual(calls, ["forget_zoom_levels", "clear_sessions", "spawn_as_neo"])

    def test_claw_sdk_turns_the_back_forward_cache_off_as_playwright_does(self):
        # A page restored from the cache fires no load event, so Playwright's goBack() would hang.
        _, sdk = self.launch("--lang=de", kind="claw-sdk")
        _, server = self.launch("--lang=de")

        self.assertIn("--disable-back-forward-cache", sdk)
        self.assertNotIn("--disable-back-forward-cache", server)
        self.assertEqual(sdk[-1], "--lang=de")

    def test_hosted_mode_turns_the_browsers_own_metrics_off(self):
        # Hosted is the image's default, and this server is the one the image ships.
        _, argv = self.launch("", kind="claw-sdk")

        self.assertEqual(argv.count("--disable-browseros-metrics"), 1)
        self.assertLess(argv.index("--disable-browseros-metrics"), argv.index("--disable-pinch"))

    def test_outside_hosted_mode_the_browser_keeps_its_metrics(self):
        for kind in ("claw-sdk", "claw-server"):
            with self.subTest(kind=kind):
                _, argv = self.launch("--lang=de", kind=kind, hosted_mode="0")

                self.assertNotIn("--disable-browseros-metrics", argv)
                self.assertEqual(argv[-1], "--lang=de")

    def test_the_switch_repeated_through_neo_args_still_comes_last(self):
        _, argv = self.launch("--disable-browseros-metrics --lang=de", kind="claw-sdk")

        self.assertEqual(argv[-2:], ["--disable-browseros-metrics", "--lang=de"])
        self.assertEqual(argv.count("--disable-browseros-metrics"), 2)

    def test_hosted_mode_enables_fresh_surface_captures(self):
        for kind in ("claw-sdk", "claw-server"):
            with self.subTest(kind=kind):
                _, argv = self.launch("", kind=kind)

                self.assertEqual(
                    [arg for arg in argv if arg.startswith("--enable-features=")],
                    ["--enable-features=CDPScreenshotNewSurface"],
                )

    def test_hosted_mode_merges_only_the_last_feature_list_in_place(self):
        for supplied, expected in [
            (
                "--lang=de --enable-features=First,Second --disable-gpu",
                ["--lang=de", "--enable-features=First,Second,CDPScreenshotNewSurface",
                 "--disable-gpu"],
            ),
            (
                "--enable-features=",
                ["--enable-features=CDPScreenshotNewSurface"],
            ),
            (
                "--enable-features",
                ["--enable-features=CDPScreenshotNewSurface"],
            ),
            (
                "--enable-features=First --lang=de --enable-features=Second --disable-gpu",
                ["--enable-features=First", "--lang=de",
                 "--enable-features=Second,CDPScreenshotNewSurface", "--disable-gpu"],
            ),
            (
                "--enable-features=CDPScreenshotNewSurface --enable-features=",
                ["--enable-features=CDPScreenshotNewSurface",
                 "--enable-features=CDPScreenshotNewSurface"],
            ),
            (
                "--enable-features=First --lang=de --enable-features --disable-gpu",
                ["--enable-features=First", "--lang=de",
                 "--enable-features=CDPScreenshotNewSurface", "--disable-gpu"],
            ),
            (
                "--enable-features --lang=de --enable-features=Second",
                ["--enable-features", "--lang=de",
                 "--enable-features=Second,CDPScreenshotNewSurface"],
            ),
        ]:
            with self.subTest(supplied=supplied):
                _, argv = self.launch(supplied, kind="claw-sdk")

                self.assertEqual(argv[-len(expected):], expected)
                self.assertEqual(
                    [arg for arg in argv if arg.partition("=")[0] == "--enable-features"],
                    [arg for arg in expected if arg.partition("=")[0] == "--enable-features"],
                )

    def test_hosted_mode_preserves_existing_capture_feature_forms(self):
        for features in (
            "CDPScreenshotNewSurface",
            "First,CDPScreenshotNewSurface,Last",
            "CDPScreenshotNewSurface<Study",
            "First,CDPScreenshotNewSurface<Study,Last",
            "CDPScreenshotNewSurface:parameter/value",
            "First,CDPScreenshotNewSurface<Study:parameter/value,Last",
        ):
            with self.subTest(features=features):
                supplied = f"--enable-features={features}"
                _, argv = self.launch(supplied, kind="claw-sdk")

                self.assertEqual(
                    [arg for arg in argv if arg.startswith("--enable-features=")],
                    [supplied],
                )

    def test_hosted_mode_stops_feature_switches_at_the_terminator(self):
        for supplied, expected, prepended in [
            (
                "--enable-features=First -- --enable-features=Second",
                ["--enable-features=First,CDPScreenshotNewSurface", "--",
                 "--enable-features=Second"],
                False,
            ),
            (
                "--enable-features -- -enable-features=Second --enable-features",
                ["--enable-features=CDPScreenshotNewSurface", "--",
                 "-enable-features=Second", "--enable-features"],
                False,
            ),
            (
                "--lang=de -- --enable-features=Second -enable-features "
                "--enable-features=*CDPScreenshotNewSurface",
                ["--lang=de", "--", "--enable-features=Second", "-enable-features",
                 "--enable-features=*CDPScreenshotNewSurface"],
                True,
            ),
        ]:
            with self.subTest(supplied=supplied):
                _, argv = self.launch(supplied, kind="claw-sdk")

                self.assertEqual(argv[-len(expected):], expected)
                self.assertEqual(
                    argv[1] == "--enable-features=CDPScreenshotNewSurface", prepended
                )

    def test_hosted_mode_merges_the_last_feature_switch_with_either_prefix(self):
        for supplied, expected in [
            (
                "-enable-features=First --lang=de",
                ["-enable-features=First,CDPScreenshotNewSurface", "--lang=de"],
            ),
            (
                "-enable-features=",
                ["-enable-features=CDPScreenshotNewSurface"],
            ),
            (
                "-enable-features",
                ["-enable-features=CDPScreenshotNewSurface"],
            ),
            (
                "--enable-features=First -enable-features=Second --lang=de",
                ["--enable-features=First", "-enable-features=Second,CDPScreenshotNewSurface",
                 "--lang=de"],
            ),
            (
                "-enable-features=First --enable-features=Second",
                ["-enable-features=First", "--enable-features=Second,CDPScreenshotNewSurface"],
            ),
            (
                "--enable-features=First -enable-features",
                ["--enable-features=First", "-enable-features=CDPScreenshotNewSurface"],
            ),
        ]:
            with self.subTest(supplied=supplied):
                _, argv = self.launch(supplied, kind="claw-sdk")

                self.assertEqual(argv[-len(expected):], expected)
                self.assertEqual(
                    [arg for arg in argv
                     if arg.partition("=")[0] in ("--enable-features", "-enable-features")],
                    [arg for arg in expected
                     if arg.partition("=")[0] in ("--enable-features", "-enable-features")],
                )

    def test_hosted_mode_promotes_default_capture_entries_without_changing_the_list(self):
        for features, expected in [
            ("*CDPScreenshotNewSurface", "CDPScreenshotNewSurface"),
            ("First,*CDPScreenshotNewSurface,Last", "First,CDPScreenshotNewSurface,Last"),
            ("*CDPScreenshotNewSurface<Study", "CDPScreenshotNewSurface<Study"),
            ("*CDPScreenshotNewSurface:parameter/value", "CDPScreenshotNewSurface:parameter/value"),
            (
                "First,,*CDPScreenshotNewSurface<Study:parameter/value,Last,",
                "First,,CDPScreenshotNewSurface<Study:parameter/value,Last,",
            ),
            (
                "*CDPScreenshotNewSurface<FirstStudy,CDPScreenshotNewSurface<LastStudy",
                "CDPScreenshotNewSurface<FirstStudy,CDPScreenshotNewSurface<LastStudy",
            ),
            (
                "CDPScreenshotNewSurface<FirstStudy,*CDPScreenshotNewSurface<LastStudy",
                "CDPScreenshotNewSurface<FirstStudy,CDPScreenshotNewSurface<LastStudy",
            ),
            (
                "*CDPScreenshotNewSurface,*CDPScreenshotNewSurface",
                "CDPScreenshotNewSurface,CDPScreenshotNewSurface",
            ),
            (
                "Other:parameter/*CDPScreenshotNewSurface,"
                "*CDPScreenshotNewSurface:parameter/*CDPScreenshotNewSurface",
                "Other:parameter/*CDPScreenshotNewSurface,"
                "CDPScreenshotNewSurface:parameter/*CDPScreenshotNewSurface",
            ),
        ]:
            for prefix in ("--", "-"):
                with self.subTest(features=features, prefix=prefix):
                    _, argv = self.launch(f"{prefix}enable-features={features}", kind="claw-sdk")

                    self.assertEqual(argv[-1], f"{prefix}enable-features={expected}")

    def test_hosted_mode_matches_the_whole_capture_feature_name(self):
        for features in (
            "OtherCDPScreenshotNewSurface,CDPScreenshotNewSurfaceOther",
            "Other<CDPScreenshotNewSurface",
            "Other:parameter/CDPScreenshotNewSurface",
            "*OtherCDPScreenshotNewSurface,*CDPScreenshotNewSurfaceOther",
            "Other<*CDPScreenshotNewSurface",
            "Other:parameter/*CDPScreenshotNewSurface",
        ):
            with self.subTest(features=features):
                _, argv = self.launch(f"--enable-features={features}", kind="claw-sdk")

                self.assertEqual(
                    argv[-1], f"--enable-features={features},CDPScreenshotNewSurface"
                )

    def test_outside_hosted_mode_feature_lists_are_unchanged(self):
        for supplied in (
            "",
            "--lang=de",
            "--enable-features=",
            "--enable-features",
            "--enable-features=First --lang=de --enable-features=Second",
            "--enable-features=First --lang=de --enable-features",
            "-enable-features",
            "--enable-features=First -enable-features=*CDPScreenshotNewSurface<Study:parameter/value",
            "-enable-features=*CDPScreenshotNewSurface -- --enable-features=Second",
            "-- --enable-features=*CDPScreenshotNewSurface -enable-features",
        ):
            with self.subTest(supplied=supplied):
                _, argv = self.launch(supplied, kind="claw-sdk", hosted_mode="0")
                expected = supplied.split()

                self.assertEqual(
                    [arg for arg in argv if arg.partition("=")[0] == "--enable-features"],
                    [arg for arg in expected if arg.partition("=")[0] == "--enable-features"],
                )
                if expected:
                    self.assertEqual(argv[-len(expected):], expected)

    def test_restarting_does_not_duplicate_or_mutate_caller_features(self):
        for supplied, expected in [
            (
                "--enable-features=First --lang=de",
                ["--enable-features=First,CDPScreenshotNewSurface", "--lang=de"],
            ),
            (
                "--enable-features=First -enable-features=*CDPScreenshotNewSurface<Study:parameter/value "
                "-- --enable-features=Second",
                ["--enable-features=First", "-enable-features=CDPScreenshotNewSurface<Study:parameter/value",
                 "--", "--enable-features=Second"],
            ),
            (
                "-- --enable-features=*CDPScreenshotNewSurface",
                ["--", "--enable-features=*CDPScreenshotNewSurface"],
            ),
        ]:
            with self.subTest(supplied=supplied):
                calls, argv = self.launch(supplied, kind="claw-sdk", start_count=2)
                expected_switches = [
                    arg for arg in expected
                    if arg.partition("=")[0] in ("--enable-features", "-enable-features")
                ]
                if supplied.startswith("-- --"):
                    expected_switches.insert(0, "--enable-features=CDPScreenshotNewSurface")

                self.assertEqual(calls, ["forget_zoom_levels", "clear_sessions", "spawn_as_neo"] * 2)
                self.assertEqual(argv[-len(expected):], expected)
                self.assertEqual(
                    [arg for arg in argv
                     if arg.partition("=")[0] in ("--enable-features", "-enable-features")],
                    expected_switches,
                )


class OutputDirectoryTest(unittest.TestCase):
    def test_output_directory_follows_data_dir_unless_overridden(self):
        declaration = re.search(r"^readonly OUTPUTS=.*$", SCRIPT, re.MULTILINE)
        self.assertIsNotNone(declaration)
        for override, expected in [
            ("", "/var/lib/host/neo/tool-output"),
            ("/shared/output files", "/shared/output files"),
        ]:
            result = run_bash(
                "NEO_DATA_DIR=/var/lib/host/neo\n"
                f"NEO_OUTPUT_DIR={shlex.quote(override)}\n"
                + declaration.group(0) + '\nprintf "%s" "$OUTPUTS"\n'
            )
            self.assertEqual(result.stdout, expected)

    def test_child_process_receives_the_configured_private_output_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            argv = Path(directory) / "argv"
            stub = Path(directory) / "setpriv"
            stub.write_text(
                "#!/bin/bash\n"
                f"printf '%s\\n' \"$@\" > {shlex.quote(str(argv))}\n"
            )
            stub.chmod(0o755)
            output = "/workspace/neo data/tool-output"
            run_bash(
                f"PATH={shlex.quote(directory)}:$PATH\n"
                "NEO_HOME=/home/neo XDG_DIR=/tmp/neo-xdg CLAW_DATA=/data/browserclaw\n"
                "PROFILE=/data/profile DOWNLOADS=/data/downloads\n"
                "BROWSERCLAW_HOSTED_MODE=1\n"
                f"OUTPUTS={shlex.quote(output)}\n"
                + shell_function("spawn_as_neo")
                + "spawn_as_neo browserclaw\nwait\n"
            )
            received = argv.read_text().splitlines()
            self.assertIn(f"BROWSEROS_OUTPUT_DIR={output}", received)
            self.assertIn("CLAW_PROFILE_DIR=/data/profile", received)
            self.assertIn("CLAW_DOWNLOADS_DIR=/data/downloads", received)


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


class ClearSessionsTest(unittest.TestCase):
    """NEO_CLEAR_SESSIONS: the browser's saved sessions, removed before each of its starts."""

    def profile(self) -> Path:
        """A profile whose saved sessions hold a typed value, beside what must stay."""
        directory = Path(self.enterContext(tempfile.TemporaryDirectory()))
        profile = directory / "data/profile"
        sessions = profile / "Default/Sessions"
        sessions.mkdir(parents=True)
        (sessions / "Session_1").write_text("4111111111111111")
        (sessions / "Tabs_1").write_text("4111111111111111")
        for kept in ["Default/Cookies", "Default/Login Data", "Default/Preferences", "Local State"]:
            (profile / kept).write_text("kept")
        return profile

    def processes(self, *argvs: list[str]) -> Path:
        """A /proc with one process per argv, each as its NUL-separated cmdline."""
        proc = Path(self.enterContext(tempfile.TemporaryDirectory()))
        for pid, argv in enumerate(argvs, start=100):
            (proc / str(pid)).mkdir()
            (proc / str(pid) / "cmdline").write_bytes(b"\0".join(a.encode() for a in argv) + b"\0")
        return proc

    def setting(self, value: str | None, hosted_mode: str) -> str:
        """Bash that reads NEO_CLEAR_SESSIONS into CLEAR_SESSIONS as the entrypoint does."""
        default = re.search(r'^: "\$\{NEO_CLEAR_SESSIONS:=false\}"$', SCRIPT, re.MULTILINE)
        parse = re.search(
            r"^CLEAR_SESSIONS=0\n.*?^readonly CLEAR_SESSIONS\n", SCRIPT, re.MULTILINE | re.DOTALL
        )
        assert default and parse, "entrypoint.sh no longer reads NEO_CLEAR_SESSIONS here"
        given = "" if value is None else f"NEO_CLEAR_SESSIONS={shlex.quote(value)}\n"
        return (
            "log() { printf '[neo] %s\\n' \"$*\"; }\n"
            'die() { log "$*" >&2; exit 1; }\n'
            + shell_function("is_true")
            + f"BROWSERCLAW_HOSTED_MODE={hosted_mode}\n{given}{default.group(0)}\n{parse.group(0)}"
        )

    def run_clear(
        self,
        profile: Path,
        value: str | None,
        hosted_mode: str = "1",
        proc: Path | None = None,
        gone_tenths: int = 20,
        then: str = "clear_sessions\n",
    ) -> subprocess.CompletedProcess:
        """`then` after the setting, profile_in_use and clear_sessions, on `proc` as /proc."""
        proc = proc or self.processes()
        return run_bash(
            self.setting(value, hosted_mode)
            + f"PROFILE={shlex.quote(str(profile))} PROC_DIR={shlex.quote(str(proc))}\n"
            + f"BROWSER_GONE_TENTHS={gone_tenths}\n"
            + shell_function("profile_in_use")
            + shell_function("clear_sessions")
            + then,
            check=False,
        )

    def assert_only_sessions_gone(self, profile: Path) -> None:
        self.assertFalse((profile / "Default/Sessions").exists())
        for kept in ["Default/Cookies", "Default/Login Data", "Default/Preferences", "Local State"]:
            self.assertEqual((profile / kept).read_text(), "kept", kept)

    def browser(self, profile: Path) -> list[str]:
        return ["/usr/lib/browserclaw/browserclaw", "--type=renderer", f"--user-data-dir={profile}"]

    def test_unset_keeps_the_saved_sessions_as_today(self):
        profile = self.profile()

        result = self.run_clear(profile, None)

        self.assertEqual((result.returncode, result.stdout), (0, ""))
        self.assertTrue((profile / "Default/Sessions/Session_1").exists())

    def test_off_keeps_them(self):
        for value in ["false", "0", "OFF"]:
            with self.subTest(value=value):
                profile = self.profile()

                self.run_clear(profile, value)

                self.assertTrue((profile / "Default/Sessions/Tabs_1").exists())

    def test_on_removes_the_sessions_folder_and_nothing_else(self):
        for value in ["1", "true", "ON"]:
            with self.subTest(value=value):
                profile = self.profile()

                result = self.run_clear(profile, value)

                self.assertEqual(result.returncode, 0, result.stderr)
                self.assert_only_sessions_gone(profile)
                self.assertIn("Removed the browser's saved sessions", result.stdout)

    def test_a_value_that_is_not_a_boolean_stops_the_container(self):
        profile = self.profile()

        result = self.run_clear(profile, "sometimes")

        self.assertEqual(result.returncode, 1)
        self.assertIn("expected true or false, got 'sometimes'", result.stderr)
        self.assertTrue((profile / "Default/Sessions/Session_1").exists())

    def test_outside_hosted_mode_it_does_nothing_and_says_so(self):
        profile = self.profile()

        result = self.run_clear(profile, "1", hosted_mode="0")

        self.assertTrue((profile / "Default/Sessions/Session_1").exists())
        self.assertIn("applies in hosted mode only", result.stdout)

    def test_a_profile_without_the_folder_starts_quietly(self):
        profile = self.profile()
        shutil.rmtree(profile / "Default/Sessions")

        result = self.run_clear(profile, "1")

        self.assertEqual((result.returncode, result.stdout), (0, ""))
        self.assertFalse((profile / "Default/Sessions").exists())

    def test_every_start_of_the_browser_removes_what_the_last_one_saved(self):
        # The first start and a restart after the browser exited both go through start_neo; each
        # browser writes its own Session_ file while it runs.
        profile = self.profile()
        sessions, seen = shlex.quote(f"{profile}/Default/Sessions"), shlex.quote(f"{profile.parent}/seen")
        # Each browser notes what is in Sessions as it starts, then saves a file of its own.
        browser = (
            f"  {{ ls {sessions} 2>/dev/null || true; }} | tr '\\n' ' ' >> {seen}; echo >> {seen}\n"
            f"  mkdir -p {sessions}; echo 4111111111111111 > {sessions}/Session_2"
        )
        result = self.run_clear(profile, "1", then=start_neo_spawning(browser) + "start_neo\nstart_neo\n")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            (profile.parent / "seen").read_text().splitlines(), ["", ""], "each started without them"
        )
        self.assertEqual(result.stdout.count("Removed the browser's saved sessions"), 2)

    def test_it_waits_for_the_last_browser_process_to_go(self):
        profile = self.profile()
        proc = self.processes(self.browser(profile))

        result = self.run_clear(
            profile, "1", proc=proc, gone_tenths=50,
            then=f"(sleep 0.3; rm -rf {shlex.quote(str(proc))}/100) &\nclear_sessions\nwait\n",
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_only_sessions_gone(profile)

    def test_a_browser_that_does_not_go_fails_the_call_and_keeps_them(self):
        profile = self.profile()

        result = self.run_clear(
            profile, "1", proc=self.processes(self.browser(profile)), gone_tenths=2
        )

        self.assertEqual(result.returncode, 1)
        self.assertTrue((profile / "Default/Sessions/Session_1").exists())
        self.assertIn("A browser still runs on", result.stdout)

    @unittest.skipIf(os.geteuid() == 0, "root removes from a read-only folder")
    def test_a_folder_that_cannot_be_removed_fails_the_call(self):
        profile = self.profile()
        (profile / "Default").chmod(0o555)
        self.addCleanup((profile / "Default").chmod, 0o755)

        result = self.run_clear(profile, "1")

        self.assertEqual(result.returncode, 1)
        self.assertTrue((profile / "Default/Sessions").exists())
        self.assertIn("Could not remove", result.stdout)

    def test_no_browser_starts_when_the_saved_sessions_could_not_go(self):
        # The supervisor stops the container on that failure, at the first start and at a restart.
        directory = Path(self.enterContext(tempfile.TemporaryDirectory()))
        spawned = directory / "spawned"
        result = run_bash(
            "clear_sessions() { return 1; }\nBROWSERCLAW_HOSTED_MODE=1 PROFILE=/data/profile\n"
            + start_neo_spawning(f"  touch {shlex.quote(str(spawned))}")
            + "if start_neo; then echo started; else echo refused; fi\n"
        )

        self.assertEqual(result.stdout.strip(), "refused")
        self.assertFalse(spawned.exists())

    def test_the_supervisor_stops_the_container_when_neo_cannot_start(self):
        # The supervisor calls start_neo twice, at the first start and at each restart, and both
        # calls stop the container when it fails.
        after_definition = SCRIPT.split("\nstart_neo() {", 1)[1]
        calls = [
            line.strip()
            for line in after_definition.splitlines()
            if re.search(r"\bstart_neo\b", line) and not line.lstrip().startswith("#")
        ]

        self.assertEqual(
            calls,
            [
                'start_neo || { log "Could not start Neo; stopping the container"; shutdown 1; }',
                'if [ "$stopping" = 0 ] && ! start_neo; then',
            ],
        )
        self.assertIn(
            'if [ "$stopping" = 0 ] && ! start_neo; then\n'
            '      log "Could not restart Neo; stopping the container"\n'
            "      shutdown 1\n",
            SCRIPT,
        )

    def test_only_the_browsers_own_argument_counts_as_in_use(self):
        # claw-sdk names a file in the profile; another profile's browser shares a prefix.
        profile = self.profile()
        proc = self.processes(
            ["browseros-claw-server", f"--config={profile}/.browseros/config.json"],
            ["browserclaw", f"--user-data-dir={profile}-other"],
            ["chrome_crashpad_handler", f"--metrics-dir={profile}"],
        )

        result = self.run_clear(profile, "1", proc=proc, gone_tenths=2)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_only_sessions_gone(profile)

    def test_a_symlinked_folder_is_removed_not_followed(self):
        profile = self.profile()
        elsewhere = profile.parent / "elsewhere"
        elsewhere.mkdir()
        (elsewhere / "keep").write_text("kept")
        shutil.rmtree(profile / "Default/Sessions")
        (profile / "Default/Sessions").symlink_to(elsewhere)

        self.run_clear(profile, "1")

        self.assertFalse((profile / "Default/Sessions").is_symlink())
        self.assertEqual((elsewhere / "keep").read_text(), "kept")


class NeoServerInfoTest(unittest.TestCase):
    def kind(self, binary: Path) -> subprocess.CompletedProcess:
        return subprocess.run(
            [SH, str(HERE / "neo-server-info.sh"), "kind", str(binary)],
            capture_output=True, text=True,
        )

    def test_the_version_marker_names_the_server(self):
        binary = Path(self.enterContext(tempfile.TemporaryDirectory())) / "browseros-claw-server"
        server = b"browseros-claw-server-version=0.0.90;"
        sdk = b"browseros-claw-sdk-version=0.1.0;"
        for contents, kind in [
            (server, "claw-server"),
            (sdk, "claw-sdk"),
            # Servers bundled before markers existed are all claw-server.
            (b"", "claw-server"),
            # Release builds refuse a binary with both; a hand-built one gets the closed display.
            (server + b"\0" + sdk, "claw-sdk"),
        ]:
            with self.subTest(contents=contents):
                binary.write_bytes(b"\x7fELF\x02\x01\x01\0" + contents + b"\0\xff\xfe")
                result = self.kind(binary)
                self.assertEqual((result.returncode, result.stdout), (0, f"{kind}\n"))

    def test_an_unreadable_binary_is_an_error_not_claw_server(self):
        result = self.kind(Path(self.enterContext(tempfile.TemporaryDirectory())) / "missing")

        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "")


class ServerPolicyTest(unittest.TestCase):
    def test_a_server_that_cannot_be_identified_stops_the_container(self):
        for identify in ["return 1", "echo claw-next"]:
            with self.subTest(identify=identify):
                result = run_bash(identified_by(identify) + "echo started\n", check=False)

                self.assertEqual(result.returncode, 1)
                self.assertNotIn("started", result.stdout)


class PrepareSettingsTest(unittest.TestCase):
    """Startup takes the host's saved file and copies the fixed image input atomically."""

    def prepare(self, kind="claw-sdk", fixed=None, failure=None, root_link=False, saved_link=None,
                puid="1000", pgid="1000"):
        directory = Path(self.enterContext(tempfile.TemporaryDirectory()))
        root = directory / "root"
        if root_link:
            target = directory / "volume" / "private-state"
            target.mkdir(parents=True, mode=0o700)
            root.symlink_to(target, target_is_directory=True)
        else:
            root.mkdir()
        state = root / "claw-sdk"
        state.mkdir()
        saved = state / "settings.json"
        if saved_link:
            target = directory / "untouched"
            if saved_link == "directory":
                target.mkdir(mode=0o700)
                (target / "original").write_text("unchanged")
            else:
                target.write_text("unchanged")
                target.chmod(0o640)
            saved.symlink_to(target, target_is_directory=saved_link == "directory")
        else:
            saved.write_text('{"trace":{"values":"on"}}')
            saved.chmod(0o644)
        source = directory / "fixed.json"
        if fixed == "directory":
            source.mkdir()
        elif fixed is not None:
            source.write_text(fixed)
        calls, templates = directory / "owners", directory / "templates"
        # The host machine has no neo user. Record its requested ownership and use real
        # filesystem operations for the copy, rename, permissions and saved bytes.
        stub = f'chown() {{ printf "%s\\n" "$*" >> {shlex.quote(str(calls))}; }}\n'
        stub += f'mktemp() {{ printf "%s\\n" "$*" >> {shlex.quote(str(templates))}; command mktemp "$@"; }}\n'
        if failure:
            stub += f'{failure}() {{ return 1; }}\n'
        prepare = shell_function("prepare_settings").replace("/etc/neo/settings.json", str(source))
        result = run_bash(
            server_is(kind) + stub + f'CLAW_DATA={shlex.quote(str(root))} PUID={puid} PGID={pgid}\n'
            + prepare
            + 'prepare_settings\necho startup-continued\n', check=False,
        )
        owners = calls.read_text().splitlines() if calls.exists() else []
        return result, state, saved, source, owners

    def test_a_host_written_folder_and_file_are_taken_for_neo(self):
        result, state, saved, _, owners = self.prepare()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"neo:neo {state}", owners)
        self.assertIn(f"neo:neo {saved}", owners)
        self.assertEqual(saved.stat().st_mode & 0o777, 0o600)

    def test_the_fixed_file_replaces_the_saved_set_without_parsing_it(self):
        for body in ['{"trace":{"values":"off"}}', '{"trace":']:
            with self.subTest(body=body):
                result, state, saved, _, owners = self.prepare(fixed=body)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(saved.read_text(), body)
                self.assertEqual(saved.stat().st_mode & 0o777, 0o600)
                self.assertTrue(any("neo:neo " in owner for owner in owners))
                self.assertEqual(list(state.iterdir()), [saved], "no temporary copy remains")

    def test_preparation_failures_keep_saved_bytes_and_continue_startup(self):
        for command in ["mkdir", "chown", "chmod", "mktemp", "cp", "python3"]:
            with self.subTest(command=command):
                result, state, saved, _, _ = self.prepare(fixed="replacement", failure=command)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn("startup-continued", result.stdout)
                self.assertIn("settings", result.stdout.lower())
                self.assertEqual(saved.read_text(), '{"trace":{"values":"on"}}')
                self.assertEqual(list(state.iterdir()), [saved], "a failed copy leaves no temp file")

    def test_a_directory_at_the_fixed_path_is_logged_and_skipped(self):
        result, _, saved, source, _ = self.prepare(fixed="directory")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(str(source), result.stdout)
        self.assertEqual(saved.read_text(), '{"trace":{"values":"on"}}')

    def test_claw_server_does_not_take_or_replace_settings(self):
        result, _, saved, _, owners = self.prepare(kind="claw-server", fixed="replacement")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(owners, [])
        self.assertEqual(saved.stat().st_mode & 0o777, 0o644)
        self.assertEqual(saved.read_text(), '{"trace":{"values":"on"}}')

    def test_ownership_names_neo_after_the_configured_ids_have_been_applied(self):
        result, _, _, _, owners = self.prepare(fixed="replacement", puid="2307", pgid="2308")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(owners)
        self.assertTrue(all(owner.startswith("neo:neo ") for owner in owners), owners)

    def test_a_state_root_symlink_and_its_target_mode_survive_the_atomic_copy(self):
        result, state, saved, source, _ = self.prepare(fixed="replacement", root_link=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        target = source.parent / "volume" / "private-state"
        self.assertTrue(state.parent.is_symlink())
        self.assertEqual(state.parent.readlink(), target)
        self.assertEqual((target / "claw-sdk" / "settings.json").read_text(), "replacement")
        self.assertEqual(target.stat().st_mode & 0o777, 0o700)
        templates = (source.parent / "templates").read_text().splitlines()
        self.assertEqual(templates, [str(state / ".settings.json.XXXXXX")])
        self.assertEqual(list(state.iterdir()), [saved])

    def test_a_saved_symlink_is_replaced_without_changing_its_target(self):
        for kind in ["file", "directory"]:
            with self.subTest(kind=kind):
                result, state, saved, source, owners = self.prepare(fixed="replacement", saved_link=kind)
                target = source.parent / "untouched"
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertFalse(saved.is_symlink())
                self.assertEqual(saved.read_text(), "replacement")
                self.assertNotIn(f"neo:neo {saved}", owners)
                self.assertEqual((target.stat().st_uid, target.stat().st_gid), (os.getuid(), os.getgid()))
                self.assertEqual(target.stat().st_mode & 0o777, 0o700 if kind == "directory" else 0o640)
                if kind == "directory":
                    self.assertEqual([item.name for item in target.iterdir()], ["original"])
                    self.assertEqual((target / "original").read_text(), "unchanged")
                else:
                    self.assertEqual(target.read_text(), "unchanged")
                self.assertEqual(list(state.iterdir()), [saved])

    def test_a_link_inside_the_state_root_is_skipped_without_writes(self):
        directory = Path(self.enterContext(tempfile.TemporaryDirectory()))
        root, target = directory / "root", directory / "target"
        root.mkdir()
        target.mkdir(mode=0o700)
        (root / "claw-sdk").symlink_to(target, target_is_directory=True)
        result = run_bash(server_is("claw-sdk") + f'CLAW_DATA={shlex.quote(str(root))}\n'
                          + shell_function("prepare_settings") + 'prepare_settings\necho startup-continued\n')
        self.assertIn("symlink", result.stdout)
        self.assertIn("startup-continued", result.stdout)
        self.assertEqual(list(target.iterdir()), [])
        self.assertEqual(target.stat().st_mode & 0o777, 0o700)


class HostSocketTest(unittest.TestCase):
    def prepare(
        self, kind: str, host_gid: str = "", caller_socket: str | None = None
    ) -> tuple[subprocess.CompletedProcess, Path, list[list[str]], str | None]:
        """prepare_host_socket's result, its directory, the chown and setpriv calls it made,
        and the CLAW_HOST_SOCKET a child process (the browser, then claw-sdk) inherits."""
        directory = Path(self.enterContext(tempfile.TemporaryDirectory()))
        socket_dir, chowns, seen = directory / "run-neo", directory / "chowns", directory / "seen"
        caller = (
            f"export CLAW_HOST_SOCKET={shlex.quote(caller_socket)}\n" if caller_socket else ""
        )
        result = run_bash(
            server_is(kind)
            # Outside the container there is no neo user; 1000 is the image's default gid.
            + "id() { [ \"$*\" = '-g neo' ] && echo 1000; }\n"
            + f"chown() {{ local IFS=$'\\t'; echo \"$*\" >> '{chowns}'; }}\n"
            # Records the identity it was asked for, then runs the command as this user.
            + f"setpriv() {{ local IFS=$'\\t'; echo \"setpriv\t$*\" >> '{chowns}'; "
            + 'while [ "$1" != --clear-groups ]; do shift; done; shift; "$@"; }\n'
            + f"NEO_HOST_GID={shlex.quote(host_gid)} HOST_SOCKET_DIR='{socket_dir}'\n"
            + caller
            + shell_function("prepare_host_socket")
            + "prepare_host_socket\n"
            + f"sh -c 'printf %s \"${{CLAW_HOST_SOCKET-}}\"' > '{seen}'\n",
            check=False,
        )
        lines = chowns.read_text().splitlines() if chowns.exists() else []
        calls = [line.split("\t") for line in lines]
        inherited = seen.read_text() if seen.exists() else None
        return result, socket_dir, calls, inherited

    def test_claw_sdk_gets_a_setgid_directory_owned_by_the_host_group(self):
        result, socket_dir, chowns, inherited = self.prepare("claw-sdk", host_gid="2005")

        self.assertEqual(result.returncode, 0, result.stderr)
        # The chmod runs in the host's group: without CAP_FSETID, Linux would drop the setgid
        # bit for a process outside it.
        self.assertEqual(
            chowns,
            [
                ["neo:2005", str(socket_dir)],
                ["setpriv", "--regid", "2005", "--clear-groups", "chmod", "2750", str(socket_dir)],
            ],
        )
        # setgid: the socket claw-sdk binds inside takes the directory's group.
        self.assertEqual(socket_dir.stat().st_mode & 0o7777, 0o2750)
        self.assertEqual(inherited, f"{socket_dir}/host.sock")

    def test_without_a_host_group_the_socket_stays_with_neos_group(self):
        _, socket_dir, chowns, _ = self.prepare("claw-sdk")

        self.assertEqual(chowns[0], ["neo:1000", str(socket_dir)])
        self.assertEqual(chowns[1][:3], ["setpriv", "--regid", "1000"])

    def test_a_callers_socket_path_wins(self):
        _, socket_dir, _, inherited = self.prepare(
            "claw-sdk", caller_socket="/tmp/dev host.sock"
        )

        self.assertTrue(socket_dir.is_dir())
        self.assertEqual(inherited, "/tmp/dev host.sock")

    def test_a_group_that_is_not_a_number_stops_the_container(self):
        for gid in ["neo-host", "-1", "2005 "]:
            with self.subTest(gid=gid):
                result, socket_dir, chowns, _ = self.prepare("claw-sdk", host_gid=gid)

                self.assertEqual(result.returncode, 1)
                self.assertIn("NEO_HOST_GID must be numeric", result.stderr)
                self.assertFalse(socket_dir.exists())
                self.assertEqual(chowns, [])

    def test_claw_server_gets_no_socket_directory_and_no_socket_path(self):
        result, socket_dir, chowns, inherited = self.prepare("claw-server", host_gid="2005")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(socket_dir.exists())
        self.assertEqual(chowns, [])
        self.assertEqual(inherited, "")


@unittest.skipUnless(shutil.which("jq"), "needs jq, as the image has")
class McpListenersTest(unittest.TestCase):
    """The Local State prepare_data leaves for the browser's two MCP proxies."""

    def local_state(
        self, neo_mcp_remote: str | None, saved: dict | None = None
    ) -> tuple[subprocess.CompletedProcess, dict | None]:
        directory = Path(self.enterContext(tempfile.TemporaryDirectory()))
        profile = directory / "data/profile"
        if saved is not None:
            profile.mkdir(parents=True)
            (profile / "Local State").write_text(json.dumps(saved))
        default = re.search(r'^: "\$\{NEO_MCP_REMOTE:=true\}"$', SCRIPT, re.MULTILINE)
        parse = re.search(
            r"^MCP_REMOTE=false\n.*?^readonly MCP_REMOTE\n", SCRIPT, re.MULTILINE | re.DOTALL
        )
        assert default and parse, "entrypoint.sh no longer reads NEO_MCP_REMOTE here"
        given = (
            "" if neo_mcp_remote is None else f"NEO_MCP_REMOTE={shlex.quote(neo_mcp_remote)}\n"
        )
        result = run_bash(
            "chown() { :; }\n"
            "log() { printf '[neo] %s\\n' \"$*\"; }\n"
            'die() { log "$*" >&2; exit 1; }\n'
            + shell_function("is_true")
            + given
            + default.group(0) + "\n"
            + parse.group(0)
            + f"NEO_DATA_DIR='{directory}/data' PROFILE='{profile}' NEO_HOME='{directory}'\n"
            + f"DOWNLOADS='{directory}/data/downloads' OUTPUTS='{directory}/data/out'\n"
            + f"CLAW_DATA='{directory}/data/browserclaw' XDG_DIR='{directory}/xdg'\n"
            + "MCP_TLS_PORT=9011\n"
            + shell_function("merge_json")
            + shell_function("prepare_data")
            + "prepare_data\n",
            check=False,
        )
        state = profile / "Local State"
        return result, json.loads(state.read_text()) if state.exists() else None

    def test_unset_keeps_remote_peers_allowed_as_today(self):
        result, state = self.local_state(None)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            state,
            {"browseros": {"server": {"allow_remote_in_mcp": True, "proxy_https_port": 9011}}},
        )

    def test_false_keeps_both_mcp_ports_to_loopback_peers(self):
        for value in ["false", "OFF", "0"]:
            with self.subTest(value=value):
                _, state = self.local_state(value)

                assert state is not None
                self.assertIs(state["browseros"]["server"]["allow_remote_in_mcp"], False)

    def test_a_value_that_is_not_a_boolean_stops_the_container(self):
        result, state = self.local_state("loopback")

        self.assertEqual(result.returncode, 1)
        self.assertIn("expected true or false, got 'loopback'", result.stderr)
        self.assertIsNone(state)

    def test_every_start_pins_the_https_port_and_keeps_the_profiles_other_state(self):
        server = {"proxy_https_port": 9012, "proxy_port": 9010, "allow_remote_in_mcp": True}
        saved = {"browseros": {"server": server}, "browser": {"enabled_labs_experiments": ["x"]}}

        _, state = self.local_state("false", saved)

        self.assertEqual(
            state,
            {
                "browseros": {
                    "server": {
                        "proxy_https_port": 9011,
                        "proxy_port": 9010,
                        "allow_remote_in_mcp": False,
                    }
                },
                "browser": {"enabled_labs_experiments": ["x"]},
            },
        )


class Display(NamedTuple):
    spawned: list[list[str]]
    supervised: list[str]
    viewer: bool
    # Whether the display socket already existed when Xvnc was started.
    stale_at_xvnc: bool
    lock_left: bool


class StartDisplayTest(unittest.TestCase):
    def start(self, kind: str, stale: bool = False) -> Display:
        """What start_display spawns and leaves. The stand-in Xvnc creates the display socket
        start_display waits for; `stale` leaves a killed Xvnc's socket and lock first."""
        directory = Path(self.enterContext(tempfile.TemporaryDirectory()))
        calls, supervised = directory / "calls", directory / "supervised"
        mark, x11, lock = directory / "neo-viewer", directory / "x11", directory / "X1-lock"
        seen = directory / "stale-at-xvnc"
        x11.mkdir()
        if stale:
            with socket.socket(socket.AF_UNIX) as display:
                display.bind(str(x11 / "X1"))
            lock.write_text("       42\n")
        bind = f"import socket; socket.socket(socket.AF_UNIX).bind('{x11 / 'X1'}')"
        run_bash(
            server_is(kind)
            + "spawn_as_neo() {\n"
            + f"  local IFS=$'\\t'; echo \"$*\" >> '{calls}'\n"
            + f"  if [ \"$1\" = Xvnc ]; then [ ! -e '{x11 / 'X1'}' ] || touch '{seen}';"
            + f" {shlex.quote(sys.executable)} -c \"{bind}\"; fi\n"
            + "  : &\n}\n"
            + f"NEO_SCREEN=1600x1000 VIEWER_MARK='{mark}'\n"
            + shell_function("start_display")
            .replace("/tmp/.X11-unix", str(x11))
            .replace("/tmp/.X1-lock", str(lock))
            + f"start_display\nwait\nprintf '%s\\n' \"${{supervised[@]}}\" > '{supervised}'\n"
        )
        return Display(
            [line.split("\t") for line in calls.read_text().splitlines()],
            supervised.read_text().split(),
            mark.exists(),
            seen.exists(),
            lock.exists(),
        )

    def test_claw_server_keeps_todays_display(self):
        display = self.start("claw-server")

        self.assertEqual(display.spawned, [TODAY_XVNC, VNCCONFIG, OPENBOX, TODAY_WEBSOCKIFY])
        self.assertEqual(display.supervised, ["xvnc", "openbox", "websockify"])
        self.assertTrue(display.viewer)

    def test_claw_sdk_keeps_the_clipboard_from_viewers_and_serves_no_novnc(self):
        display = self.start("claw-sdk")

        self.assertEqual(display.spawned, [TODAY_XVNC + CLIPBOARD_OFF, VNCCONFIG, OPENBOX])
        self.assertEqual(display.supervised, ["xvnc", "openbox"])
        self.assertFalse(display.viewer)

    def test_a_killed_xvncs_socket_and_lock_are_gone_before_xvnc_starts(self):
        # Left in place, the socket would let start_display go on before the new Xvnc listens.
        display = self.start("claw-sdk", stale=True)

        self.assertFalse(display.stale_at_xvnc)
        self.assertFalse(display.lock_left)
        self.assertEqual(display.spawned[0], TODAY_XVNC + CLIPBOARD_OFF)


class ConfigureNeoTest(unittest.TestCase):
    PUT_SETTINGS = [
        "-fsS", "-X", "PUT", "http://127.0.0.1:9010/api/v1/settings/agent",
        "-H", "Content-Type: application/json", "-d", '{"humanHelpEnabled":false}',
    ]

    def configure(self, kind: str, settings_status: int = 0) -> tuple[int, list[list[str]], bool, str]:
        """configure_neo's exit status as the supervisor's wait sees it, its curl calls,
        whether it marked the container configured, and its log."""
        directory = Path(self.enterContext(tempfile.TemporaryDirectory()))
        calls, configured = directory / "calls", directory / "neo-configured"
        result = run_bash(
            server_is(kind)
            + f"curl() {{ local IFS=$'\\t'; echo \"$*\" >> '{calls}'; "
            + f'case "$*" in *PUT*) return {settings_status} ;; esac; }}\n'
            + f"MCP_URL=http://127.0.0.1:9010 CONFIGURED='{configured}'\n"
            + shell_function("configure_neo")
            # The supervisor starts it in the background and reads its status with wait.
            + "configure_neo &\nif wait $!; then echo status=0; else echo status=$?; fi\n"
        )
        status = int(result.stdout.splitlines()[-1].removeprefix("status="))
        curls = [line.split("\t") for line in calls.read_text().splitlines()]
        return status, curls, configured.exists(), result.stdout

    def test_claw_server_turns_human_help_off_before_it_reports_ready(self):
        status, curls, configured, log = self.configure("claw-server")

        self.assertEqual(status, 0)
        self.assertIn(self.PUT_SETTINGS, curls)
        self.assertTrue(configured)
        self.assertIn("Viewer: http://localhost:6080", log)

    def test_claw_server_still_stops_when_the_settings_call_fails(self):
        status, _, configured, log = self.configure("claw-server", settings_status=22)

        self.assertEqual(status, 1)
        self.assertFalse(configured)
        self.assertIn("Could not apply agent settings", log)

    def test_claw_sdk_is_never_sent_the_settings_call_and_still_turns_ready(self):
        status, curls, configured, log = self.configure("claw-sdk")

        self.assertEqual(status, 0)
        self.assertFalse(any("/api/v1/settings/agent" in arg for curl in curls for arg in curl))
        self.assertTrue(configured)
        self.assertNotIn("Viewer:", log)


class ShutdownTest(unittest.TestCase):
    def terminated(self, websockify_pid: str) -> list[str]:
        """The arguments of shutdown's SIGTERM to the display, once Neo has stopped."""
        result = run_bash(
            "kill() { local IFS=$'\\t'; echo \"$*\"; }\n"
            "wait() { :; }\n"
            "neo_pid='' configure_pid=''\n"
            f"websockify_pid='{websockify_pid}' openbox_pid=3 vncconfig_pid=2 xvnc_pid=1\n"
            + shell_function("shutdown")
            + "shutdown 0\n"
        )
        return result.stdout.rstrip("\n").split("\t")

    def test_the_display_stops_in_todays_order(self):
        self.assertEqual(self.terminated("4"), ["-TERM", "4", "3", "2", "1"])

    def test_a_viewer_that_never_started_is_skipped(self):
        self.assertEqual(self.terminated(""), ["-TERM", "3", "2", "1"])


class HealthcheckTest(unittest.TestCase):
    READY = "http://127.0.0.1:9010/system/ready"
    NOVNC = "http://127.0.0.1:6080/"

    def check(self, viewer: bool, down: str = "") -> tuple[int, list[str]]:
        """The healthcheck's exit status and the URLs it probed, in a configured container."""
        directory = Path(self.enterContext(tempfile.TemporaryDirectory()))
        configured, mark, probes = (
            directory / "neo-configured", directory / "neo-viewer", directory / "probes"
        )
        configured.touch()
        if viewer:
            mark.touch()
        curl = directory / "curl"
        curl.write_text(f"#!/bin/sh\necho \"$2\" >> '{probes}'\n[ \"$2\" != '{down}' ]\n")
        curl.chmod(0o755)
        script = HEALTHCHECK.replace("/tmp/neo-configured", str(configured)).replace(
            "/tmp/neo-viewer", str(mark)
        )
        result = subprocess.run(
            [SH, "-c", script],
            env={**os.environ, "PATH": f"{directory}{os.pathsep}{os.environ['PATH']}"},
            capture_output=True, text=True,
        )
        return result.returncode, probes.read_text().split() if probes.exists() else []

    def test_probes_novnc_only_where_the_entrypoint_started_it(self):
        self.assertEqual(self.check(viewer=True), (0, [self.READY, self.NOVNC]))
        self.assertEqual(self.check(viewer=False), (0, [self.READY]))

    def test_a_viewer_that_stops_answering_makes_the_container_unhealthy(self):
        status, _ = self.check(viewer=True, down=self.NOVNC)

        self.assertNotEqual(status, 0)


if __name__ == "__main__":
    unittest.main()
