#!/usr/bin/env python3
"""Behavior tests for the public Linux packaging interface."""

import hashlib
import os
import shutil
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from bos_build.core.context import ArtifactRegistry
from bos_build.core.products import get_product_descriptor
from bos_build.steps.package.linux_packaging import (
    LinuxArtifactPair,
    LinuxPackagingError,
    build_linux_artifacts,
    require_linux_artifacts,
)
from bos_build.steps.package.linux_packaging.system import SubprocessLinuxToolchain


def _packaging_context(dist_dir: Path) -> SimpleNamespace:
    names = {
        "appimage": "BrowserOS_v1.2.3_x64.AppImage",
        "deb": "BrowserOS_v1.2.3_amd64.deb",
    }
    return SimpleNamespace(
        architecture="x64",
        artifact_registry=ArtifactRegistry(),
        get_artifact_name=lambda kind: names[kind],
        get_dist_dir=lambda: dist_dir,
    )


def _write_artifact(path: Path, content: bytes = b"package") -> None:
    path.write_bytes(content)
    if path.suffix == ".AppImage":
        path.chmod(0o755)


def _build_context(root: Path) -> SimpleNamespace:
    product = get_product_descriptor("browseros")
    chromium_src = root / "chromium"
    out_dir = Path("out/Default_browseros_x64")
    dist_dir = root / "releases/1.2.3"
    names = {
        "appimage": "BrowserOS_v1.2.3_x64.AppImage",
        "deb": "BrowserOS_v1.2.3_amd64.deb",
    }
    return SimpleNamespace(
        root_dir=root,
        chromium_src=chromium_src,
        out_dir=str(out_dir),
        architecture="x64",
        product=product,
        BROWSEROS_APP_NAME=product.linux.launcher_name,
        artifact_registry=ArtifactRegistry(),
        get_artifact_name=lambda kind: names[kind],
        get_browseros_chromium_version=lambda: "136.0.0.0.1",
        get_dist_dir=lambda: dist_dir,
    )


def _write_complete_runtime(ctx: SimpleNamespace) -> None:
    out_dir = ctx.chromium_src / ctx.out_dir
    out_dir.mkdir(parents=True)
    for name in (
        ctx.BROWSEROS_APP_NAME,
        "chrome_crashpad_handler",
        "chrome_sandbox",
        "icudtl.dat",
        "v8_context_snapshot.bin",
        "chrome_100_percent.pak",
        "chrome_200_percent.pak",
        "resources.pak",
        "libEGL.so",
        "libGLESv2.so",
        "libqt5_shim.so",
        "libqt6_shim.so",
    ):
        (out_dir / name).write_bytes(f"contents:{name}".encode())
    for directory, filename in (
        ("browseros_extensions", "manifest.json"),
        ("locales", "en-US.pak"),
        ("BrowserOSServer", "browseros_server"),
    ):
        path = out_dir / directory / filename
        path.parent.mkdir(parents=True)
        path.write_bytes(f"contents:{directory}/{filename}".encode())

    icon = (
        Path(ctx.root_dir)
        / "resources"
        / ctx.product.id
        / "icons"
        / "product_logo_256.png"
    )
    icon.parent.mkdir(parents=True)
    icon.write_bytes(b"png-icon")


class _FakeDownloader:
    def __init__(self, content: bytes) -> None:
        self.content = content
        self.urls: list[str] = []

    def download(self, url: str, destination: Path) -> None:
        self.urls.append(url)
        destination.write_bytes(self.content)


