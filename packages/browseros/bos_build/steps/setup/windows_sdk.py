"""Satisfy the selected Chromium pin's host Windows SDK before product work.

Source preparation owns checkout/sync; this module owns the machine prerequisite.
Local builds only inspect it. Explicitly opted-in, elevated GitHub runners may
install Microsoft's SDK, then prove Chromium can load both x86 and x64 tools.
"""

import ast
import ctypes
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request
from dataclasses import dataclass
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import urlparse

from ...core.context import Context
from ...core.step import Step, ValidationError, step
from ...lib.utils import log_info, log_success, log_warning

_DOWNLOADS = "https://learn.microsoft.com/en-us/windows/apps/windows-sdk/downloads"
_FEATURES = (
    "OptionId.DesktopCPPx86",
    "OptionId.DesktopCPPx64",
    "OptionId.WindowsDesktopDebuggers",
)
_VERSION = r"\d+\.\d+\.\d+\.\d+"


@dataclass(frozen=True)
class _Requirement:
    """Keep the installer servicing version distinct from the SDK folder pin."""

    directory: str
    package: str
    debugger: str


def _requirement(ctx: Context) -> _Requirement:
    # Do this just-in-time, never during whole-plan preflight: provisioning may
    # replace a cached older checkout earlier in this very pipeline.
    head = _capture(["git", "rev-parse", "HEAD"], ctx.chromium_src).strip()
    pin = _capture(
        ["git", "rev-parse", f"refs/tags/{ctx.chromium_version}^{{commit}}"],
        ctx.chromium_src,
    ).strip()
    if head != pin:
        raise ValidationError(
            f"Windows SDK check requires prepared Chromium {ctx.chromium_version}; "
            f"HEAD is {head}, expected {pin}. Run source preparation first."
        )
    versions = []
    for relative in ("build/vs_toolchain.py", "build/toolchain/win/setup_toolchain.py"):
        path = ctx.chromium_src / relative
        tree = ast.parse(path.read_text(encoding="utf-8"))
        values = [
            node.value.value
            for node in tree.body
            if isinstance(node, ast.Assign)
            and any(
                isinstance(t, ast.Name) and t.id == "SDK_VERSION" for t in node.targets
            )
            and isinstance(node.value, ast.Constant)
            and isinstance(node.value.value, str)
        ]
        if len(values) != 1 or not re.fullmatch(_VERSION, values[0]):
            raise ValidationError(f"Cannot read Chromium SDK_VERSION from {path}")
        versions.append(values[0])
    if versions[0] != versions[1]:
        raise ValidationError(f"Chromium SDK pins disagree: {versions}")
    instructions = (ctx.chromium_src / "docs/windows_build_instructions.md").read_text(
        encoding="utf-8"
    )
    match = re.search(
        r"\[Windows (?:10|11) SDK\]\([^\n]+\)\s*version\s+(" + _VERSION + r")",
        instructions,
    )
    if not match or match[1].split(".")[:3] != versions[0].split(".")[:3]:
        raise ValidationError(
            "Cannot reconcile the Windows SDK package in pinned Chromium docs "
            "with SDK_VERSION; update the SDK requirement reader for this pin."
        )
    debugger = re.search(r"SDK Debugging Tools\s+(" + _VERSION + r")", instructions)
    if not debugger:
        raise ValidationError(
            "Cannot read the debugging tools requirement from pinned Chromium docs"
        )
    return _Requirement(versions[0], match[1], debugger[1])


def _sdk_root() -> Path:
    # Match vs_toolchain.py's host-SDK lookup, including an explicit local override.
    return Path(
        os.environ.get("WINDOWSSDKDIR")
        or (
            Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)"))
            / "Windows Kits"
            / "10"
        )
    )


