"""Run entrypoint.sh functions under bash, with the container's commands stubbed."""

import json
import os
import re
import shlex
import shutil
import socket
import subprocess
import tempfile
import unittest
from pathlib import Path

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


class OutputDirectoryTest(unittest.TestCase):
    def test_output_directory_follows_data_dir_unless_overridden(self):
        declaration = re.search(r"^readonly OUTPUTS=.*$", SCRIPT, re.MULTILINE)
        self.assertIsNotNone(declaration)
        for override, expected in [
            ("", "/var/lib/mirror/neo/tool-output"),
            ("/shared/output files", "/shared/output files"),
        ]:
            result = run_bash(
                "NEO_DATA_DIR=/var/lib/mirror/neo\n"
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
                "BROWSERCLAW_HOSTED_MODE=1\n"
                f"OUTPUTS={shlex.quote(output)}\n"
                + shell_function("spawn_as_neo")
                + "spawn_as_neo browserclaw\nwait\n"
            )
            self.assertIn(f"BROWSEROS_OUTPUT_DIR={output}", argv.read_text().splitlines())


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


class StartDisplayTest(unittest.TestCase):
    def start(self, kind: str) -> tuple[list[list[str]], list[str], bool]:
        """The processes start_display spawns, the ones the supervisor watches, and
        whether it marked a viewer for the healthcheck."""
        directory = Path(self.enterContext(tempfile.TemporaryDirectory()))
        calls, supervised = directory / "calls", directory / "supervised"
        mark, x11 = directory / "neo-viewer", directory / "x11"
        x11.mkdir()
        # The display socket Xvnc creates, which start_display waits for.
        with socket.socket(socket.AF_UNIX) as display:
            display.bind(str(x11 / "X1"))
        run_bash(
            server_is(kind)
            + f"spawn_as_neo() {{ local IFS=$'\\t'; echo \"$*\" >> '{calls}'; : & }}\n"
            + f"NEO_SCREEN=1600x1000 VIEWER_MARK='{mark}'\n"
            + shell_function("start_display").replace("/tmp/.X11-unix", str(x11))
            + f"start_display\nwait\nprintf '%s\\n' \"${{supervised[@]}}\" > '{supervised}'\n"
        )
        spawned = [line.split("\t") for line in calls.read_text().splitlines()]
        return spawned, supervised.read_text().split(), mark.exists()

    def test_claw_server_keeps_todays_display(self):
        spawned, supervised, viewer = self.start("claw-server")

        self.assertEqual(spawned, [TODAY_XVNC, VNCCONFIG, OPENBOX, TODAY_WEBSOCKIFY])
        self.assertEqual(supervised, ["xvnc", "openbox", "websockify"])
        self.assertTrue(viewer)

    def test_claw_sdk_keeps_the_clipboard_from_viewers_and_serves_no_novnc(self):
        spawned, supervised, viewer = self.start("claw-sdk")

        self.assertEqual(spawned, [TODAY_XVNC + CLIPBOARD_OFF, VNCCONFIG, OPENBOX])
        self.assertEqual(supervised, ["xvnc", "openbox"])
        self.assertFalse(viewer)


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