class _FakeLinuxToolchain:
    """Model only the external archive commands; staging remains production code."""

    def __init__(
        self,
        *,
        unresolved: tuple[str, ...] = (),
        storage: Path | None = None,
        fail_deb: bool = False,
        omit_extracted_desktop: bool = False,
        symlink_appimage_output: bool = False,
    ) -> None:
        self.unresolved = unresolved
        self.storage = storage
        self.fail_deb = fail_deb
        self.omit_extracted_desktop = omit_extracted_desktop
        self.symlink_appimage_output = symlink_appimage_output
        self._appimage_tree: Path | None = None
        self._deb_tree: Path | None = None
        self._appimage_builds = 0
        self._deb_builds = 0
        self.appimage_runtime: bytes | None = None
        self.deb_runtime: bytes | None = None
        self.appimage_sandbox_mode: int | None = None
        self.deb_sandbox_mode: int | None = None
        self.debian_control = ""
        self.corrupt_next_appimage_extract_with_symlink = False

    def can_execute_target(self, architecture: str) -> bool:
        return architecture == "x64"

    def unresolved_libraries(
        self,
        executable: Path,
        library_dir: Path,
    ) -> tuple[str, ...]:
        return self.unresolved

    def build_appimage(
        self,
        tool: Path,
        appdir: Path,
        output: Path,
        architecture: str,
    ) -> None:
        self._appimage_builds += 1
        storage = self.storage or output.parent
        storage.mkdir(parents=True, exist_ok=True)
        self._appimage_tree = storage / f"fake-appimage-tree-{self._appimage_builds}"
        shutil.copytree(appdir, self._appimage_tree)
        runtime = self._appimage_tree / "opt/browseros"
        self.appimage_runtime = (runtime / "browseros").read_bytes()
        self.appimage_sandbox_mode = (
            runtime / "chrome_sandbox"
        ).stat().st_mode & 0o7777
        if self.symlink_appimage_output:
            target = output.parent / "fake-appimage-output"
            target.write_bytes(b"\x7fELFfake-appimage")
            output.symlink_to(target.name)
        else:
            output.write_bytes(b"\x7fELFfake-appimage")

    def extract_appimage(self, package: Path, destination: Path) -> None:
        assert self._appimage_tree is not None
        shutil.copytree(self._appimage_tree, destination)
        if self.corrupt_next_appimage_extract_with_symlink:
            browser = destination / "opt/browseros/browseros"
            browser.unlink()
            browser.symlink_to("chrome_crashpad_handler")
            self.corrupt_next_appimage_extract_with_symlink = False

    def build_deb(self, root: Path, output: Path) -> None:
        if self.fail_deb:
            raise OSError("simulated dpkg-deb failure")
        self._deb_builds += 1
        storage = self.storage or output.parent
        storage.mkdir(parents=True, exist_ok=True)
        self._deb_tree = storage / f"fake-deb-tree-{self._deb_builds}"
        shutil.copytree(root, self._deb_tree)
        runtime = self._deb_tree / "usr/lib/browseros"
        self.deb_runtime = (runtime / "browseros").read_bytes()
        self.deb_sandbox_mode = (runtime / "chrome_sandbox").stat().st_mode & 0o7777
        self.debian_control = (self._deb_tree / "DEBIAN/control").read_text()
        output.write_bytes(b"!<arch>\nfake-deb")

    def deb_fields(self, package: Path, fields: tuple[str, ...]) -> dict[str, str]:
        values = {}
        for line in self.debian_control.splitlines():
            name, separator, value = line.partition(":")
            if separator and name in fields:
                values[name] = value.strip()
        return values

    def extract_deb_data(self, package: Path, destination: Path) -> None:
        assert self._deb_tree is not None
        destination.mkdir(parents=True)
        for source in self._deb_tree.iterdir():
            if source.name == "DEBIAN":
                continue
            target = destination / source.name
            if source.is_dir():
                shutil.copytree(source, target)
            else:
                shutil.copy2(source, target)
        if self.omit_extracted_desktop:
            (destination / "usr/share/applications/browseros.desktop").unlink(
                missing_ok=True
            )

    def extract_deb_control(self, package: Path, destination: Path) -> None:
        assert self._deb_tree is not None
        shutil.copytree(self._deb_tree / "DEBIAN", destination)


class _RealDebFakeAppImageToolchain(_FakeLinuxToolchain):
    """Use real dpkg-deb while retaining a deterministic AppImage executable."""

    def __init__(self) -> None:
        super().__init__()
        self._real = SubprocessLinuxToolchain()

    def can_execute_target(self, architecture: str) -> bool:
        return False

    def build_deb(self, root: Path, output: Path) -> None:
        self._real.build_deb(root, output)

    def deb_fields(self, package: Path, fields: tuple[str, ...]) -> dict[str, str]:
        return dict(self._real.deb_fields(package, fields))

    def extract_deb_data(self, package: Path, destination: Path) -> None:
        self._real.extract_deb_data(package, destination)

    def extract_deb_control(self, package: Path, destination: Path) -> None:
        self._real.extract_deb_control(package, destination)