def _required_files(root: Path, version: str, target: str) -> list[Path]:
    headers = [
        "um/windows.h",
        "shared/sdkddkver.h",
        "ucrt/stdio.h",
        "winrt/windows.foundation.h",
        "cppwinrt/winrt/base.h",
    ]
    files = [root / "Include" / version / name for name in headers]
    for arch in ("x86", "x64"):
        files.extend(
            [
                root / "Lib" / version / "um" / arch / "kernel32.lib",
                root / "Lib" / version / "ucrt" / arch / "ucrt.lib",
                root / "Debuggers" / arch / "dbghelp.dll",
            ]
        )
        files.extend(
            root / "bin" / version / arch / tool
            for tool in ("rc.exe", "mt.exe", "midl.exe", "d3dcompiler_47.dll")
        )
    # Cross-compiling ARM64 still uses x64-hosted tools. Keep that existing
    # local flow checkable without adding ARM provisioning to the CI image.
    if target == "arm64":
        files.extend(
            [
                root / "Lib" / version / "um" / "arm64" / "kernel32.lib",
                root / "Lib" / version / "ucrt" / "arm64" / "ucrt.lib",
                root / "Debuggers" / "arm64" / "dbghelp.dll",
            ]
        )
    return files


def _installed_packages() -> list[str]:
    import winreg

    versions = set()
    # SDK bundle registration is machine-wide, often in the 32-bit registry
    # view. DisplayVersion uses 10.1.BUILD.SERVICE while downloads use 10.0.
    for view in (winreg.KEY_WOW64_32KEY, winreg.KEY_WOW64_64KEY):
        with winreg.OpenKey(
            winreg.HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
            0,
            winreg.KEY_READ | view,
        ) as root:
            for index in range(winreg.QueryInfoKey(root)[0]):
                with winreg.OpenKey(root, winreg.EnumKey(root, index)) as key:
                    try:
                        name = winreg.QueryValueEx(key, "DisplayName")[0]
                        version = winreg.QueryValueEx(key, "DisplayVersion")[0]
                    except FileNotFoundError:
                        continue
                    if name.startswith(
                        "Windows Software Development Kit"
                    ) and re.fullmatch(_VERSION, version):
                        versions.add(version)
    return sorted(versions)


def _missing(root: Path, requirement: _Requirement, target: str) -> list[str]:
    missing = [
        str(p)
        for p in _required_files(root, requirement.directory, target)
        if not p.is_file() or p.stat().st_size == 0
    ]
    installed = _installed_packages()
    log_info(f"Registered Windows SDK packages: {', '.join(installed) or 'none'}")
    required = tuple(map(int, requirement.package.split(".")))
    if not any(
        (v[0], v[2]) == (required[0], required[2]) and v[3] >= required[3]
        for v in (tuple(map(int, value.split("."))) for value in installed)
    ):
        missing.append(
            f"SDK package >= {requirement.package} in the {requirement.directory} family"
        )
    # Debuggers live outside the versioned SDK tree. A new SDK registration
    # alone cannot prove an older separately installed dbghelp supports large PDBs.
    for arch in dict.fromkeys(("x86", "x64", target)):
        dll = root / "Debuggers" / arch / "dbghelp.dll"
        if dll.is_file():
            version = json.loads(
                _capture(
                    [
                        "powershell.exe",
                        "-NoProfile",
                        "-NonInteractive",
                        "-Command",
                        "$v = (Get-Item -LiteralPath $env:BROWSEROS_SDK_FILE).VersionInfo; "
                        "@($v.FileMajorPart, $v.FileMinorPart, $v.FileBuildPart, $v.FilePrivatePart) | ConvertTo-Json -Compress",
                    ],
                    root,
                    dict(os.environ, BROWSEROS_SDK_FILE=str(dll)),
                )
            )
            log_info(f"Debugger {arch} file version: {'.'.join(map(str, version))}")
            if tuple(version) < tuple(map(int, requirement.debugger.split("."))):
                missing.append(f"{dll} must be >= {requirement.debugger}")
    return missing


class _Downloads(HTMLParser):
    """Read Microsoft's release table so future Chromium pins need no URL map."""

    def __init__(self, version: str):
        super().__init__()
        self.version = version
        self.rows: list[str] = []
        self.links: list[str] = []
        self.in_row = False
        self.installer: str | None = None

    def handle_starttag(self, tag, attrs):
        if tag == "tr":
            self.in_row = True
            self.rows, self.links = [], []
        elif tag == "a" and self.in_row:
            href = dict(attrs).get("href", "")
            if urlparse(href).hostname == "go.microsoft.com":
                self.links.append(href)

    def handle_data(self, data):
        if self.in_row:
            self.rows.append(data)

    def handle_endtag(self, tag):
        if tag == "tr":
            if f"({self.version})" in "".join(self.rows) and self.links:
                # Microsoft's table orders Installer before ISO. Require the
                # label order as well, failing closed if the catalog changes.
                row = "".join(self.rows)
                if "Installer" in row and (
                    "ISO" not in row or row.index("Installer") < row.index("ISO")
                ):
                    self.installer = self.links[0]
            self.in_row = False


