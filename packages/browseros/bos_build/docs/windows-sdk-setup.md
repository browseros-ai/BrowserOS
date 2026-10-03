# Windows SDK setup

Windows release/debug plans run `windows_sdk` after source preparation and before product resources and patches. It also runs for prepared CI checkouts (`--provision none`). Planning with `--show-plan` does not inspect source or install anything.

The selected Chromium tag supplies both SDK directory pins (`build/vs_toolchain.py` and `build/toolchain/win/setup_toolchain.py`) and the package requirement (`docs/windows_build_instructions.md`). For example, SDK package `10.0.28000.2270` installs into directory `10.0.28000.0`. An older source checkout or conflicting requirements fail before installation.

To check an already prepared checkout without building:

```powershell
uv run --project packages/browseros browseros build --modules windows_sdk `
  --chromium-src C:/chromium/src --arch x64 --build-type release
```

Local builds only check. When prerequisites are missing, install the package requested by the pinned Chromium documentation from [Microsoft's SDK downloads](https://learn.microsoft.com/en-us/windows/apps/windows-sdk/downloads), selecting Desktop C++ x86/x64 and Debugging Tools for Windows, then rerun. ARM64 builds additionally need their existing target libraries and debugging tools; automatic installation is currently scoped to the x64 CI lane.

The reusable Windows build explicitly sets `BROWSEROS_INSTALL_WINDOWS_SDK=1`. Installation additionally requires `GITHUB_ACTIONS=true` and an already elevated process. The module never opens a UAC prompt or requests a restart. It resolves the exact package from Microsoft's supported downloads table and validates its Microsoft Authenticode signature before launching the quiet installer. Installation is machine-wide; `WINDOWSSDKDIR`, when set, selects the same root used by Chromium.

Every invocation verifies nonempty headers, libraries, resource/manifest/IDL tools, and debugging DLLs, plus machine registration at or above the required servicing version in the same SDK family. It then runs the pinned Chromium `vs_toolchain.py get_toolchain_dir` and `setup_toolchain.py` for x86/x64 (and ARM64 when requested). The environment-block argument is `none`, so these checks generate no environment files, GN files, or Ninja files.

An installer failure leaves logs in the printed `browseros-windows-sdk-*` temporary directory. Only the transient “another installation is running” result is automatically retried, up to three attempts. Other failures stop the build and can be retried after fixing the reported cause; there is no completion marker that could conceal a partial installation. A successful installer result still requires the file and Chromium environment checks to pass. A reboot-required result is reported, then accepted only if those checks succeed immediately.

Microsoft's [runner image installation script](https://github.com/actions/runner-images/blob/main/images/windows/scripts/build/Install-VisualStudio.ps1) documents the SDK feature selection and quiet/no-restart pattern; [Windows Installer error codes](https://learn.microsoft.com/en-us/windows/win32/msi/error-codes) describe the busy and reboot results. A passing SDK check establishes SDK environment readiness, not a complete Chromium build or release.