class LinuxToolchainAdapterTest(unittest.TestCase):
    def test_appimagetool_uses_compressor_supported_by_pinned_tool(self) -> None:
        completed = SimpleNamespace(returncode=0, stdout="", stderr="")
        with (
            mock.patch.dict(os.environ, {}, clear=True),
            mock.patch(
                "bos_build.steps.package.linux_packaging.system.subprocess.run",
                return_value=completed,
            ) as run,
        ):
            SubprocessLinuxToolchain().build_appimage(
                Path("/tools/appimagetool"),
                Path("/staging/AppDir"),
                Path("/dist/BrowserOS.AppImage"),
                "x86_64",
            )

        self.assertEqual(
            run.call_args.args[0],
            (
                "/tools/appimagetool",
                "--comp",
                "zstd",
                "/staging/AppDir",
                "/dist/BrowserOS.AppImage",
            ),
        )
        self.assertEqual(run.call_args.kwargs["env"]["ARCH"], "x86_64")

    def test_ldd_uses_a_stable_locale_and_checks_both_output_streams(self) -> None:
        completed = SimpleNamespace(
            returncode=0,
            stdout="",
            stderr="libmissing.so => not found\n",
        )
        with (
            mock.patch.dict(os.environ, {}, clear=True),
            mock.patch(
                "bos_build.steps.package.linux_packaging.system.subprocess.run",
                return_value=completed,
            ) as run,
        ):
            unresolved = SubprocessLinuxToolchain().unresolved_libraries(
                Path("/runtime/browseros"),
                Path("/runtime"),
            )

        self.assertEqual(unresolved, ("libmissing.so => not found",))
        self.assertEqual(run.call_args.kwargs["env"]["LC_ALL"], "C")
        self.assertEqual(
            run.call_args.kwargs["env"]["LD_LIBRARY_PATH"],
            "/runtime",
        )