def _download(url: str, destination: Path) -> None:
    for attempt in range(3):
        try:
            with (
                urllib.request.urlopen(url, timeout=60) as response,
                destination.open("wb") as output,
            ):
                if urlparse(response.url).scheme != "https":
                    raise ValidationError("SDK download redirected away from HTTPS")
                shutil.copyfileobj(response, output)
            return
        except OSError:
            if attempt == 2:
                raise
            time.sleep(5)


def _install(requirement: _Requirement, root: Path) -> None:
    if (
        os.environ.get("BROWSEROS_INSTALL_WINDOWS_SDK") != "1"
        or os.environ.get("GITHUB_ACTIONS") != "true"
    ):
        raise ValidationError(
            f"Install Windows SDK {requirement.package} with x86/x64 Desktop C++ "
            "and Debugging Tools using Microsoft's SDK installer, then retry. "
            "Automatic machine-wide installation requires an elevated managed "
            "GitHub runner with BROWSEROS_INSTALL_WINDOWS_SDK=1; local builds only check."
        )
    if not ctypes.windll.shell32.IsUserAnAdmin():
        raise ValidationError(
            "SDK installation requires an already-elevated CI runner; no UAC prompt will be opened."
        )
    # Logs survive failure; the downloaded executable is removed after use. A
    # failed attempt never writes a readiness marker: every retry inspects files.
    directory = Path(tempfile.mkdtemp(prefix="browseros-windows-sdk-"))
    log_info(f"Windows SDK installer logs: {directory}")
    catalog = directory / "downloads.html"
    _download(_DOWNLOADS, catalog)
    parser = _Downloads(requirement.package)
    parser.feed(catalog.read_text(encoding="utf-8"))
    if not parser.installer:
        raise ValidationError(
            f"Microsoft's supported SDK catalog has no installer for {requirement.package}: {_DOWNLOADS}"
        )
    installer = directory / "winsdksetup.exe"
    try:
        _download(parser.installer, installer)
        env = dict(os.environ, BROWSEROS_SDK_INSTALLER=str(installer))
        # The catalog is live, so require Microsoft's Windows trust-chain
        # validation before executing its bootstrapper with machine privileges.
        _capture(
            [
                shutil.which("pwsh") or "powershell.exe",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "$ErrorActionPreference = 'Stop'; "
                "$s = Get-AuthenticodeSignature -LiteralPath $env:BROWSEROS_SDK_INSTALLER; "
                "if ($s.Status -ne 'Valid' -or $s.SignerCertificate.Subject -notmatch '(^|, )CN=Microsoft Corporation(,|$)') "
                "{ throw 'Windows SDK installer is not validly signed by Microsoft Corporation' }; "
                "$s.SignerCertificate.Subject",
            ],
            directory,
            env,
        )
        for attempt in range(3):
            command = [
                str(installer),
                "/quiet",
                "/norestart",
                "/ceip",
                "off",
                "/installpath",
                str(root),
                "/log",
                str(directory / f"install-{attempt + 1}.log"),
                "/features",
                *_FEATURES,
            ]
            log_info(
                f"Installing Windows SDK {requirement.package} (attempt {attempt + 1})"
            )
            try:
                result = subprocess.run(command, timeout=1800, check=False)
            except subprocess.TimeoutExpired as exc:
                raise ValidationError(
                    f"SDK installer exceeded 30 minutes; inspect {directory} and finish any active installer before retrying."
                ) from exc
            code = result.returncode & 0xFFFFFFFF
            if code in (0, 3010):
                if code == 3010:
                    log_warning(
                        "SDK installer requested a reboot; checking whether this runner can use the SDK now."
                    )
                return
            # Burn can wrap MSI's busy code in HRESULT_FROM_WIN32. Retrying
            # only this transient failure avoids concealing fatal setup errors.
            if code in (1618, 0x80070652) and attempt < 2:
                time.sleep(15)
                continue
            raise ValidationError(
                f"SDK installer failed with exit {code} (0x{code:08X}); logs: {directory}. Fix the installer error and rerun windows_sdk."
            )
    finally:
        installer.unlink(missing_ok=True)


