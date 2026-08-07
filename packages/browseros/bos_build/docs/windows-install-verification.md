# Windows Install Verification Checklist

Use a Windows machine or VM with a clean user profile when verifying a fresh
install. BrowserOS installs per-user; the Apps & Features entry is under:

`HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall`

## Fresh Install

- [ ] Download the Windows installer artifact and double-click it.
- [ ] A progress window appears with the title/text `Installing BrowserOS…`.
- [ ] The installer window closes when installation completes.
- [ ] BrowserOS launches automatically after the fresh install.
- [ ] Apps & Features shows `BrowserOS`.
- [ ] Apps & Features shows publisher `BrowserOS Software Inc`.
- [ ] Apps & Features shows the BrowserOS icon.
- [ ] Apps & Features shows the expected version.
- [ ] A Start Menu shortcut exists and launches BrowserOS.
- [ ] A desktop shortcut exists and launches BrowserOS.

## Update Path

- [ ] Install an older BrowserOS version.
- [ ] Open `chrome://settings/help` and let WinSparkle find the newer update.
- [ ] Start the update from the WinSparkle flow.
- [ ] No installer progress window appears during the update.
- [ ] BrowserOS does not auto-launch as part of the update install.
- [ ] Relaunch BrowserOS after the update completes.
- [ ] `chrome://settings/help` shows the new expected version.
- [ ] `WinSparkle.dll` exists in
      `%LOCALAPPDATA%\BrowserOS\Application\<version>\`.

## Silent Fresh Install

- [ ] Run the installer with `--silent` from Command Prompt or PowerShell.
- [ ] No installer UI appears.
- [ ] BrowserOS is installed successfully.
- [ ] BrowserOS does not auto-launch during the silent install.

## Portable Zip

- [ ] Download the portable zip artifact.
- [ ] Extract the zip.
- [ ] The extracted contents include the Windows installer executable.