class LinuxArtifactResolutionTest(unittest.TestCase):
    def test_expected_artifact_name_cannot_escape_the_distribution_directory(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            ctx = _packaging_context(Path(tmp))
            ctx.get_artifact_name = lambda kind: ".."

            with self.assertRaisesRegex(
                LinuxPackagingError,
                "must be one safe filename",
            ):
                require_linux_artifacts(ctx)

    def test_complete_registered_pair_resolves_in_format_order(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            ctx = _packaging_context(Path(tmp))
            appimage = Path(tmp) / ctx.get_artifact_name("appimage")
            deb = Path(tmp) / ctx.get_artifact_name("deb")
            _write_artifact(appimage)
            _write_artifact(deb)
            ctx.artifact_registry.add("deb", deb)
            ctx.artifact_registry.add("appimage", appimage)

            pair = require_linux_artifacts(ctx)

        self.assertEqual(pair, LinuxArtifactPair(appimage=appimage, deb=deb))
        self.assertEqual(pair.paths, (appimage, deb))

    def test_disk_recovery_rejects_executable_debian_archive(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            ctx = _packaging_context(Path(tmp))
            appimage = Path(tmp) / ctx.get_artifact_name("appimage")
            deb = Path(tmp) / ctx.get_artifact_name("deb")
            _write_artifact(appimage)
            _write_artifact(deb)
            deb.chmod(0o755)

            with self.assertRaisesRegex(
                LinuxPackagingError,
                "DEB must not be executable",
            ):
                require_linux_artifacts(ctx)

    def test_disk_recovery_reports_an_artifact_symlink_without_resolving_it(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            ctx = _packaging_context(Path(tmp))
            appimage = Path(tmp) / ctx.get_artifact_name("appimage")
            deb = Path(tmp) / ctx.get_artifact_name("deb")
            appimage.symlink_to(appimage.name)
            _write_artifact(deb)

            with self.assertRaisesRegex(
                LinuxPackagingError,
                "AppImage is missing or not a regular file",
            ):
                require_linux_artifacts(ctx)

    def test_partial_registry_never_mixes_with_a_complete_disk_pair(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            ctx = _packaging_context(Path(tmp))
            appimage = Path(tmp) / ctx.get_artifact_name("appimage")
            deb = Path(tmp) / ctx.get_artifact_name("deb")
            _write_artifact(appimage)
            _write_artifact(deb)
            ctx.artifact_registry.add("appimage", appimage)

            with self.assertRaisesRegex(
                LinuxPackagingError,
                "refusing disk fallback",
            ):
                require_linux_artifacts(ctx)

    def test_present_registry_keys_must_contain_paths(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            ctx = _packaging_context(Path(tmp))
            ctx.artifact_registry.add("appimage", None)
            ctx.artifact_registry.add("deb", None)

            with self.assertRaisesRegex(
                LinuxPackagingError,
                "registry value 'appimage' must be a Path",
            ):
                require_linux_artifacts(ctx)


class LinuxArtifactBuildTest(unittest.TestCase):
    def test_build_reports_all_missing_required_runtime_inputs(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            ctx = _build_context(Path(tmp))
            out_dir = ctx.chromium_src / ctx.out_dir
            out_dir.mkdir(parents=True)
            (out_dir / ctx.BROWSEROS_APP_NAME).write_bytes(b"browser")

            with self.assertRaises(LinuxPackagingError) as raised:
                build_linux_artifacts(ctx)

        message = str(raised.exception)
        self.assertIn("chrome_crashpad_handler", message)
        self.assertIn("chrome_sandbox", message)
        self.assertIn("libEGL.so", message)
        self.assertIn("browseros_extensions/", message)
        self.assertIn("locales/", message)
        self.assertIn("BrowserOSServer/", message)
        self.assertIn("product icon", message)

    def test_build_rejects_unresolved_same_host_libraries_before_packaging(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            ctx = _build_context(root)
            _write_complete_runtime(ctx)
            toolchain = _FakeLinuxToolchain(
                unresolved=("libmissing.so => not found",),
            )
            with (
                mock.patch(
                    "bos_build.steps.package.linux_packaging.pipeline._TOOLCHAIN",
                    toolchain,
                ),
                self.assertRaisesRegex(
                    LinuxPackagingError,
                    "libmissing.so => not found",
                ),
            ):
                build_linux_artifacts(ctx)

            self.assertEqual(toolchain._appimage_builds, 0)
            self.assertEqual(toolchain._deb_builds, 0)

    def test_build_publishes_two_formats_from_one_verified_runtime(self) -> None:
        tool_bytes = b"deterministic-fake-appimagetool"
        toolchain = _FakeLinuxToolchain()
        downloader = _FakeDownloader(tool_bytes)
        pin = SimpleNamespace(
            version="test",
            filename="appimagetool-x86_64.AppImage",
            url="https://example.test/appimagetool",
            sha256=hashlib.sha256(tool_bytes).hexdigest(),
        )

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            ctx = _build_context(root)
            _write_complete_runtime(ctx)
            cached_tool = root / "build/tools/appimagetool" / pin.version / pin.filename
            cached_tool.parent.mkdir(parents=True)
            cached_tool.write_bytes(b"poisoned-cache")
            with (
                mock.patch(
                    "bos_build.steps.package.linux_packaging.pipeline._TOOLCHAIN",
                    toolchain,
                ),
                mock.patch(
                    "bos_build.steps.package.linux_packaging.appimage._DOWNLOADER",
                    downloader,
                ),
                mock.patch(
                    "bos_build.steps.package.linux_packaging.appimage.get_platform_arch",
                    return_value="x64",
                ),
                mock.patch.dict(
                    "bos_build.steps.package.linux_packaging.appimage._APPIMAGETOOL_PINS",
                    {"x64": pin},
                    clear=True,
                ),
            ):
                pair = build_linux_artifacts(ctx)
                second_pair = build_linux_artifacts(ctx)

            self.assertEqual(
                pair,
                LinuxArtifactPair(
                    appimage=ctx.get_dist_dir() / ctx.get_artifact_name("appimage"),
                    deb=ctx.get_dist_dir() / ctx.get_artifact_name("deb"),
                ),
            )
            self.assertEqual(ctx.artifact_registry.get("appimage"), pair.appimage)
            self.assertEqual(ctx.artifact_registry.get("deb"), pair.deb)
            self.assertEqual(second_pair, pair)
            self.assertEqual(downloader.urls, [pin.url])
            self.assertEqual(cached_tool.read_bytes(), tool_bytes)
            self.assertEqual(toolchain.appimage_runtime, b"contents:browseros")
            self.assertEqual(toolchain.deb_runtime, b"contents:browseros")
            self.assertEqual(toolchain.appimage_sandbox_mode, 0o4755)
            self.assertEqual(toolchain.deb_sandbox_mode, 0o755)
            self.assertIn("Package: browseros", toolchain.debian_control)
            self.assertIn("Architecture: amd64", toolchain.debian_control)
            self.assertTrue(pair.appimage.stat().st_mode & 0o111)
            self.assertFalse(pair.deb.stat().st_mode & 0o111)
            self.assertEqual(list(ctx.get_dist_dir().glob(".linux-package-*")), [])

    def test_debian_failure_preserves_an_older_pair_and_registers_nothing(self) -> None:
        tool_bytes = b"fake-tool"
        toolchain = _FakeLinuxToolchain(fail_deb=True)
        downloader = _FakeDownloader(tool_bytes)
        pin = SimpleNamespace(
            version="test",
            filename="appimagetool-x86_64.AppImage",
            url="https://example.test/appimagetool",
            sha256=hashlib.sha256(tool_bytes).hexdigest(),
        )

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            ctx = _build_context(root)
            _write_complete_runtime(ctx)
            ctx.get_dist_dir().mkdir(parents=True)
            old_appimage = ctx.get_dist_dir() / ctx.get_artifact_name("appimage")
            old_deb = ctx.get_dist_dir() / ctx.get_artifact_name("deb")
            old_appimage.write_bytes(b"old-appimage")
            old_appimage.chmod(0o755)
            old_deb.write_bytes(b"old-deb")
            with (
                mock.patch(
                    "bos_build.steps.package.linux_packaging.pipeline._TOOLCHAIN",
                    toolchain,
                ),
                mock.patch(
                    "bos_build.steps.package.linux_packaging.appimage._DOWNLOADER",
                    downloader,
                ),
                mock.patch(
                    "bos_build.steps.package.linux_packaging.appimage.get_platform_arch",
                    return_value="x64",
                ),
                mock.patch.dict(
                    "bos_build.steps.package.linux_packaging.appimage._APPIMAGETOOL_PINS",
                    {"x64": pin},
                    clear=True,
                ),
                self.assertRaisesRegex(LinuxPackagingError, "deb:.*simulated"),
            ):
                build_linux_artifacts(ctx)

            self.assertEqual(old_appimage.read_bytes(), b"old-appimage")
            self.assertEqual(old_deb.read_bytes(), b"old-deb")
            self.assertIsNone(ctx.artifact_registry.get("appimage"))
            self.assertIsNone(ctx.artifact_registry.get("deb"))
            self.assertEqual(list(ctx.get_dist_dir().glob(".linux-package-*")), [])

    def test_final_validation_failure_restores_an_older_pair(self) -> None:
        tool_bytes = b"fake-tool"
        toolchain = _FakeLinuxToolchain(symlink_appimage_output=True)
        downloader = _FakeDownloader(tool_bytes)
        pin = SimpleNamespace(
            version="test",
            filename="appimagetool-x86_64.AppImage",
            url="https://example.test/appimagetool",
            sha256=hashlib.sha256(tool_bytes).hexdigest(),
        )

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            ctx = _build_context(root)
            _write_complete_runtime(ctx)
            ctx.get_dist_dir().mkdir(parents=True)
            old_appimage = ctx.get_dist_dir() / ctx.get_artifact_name("appimage")
            old_deb = ctx.get_dist_dir() / ctx.get_artifact_name("deb")
            old_appimage.write_bytes(b"old-appimage")
            old_appimage.chmod(0o755)
            old_deb.write_bytes(b"old-deb")
            with (
                mock.patch(
                    "bos_build.steps.package.linux_packaging.pipeline._TOOLCHAIN",
                    toolchain,
                ),
                mock.patch(
                    "bos_build.steps.package.linux_packaging.appimage._DOWNLOADER",
                    downloader,
                ),
                mock.patch(
                    "bos_build.steps.package.linux_packaging.appimage.get_platform_arch",
                    return_value="x64",
                ),
                mock.patch.dict(
                    "bos_build.steps.package.linux_packaging.appimage._APPIMAGETOOL_PINS",
                    {"x64": pin},
                    clear=True,
                ),
                self.assertRaisesRegex(
                    LinuxPackagingError,
                    "AppImage is missing or not a regular file",
                ),
            ):
                build_linux_artifacts(ctx)

            self.assertEqual(old_appimage.read_bytes(), b"old-appimage")
            self.assertEqual(old_deb.read_bytes(), b"old-deb")
            self.assertIsNone(ctx.artifact_registry.get("appimage"))
            self.assertIsNone(ctx.artifact_registry.get("deb"))

    def test_second_publish_failure_rolls_back_both_older_artifacts(self) -> None:
        self._assert_second_publish_interruption_rolls_back(
            OSError("simulated second rename failure"),
            LinuxPackagingError,
        )

    def test_second_publish_cancellation_rolls_back_before_propagating(self) -> None:
        self._assert_second_publish_interruption_rolls_back(
            KeyboardInterrupt("simulated publish cancellation"),
            KeyboardInterrupt,
        )

    def _assert_second_publish_interruption_rolls_back(
        self,
        interruption: BaseException,
        expected_exception: type[BaseException],
    ) -> None:
        tool_bytes = b"fake-tool"
        toolchain = _FakeLinuxToolchain()
        downloader = _FakeDownloader(tool_bytes)
        pin = SimpleNamespace(
            version="test",
            filename="appimagetool-x86_64.AppImage",
            url="https://example.test/appimagetool",
            sha256=hashlib.sha256(tool_bytes).hexdigest(),
        )
        real_replace = os.replace

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            ctx = _build_context(root)
            _write_complete_runtime(ctx)
            ctx.get_dist_dir().mkdir(parents=True)
            old_appimage = ctx.get_dist_dir() / ctx.get_artifact_name("appimage")
            old_deb = ctx.get_dist_dir() / ctx.get_artifact_name("deb")
            old_appimage.write_bytes(b"old-appimage")
            old_appimage.chmod(0o755)
            old_deb.write_bytes(b"old-deb")
            failed = False

            def fail_new_deb_publish(source: object, destination: object) -> None:
                nonlocal failed
                source_path = Path(source)
                destination_path = Path(destination)
                if (
                    not failed
                    and destination_path == old_deb
                    and source_path.parent.name.startswith(".linux-package-")
                ):
                    failed = True
                    raise interruption
                real_replace(source, destination)

            with (
                mock.patch(
                    "bos_build.steps.package.linux_packaging.pipeline._TOOLCHAIN",
                    toolchain,
                ),
                mock.patch(
                    "bos_build.steps.package.linux_packaging.appimage._DOWNLOADER",
                    downloader,
                ),
                mock.patch(
                    "bos_build.steps.package.linux_packaging.appimage.get_platform_arch",
                    return_value="x64",
                ),
                mock.patch.dict(
                    "bos_build.steps.package.linux_packaging.appimage._APPIMAGETOOL_PINS",
                    {"x64": pin},
                    clear=True,
                ),
                mock.patch(
                    "bos_build.steps.package.linux_packaging.pipeline.os.replace",
                    side_effect=fail_new_deb_publish,
                ),
                self.assertRaisesRegex(
                    expected_exception,
                    "simulated",
                ),
            ):
                build_linux_artifacts(ctx)

            self.assertTrue(failed)
            self.assertEqual(old_appimage.read_bytes(), b"old-appimage")
            self.assertEqual(old_deb.read_bytes(), b"old-deb")
            self.assertIsNone(ctx.artifact_registry.get("appimage"))
            self.assertIsNone(ctx.artifact_registry.get("deb"))

    def test_bad_tool_download_never_enters_the_cache_or_dist_directory(self) -> None:
        expected_tool = b"expected-tool"
        downloaded_tool = b"tampered-tool"
        toolchain = _FakeLinuxToolchain()
        downloader = _FakeDownloader(downloaded_tool)
        pin = SimpleNamespace(
            version="test",
            filename="appimagetool-x86_64.AppImage",
            url="https://example.test/appimagetool",
            sha256=hashlib.sha256(expected_tool).hexdigest(),
        )

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            ctx = _build_context(root)
            _write_complete_runtime(ctx)
            with (
                mock.patch(
                    "bos_build.steps.package.linux_packaging.pipeline._TOOLCHAIN",
                    toolchain,
                ),
                mock.patch(
                    "bos_build.steps.package.linux_packaging.appimage._DOWNLOADER",
                    downloader,
                ),
                mock.patch(
                    "bos_build.steps.package.linux_packaging.appimage.get_platform_arch",
                    return_value="x64",
                ),
                mock.patch.dict(
                    "bos_build.steps.package.linux_packaging.appimage._APPIMAGETOOL_PINS",
                    {"x64": pin},
                    clear=True,
                ),
                self.assertRaisesRegex(LinuxPackagingError, "checksum mismatch"),
            ):
                build_linux_artifacts(ctx)

            cache = root / "build/tools/appimagetool/test"
            self.assertFalse((cache / pin.filename).exists())
            self.assertEqual(list(cache.glob("*.part")), [])
            self.assertEqual(
                list(ctx.get_dist_dir().glob("BrowserOS_v1.2.3_*")),
                [],
            )

    def test_unsafe_product_policy_fails_before_external_packaging(self) -> None:
        for bad_name in ("BrowserOS\nInjected", "BrowserOS\tInjected"):
            with (
                self.subTest(bad_name=repr(bad_name)),
                tempfile.TemporaryDirectory() as tmp,
            ):
                root = Path(tmp)
                ctx = _build_context(root)
                ctx.product = replace(ctx.product, display_name=bad_name)
                _write_complete_runtime(ctx)
                toolchain = _FakeLinuxToolchain()
                with (
                    mock.patch(
                        "bos_build.steps.package.linux_packaging.pipeline._TOOLCHAIN",
                        toolchain,
                    ),
                    mock.patch(
                        "bos_build.steps.package.linux_packaging.appimage.acquire_appimagetool",
                        return_value=root / "unused-appimagetool",
                    ),
                    self.assertRaisesRegex(
                        LinuxPackagingError,
                        "must be one line without control characters",
                    ),
                ):
                    build_linux_artifacts(ctx)

                self.assertEqual(toolchain._appimage_builds, 0)
                self.assertIsNone(ctx.artifact_registry.get("appimage"))
                self.assertIsNone(ctx.artifact_registry.get("deb"))

    def test_finished_debian_archive_is_inspected_not_trusted_by_exit_code(
        self,
    ) -> None:
        tool_bytes = b"fake-tool"
        toolchain = _FakeLinuxToolchain(omit_extracted_desktop=True)
        downloader = _FakeDownloader(tool_bytes)
        pin = SimpleNamespace(
            version="test",
            filename="appimagetool-x86_64.AppImage",
            url="https://example.test/appimagetool",
            sha256=hashlib.sha256(tool_bytes).hexdigest(),
        )

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            ctx = _build_context(root)
            _write_complete_runtime(ctx)
            with (
                mock.patch(
                    "bos_build.steps.package.linux_packaging.pipeline._TOOLCHAIN",
                    toolchain,
                ),
                mock.patch(
                    "bos_build.steps.package.linux_packaging.appimage._DOWNLOADER",
                    downloader,
                ),
                mock.patch(
                    "bos_build.steps.package.linux_packaging.appimage.get_platform_arch",
                    return_value="x64",
                ),
                mock.patch.dict(
                    "bos_build.steps.package.linux_packaging.appimage._APPIMAGETOOL_PINS",
                    {"x64": pin},
                    clear=True,
                ),
                self.assertRaisesRegex(
                    LinuxPackagingError,
                    "Debian layout is missing",
                ),
            ):
                build_linux_artifacts(ctx)

            self.assertIsNone(ctx.artifact_registry.get("appimage"))
            self.assertIsNone(ctx.artifact_registry.get("deb"))

    def test_sliced_recovery_fully_inspects_both_exact_packages(self) -> None:
        tool_bytes = b"fake-tool"
        downloader = _FakeDownloader(tool_bytes)
        pin = SimpleNamespace(
            version="test",
            filename="appimagetool-x86_64.AppImage",
            url="https://example.test/appimagetool",
            sha256=hashlib.sha256(tool_bytes).hexdigest(),
        )

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            ctx = _build_context(root)
            _write_complete_runtime(ctx)
            toolchain = _FakeLinuxToolchain(storage=root / "fake-tool-archives")
            with (
                mock.patch(
                    "bos_build.steps.package.linux_packaging.pipeline._TOOLCHAIN",
                    toolchain,
                ),
                mock.patch(
                    "bos_build.steps.package.linux_packaging.appimage._DOWNLOADER",
                    downloader,
                ),
                mock.patch(
                    "bos_build.steps.package.linux_packaging.appimage.get_platform_arch",
                    return_value="x64",
                ),
                mock.patch.dict(
                    "bos_build.steps.package.linux_packaging.appimage._APPIMAGETOOL_PINS",
                    {"x64": pin},
                    clear=True,
                ),
            ):
                built = build_linux_artifacts(ctx)
                ctx.artifact_registry = ArtifactRegistry()
                stale = ctx.get_dist_dir() / "BrowserOS_v1.2.3_stale.AppImage"
                stale.write_bytes(b"stale")
                recovered = require_linux_artifacts(ctx)

            self.assertEqual(recovered, built)
            self.assertEqual(ctx.artifact_registry.get("appimage"), built.appimage)
            self.assertEqual(ctx.artifact_registry.get("deb"), built.deb)

    def test_sliced_recovery_rejects_a_runtime_symlink(self) -> None:
        tool_bytes = b"fake-tool"
        downloader = _FakeDownloader(tool_bytes)
        pin = SimpleNamespace(
            version="test",
            filename="appimagetool-x86_64.AppImage",
            url="https://example.test/appimagetool",
            sha256=hashlib.sha256(tool_bytes).hexdigest(),
        )

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            ctx = _build_context(root)
            _write_complete_runtime(ctx)
            toolchain = _FakeLinuxToolchain(storage=root / "fake-tool-archives")
            with (
                mock.patch(
                    "bos_build.steps.package.linux_packaging.pipeline._TOOLCHAIN",
                    toolchain,
                ),
                mock.patch(
                    "bos_build.steps.package.linux_packaging.appimage._DOWNLOADER",
                    downloader,
                ),
                mock.patch(
                    "bos_build.steps.package.linux_packaging.appimage.get_platform_arch",
                    return_value="x64",
                ),
                mock.patch.dict(
                    "bos_build.steps.package.linux_packaging.appimage._APPIMAGETOOL_PINS",
                    {"x64": pin},
                    clear=True,
                ),
            ):
                build_linux_artifacts(ctx)
                ctx.artifact_registry = ArtifactRegistry()
                toolchain.corrupt_next_appimage_extract_with_symlink = True
                with self.assertRaisesRegex(
                    LinuxPackagingError,
                    "symbolic link",
                ):
                    require_linux_artifacts(ctx)

            self.assertIsNone(ctx.artifact_registry.get("appimage"))
            self.assertIsNone(ctx.artifact_registry.get("deb"))

    @unittest.skipUnless(shutil.which("dpkg-deb"), "dpkg-deb is not installed")
    def test_public_build_interface_produces_a_real_inspectable_debian_archive(
        self,
    ) -> None:
        tool_bytes = b"fake-appimage-tool"
        downloader = _FakeDownloader(tool_bytes)
        toolchain = _RealDebFakeAppImageToolchain()
        pin = SimpleNamespace(
            version="test",
            filename="appimagetool-x86_64.AppImage",
            url="https://example.test/appimagetool",
            sha256=hashlib.sha256(tool_bytes).hexdigest(),
        )

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            ctx = _build_context(root)
            ctx.product = replace(
                ctx.product,
                summary="Agents & privacy",
                description="Use <tools> & browse.",
            )
            _write_complete_runtime(ctx)
            with (
                mock.patch(
                    "bos_build.steps.package.linux_packaging.pipeline._TOOLCHAIN",
                    toolchain,
                ),
                mock.patch(
                    "bos_build.steps.package.linux_packaging.appimage._DOWNLOADER",
                    downloader,
                ),
                mock.patch(
                    "bos_build.steps.package.linux_packaging.appimage.get_platform_arch",
                    return_value="x64",
                ),
                mock.patch.dict(
                    "bos_build.steps.package.linux_packaging.appimage._APPIMAGETOOL_PINS",
                    {"x64": pin},
                    clear=True,
                ),
            ):
                pair = build_linux_artifacts(ctx)

            fields = toolchain._real.deb_fields(
                pair.deb,
                ("Package", "Version", "Architecture"),
            )
            extracted = root / "real-deb-inspection"
            toolchain._real.extract_deb_data(pair.deb, extracted)
            metainfo = (
                extracted / "usr/share/metainfo/browseros.metainfo.xml"
            ).read_text(encoding="utf-8")

        self.assertEqual(
            fields,
            {
                "Package": "browseros",
                "Version": "136.0.0.0.1",
                "Architecture": "amd64",
            },
        )
        self.assertIn("Agents &amp; privacy", metainfo)
        self.assertIn("Use &lt;tools&gt; &amp; browse.", metainfo)


if __name__ == "__main__":
    unittest.main()