def _capture(command: list[str], cwd: Path, env: dict[str, str] | None = None) -> str:
    if Path(command[0]).stem.lower() in ("powershell", "pwsh"):
        env = dict(os.environ if env is None else env)
        # GitHub's pwsh shell exports its module path. A different PowerShell
        # edition must rebuild that path or native security cmdlets can fail.
        env.pop("PSModulePath", None)
    result = subprocess.run(
        command,
        cwd=cwd,
        env=env,
        text=True,
        encoding="utf-8",
        errors="replace",
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=120,
        check=False,
    )
    if result.returncode:
        raise ValidationError(
            f"{Path(command[0]).name} failed ({result.returncode}):\n{result.stdout}"
        )
    return result.stdout


def _verify_toolchain(ctx: Context, root: Path, requirement: _Requirement) -> None:
    env = dict(os.environ, DEPOT_TOOLS_WIN_TOOLCHAIN="0", WINDOWSSDKDIR=str(root))
    output = _capture(
        [
            sys.executable,
            str(ctx.chromium_src / "build/vs_toolchain.py"),
            "get_toolchain_dir",
        ],
        ctx.chromium_src,
        env,
    )
    # These scripts print GN-value syntax, but neither invokes GN. Passing
    # 'none' explicitly prevents setup_toolchain from writing environment files.
    values = dict(
        re.findall(
            r'^(vs_path|sdk_path|sdk_version|runtime_dirs) = "(.*)"$',
            output,
            re.MULTILINE,
        )
    )
    if values.get("sdk_version") != requirement.directory or not all(
        name in values for name in ("vs_path", "sdk_path", "runtime_dirs")
    ):
        raise ValidationError(f"Unexpected Chromium toolchain selection:\n{output}")
    for arch in dict.fromkeys(("x86", "x64", ctx.architecture)):
        result = _capture(
            [
                sys.executable,
                str(ctx.chromium_src / "build/toolchain/win/setup_toolchain.py"),
                values["vs_path"],
                values["sdk_path"],
                values["runtime_dirs"],
                "win",
                arch,
                "none",
            ],
            ctx.chromium_src,
            env,
        )
        if requirement.directory not in result:
            raise ValidationError(
                f"Chromium {arch} environment did not select SDK {requirement.directory}:\n{result}"
            )
        log_info(
            f"Chromium setup_toolchain passed for {arch}: {values['vs_path']}, SDK {values['sdk_path']}/{requirement.directory}"
        )


@step("windows_sdk", phase="setup", platforms=("windows",))
class WindowsSDKModule(Step):
    """One pipeline seam for requirement discovery, installation, and acceptance."""

    description = "Verify the pinned Windows SDK; install only on opted-in managed CI"

    def validate(self, ctx: Context) -> None:
        if sys.platform != "win32":
            raise ValidationError("Host Windows SDK setup requires Windows")

    def execute(self, ctx: Context) -> None:
        requirement = _requirement(ctx)
        root = _sdk_root()
        log_info(
            f"Chromium {ctx.chromium_version}: Windows SDK package {requirement.package}, directory {requirement.directory}, root {root}"
        )
        missing = _missing(root, requirement, ctx.architecture)
        if missing:
            log_warning("Missing Windows SDK prerequisites:\n" + "\n".join(missing))
            if ctx.architecture != "x64":
                raise ValidationError(
                    f"Install SDK {requirement.package} with Desktop C++ and Debugging "
                    f"Tools for {ctx.architecture}, then retry. Automatic CI SDK "
                    "installation is scoped to x64; existing ARM64 installations "
                    "remain supported."
                )
            _install(requirement, root)
            missing = _missing(root, requirement, ctx.architecture)
            if missing:
                raise ValidationError(
                    "SDK installer completed but required files/package are still missing:\n"
                    + "\n".join(missing)
                )
        else:
            log_info("Windows SDK already complete; skipping installer")
        for path in _required_files(root, requirement.directory, ctx.architecture):
            log_info(f"Verified SDK file: {path} ({path.stat().st_size} bytes)")
        _verify_toolchain(ctx, root, requirement)
        log_success("Pinned Windows SDK and Chromium x86/x64 environments verified")
