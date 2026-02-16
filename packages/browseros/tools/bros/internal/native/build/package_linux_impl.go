package build

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
)

func runPackageLinux(ctx *Context) error {
	if runtime.GOOS != "linux" {
		return fmt.Errorf("Linux packaging requires Linux")
	}

	browserBinary := ctx.BrowserOSAppPath()
	if !fileExists(browserBinary) {
		return fmt.Errorf("Chrome binary not found: %s", browserBinary)
	}

	packageDir := ctx.DistDir()
	if err := os.MkdirAll(packageDir, 0o755); err != nil {
		return fmt.Errorf("creating dist dir: %w", err)
	}

	appImagePath, appImageErr := packageLinuxAppImage(ctx, packageDir)
	debPath, debErr := packageLinuxDeb(ctx, packageDir)

	if appImagePath == "" && debPath == "" {
		return fmt.Errorf("both AppImage and .deb packaging failed: appimage=%v, deb=%v", appImageErr, debErr)
	}

	return nil
}

func packageLinuxAppImage(ctx *Context, packageDir string) (string, error) {
	appDir := filepath.Join(packageDir, strings.ToLower(appBaseName)+".AppDir")
	_ = os.RemoveAll(appDir)

	if err := prepareLinuxAppDir(ctx, appDir); err != nil {
		_ = os.RemoveAll(appDir)
		return "", err
	}

	filename, err := ctx.ArtifactName("appimage")
	if err != nil {
		_ = os.RemoveAll(appDir)
		return "", err
	}
	outputPath := filepath.Join(packageDir, filename)

	if err := createLinuxAppImage(ctx, appDir, outputPath); err != nil {
		_ = os.RemoveAll(appDir)
		return "", err
	}

	_ = os.RemoveAll(appDir)
	return outputPath, nil
}

func packageLinuxDeb(ctx *Context, packageDir string) (string, error) {
	debDir := filepath.Join(packageDir, strings.ToLower(appBaseName)+"_deb")
	_ = os.RemoveAll(debDir)

	if err := prepareLinuxDebDir(ctx, debDir); err != nil {
		_ = os.RemoveAll(debDir)
		return "", err
	}

	filename, err := ctx.ArtifactName("deb")
	if err != nil {
		_ = os.RemoveAll(debDir)
		return "", err
	}
	outputPath := filepath.Join(packageDir, filename)

	if err := createLinuxDeb(debDir, outputPath); err != nil {
		_ = os.RemoveAll(debDir)
		return "", err
	}

	_ = os.RemoveAll(debDir)
	return outputPath, nil
}

func prepareLinuxAppDir(ctx *Context, appDir string) error {
	appRoot := filepath.Join(appDir, "opt", "browseros")
	usrShare := filepath.Join(appDir, "usr", "share")
	iconsDir := filepath.Join(usrShare, "icons", "hicolor")
	appsDir := filepath.Join(usrShare, "applications")

	if err := copyLinuxBrowserFiles(ctx, appRoot, true); err != nil {
		return err
	}

	desktopPath, err := createLinuxDesktopFile(appsDir, "/opt/browseros/"+strings.ToLower(appBaseName))
	if err != nil {
		return err
	}

	iconSource := filepath.Join(ctx.PackagesDir, "resources", "icons", "product_logo.png")
	_ = copyLinuxIcon(iconSource, iconsDir)

	appDirDesktop := filepath.Join(appDir, "browseros.desktop")
	if err := copyFile(desktopPath, appDirDesktop); err != nil {
		return err
	}
	desktopData, err := os.ReadFile(appDirDesktop)
	if err != nil {
		return err
	}
	updatedDesktop := strings.ReplaceAll(string(desktopData), "Exec=/opt/browseros/"+strings.ToLower(appBaseName)+" %U", "Exec=AppRun %U")
	if err := os.WriteFile(appDirDesktop, []byte(updatedDesktop), 0o644); err != nil {
		return err
	}

	if fileExists(iconSource) {
		if err := copyFile(iconSource, filepath.Join(appDir, "browseros.png")); err != nil {
			return err
		}
	}

	appRunContent := fmt.Sprintf(`#!/bin/sh
THIS="$(readlink -f "${0}")"
HERE="$(dirname "${THIS}")"
export LD_LIBRARY_PATH="${HERE}"/opt/browseros:$LD_LIBRARY_PATH
export CHROME_WRAPPER="${THIS}"
"${HERE}"/opt/browseros/%s "$@"
`, strings.ToLower(appBaseName))
	appRunPath := filepath.Join(appDir, "AppRun")
	if err := os.WriteFile(appRunPath, []byte(appRunContent), 0o755); err != nil {
		return err
	}

	return nil
}

func createLinuxAppImage(ctx *Context, appDir, outputPath string) error {
	toolPath, err := ensureAppImageTool(ctx)
	if err != nil {
		return err
	}

	arch := "x86_64"
	if strings.TrimSpace(ctx.Architecture) == "arm64" {
		arch = "aarch64"
	}

	if err := runCmdWithEnv("", map[string]string{"ARCH": arch}, toolPath, "--comp", "gzip", appDir, outputPath); err != nil {
		return err
	}

	if err := os.Chmod(outputPath, 0o755); err != nil {
		return err
	}

	return nil
}

func ensureAppImageTool(ctx *Context) (string, error) {
	toolDir := filepath.Join(ctx.PackagesDir, "build", "tools")
	if err := os.MkdirAll(toolDir, 0o755); err != nil {
		return "", err
	}

	toolPath := filepath.Join(toolDir, "appimagetool-x86_64.AppImage")
	if fileExists(toolPath) {
		return toolPath, nil
	}

	if err := downloadFile(
		"https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-x86_64.AppImage",
		toolPath,
	); err != nil {
		return "", fmt.Errorf("downloading appimagetool: %w", err)
	}

	if err := os.Chmod(toolPath, 0o755); err != nil {
		return "", err
	}
	return toolPath, nil
}

func prepareLinuxDebDir(ctx *Context, debDir string) error {
	libDir := filepath.Join(debDir, "usr", "lib", "browseros")
	binDir := filepath.Join(debDir, "usr", "bin")
	shareDir := filepath.Join(debDir, "usr", "share")
	appsDir := filepath.Join(shareDir, "applications")
	iconsDir := filepath.Join(shareDir, "icons", "hicolor")
	debianDir := filepath.Join(debDir, "DEBIAN")

	if err := copyLinuxBrowserFiles(ctx, libDir, false); err != nil {
		return err
	}

	if err := createLinuxLauncher(binDir, strings.ToLower(appBaseName)); err != nil {
		return err
	}

	if _, err := createLinuxDesktopFile(appsDir, "/usr/bin/browseros"); err != nil {
		return err
	}

	iconSource := filepath.Join(ctx.PackagesDir, "resources", "icons", "product_logo.png")
	_ = copyLinuxIcon(iconSource, iconsDir)

	if err := createLinuxControlFile(ctx, debianDir); err != nil {
		return err
	}
	if err := createLinuxPostinstFile(debianDir); err != nil {
		return err
	}

	return nil
}

func createLinuxDeb(debDir, outputPath string) error {
	if _, err := exec.LookPath("dpkg-deb"); err != nil {
		return fmt.Errorf("dpkg-deb not found")
	}

	if err := runCmd("", "dpkg-deb", "--build", "--root-owner-group", debDir, outputPath); err != nil {
		return fmt.Errorf("building deb: %w", err)
	}

	if err := os.Chmod(outputPath, 0o644); err != nil {
		return err
	}
	return nil
}

func copyLinuxBrowserFiles(ctx *Context, targetDir string, setSandboxSUID bool) error {
	if err := os.MkdirAll(targetDir, 0o755); err != nil {
		return err
	}

	outDir := filepath.Join(ctx.ChromiumSrc, ctx.OutDir)
	filesToCopy := []string{
		strings.ToLower(appBaseName),
		"chrome_crashpad_handler",
		"chrome_sandbox",
		"chromedriver",
		"libEGL.so",
		"libGLESv2.so",
		"libvk_swiftshader.so",
		"libvulkan.so.1",
		"vk_swiftshader_icd.json",
		"icudtl.dat",
		"snapshot_blob.bin",
		"v8_context_snapshot.bin",
		"chrome_100_percent.pak",
		"chrome_200_percent.pak",
		"resources.pak",
	}
	for _, name := range filesToCopy {
		src := filepath.Join(outDir, name)
		if !fileExists(src) {
			continue
		}
		if err := copyFile(src, filepath.Join(targetDir, name)); err != nil {
			return err
		}
	}

	dirsToCopy := []string{"locales", "MEIPreload", "BrowserOSServer"}
	for _, dirName := range dirsToCopy {
		src := filepath.Join(outDir, dirName)
		if !dirExists(src) {
			continue
		}
		if err := copyDir(src, filepath.Join(targetDir, dirName)); err != nil {
			return err
		}
	}

	browserPath := filepath.Join(targetDir, strings.ToLower(appBaseName))
	if fileExists(browserPath) {
		_ = os.Chmod(browserPath, 0o755)
	}

	sandboxPath := filepath.Join(targetDir, "chrome_sandbox")
	if fileExists(sandboxPath) {
		if setSandboxSUID {
			_ = os.Chmod(sandboxPath, 0o4755)
		} else {
			_ = os.Chmod(sandboxPath, 0o755)
		}
	}

	crashpadPath := filepath.Join(targetDir, "chrome_crashpad_handler")
	if fileExists(crashpadPath) {
		_ = os.Chmod(crashpadPath, 0o755)
	}

	return nil
}

func createLinuxDesktopFile(appsDir, execPath string) (string, error) {
	if err := os.MkdirAll(appsDir, 0o755); err != nil {
		return "", err
	}
	desktopContent := fmt.Sprintf(`[Desktop Entry]
Version=1.0
Name=BrowserOS
GenericName=Web Browser
Comment=Browse the World Wide Web
Exec=%s %%U
Terminal=false
Type=Application
Categories=Network;WebBrowser;
MimeType=text/html;text/xml;application/xhtml+xml;application/xml;application/vnd.mozilla.xul+xml;application/rss+xml;application/rdf+xml;image/gif;image/jpeg;image/png;x-scheme-handler/http;x-scheme-handler/https;x-scheme-handler/ftp;x-scheme-handler/chrome;video/webm;application/x-xpinstall;
Icon=browseros
StartupWMClass=chromium-browser
`, execPath)
	path := filepath.Join(appsDir, "browseros.desktop")
	if err := os.WriteFile(path, []byte(desktopContent), 0o644); err != nil {
		return "", err
	}
	return path, nil
}

func copyLinuxIcon(iconSource, iconsDir string) error {
	if !fileExists(iconSource) {
		return nil
	}
	dest := filepath.Join(iconsDir, "256x256", "apps", "browseros.png")
	if err := os.MkdirAll(filepath.Dir(dest), 0o755); err != nil {
		return err
	}
	return copyFile(iconSource, dest)
}

func createLinuxLauncher(binDir, appBinary string) error {
	if err := os.MkdirAll(binDir, 0o755); err != nil {
		return err
	}
	content := fmt.Sprintf(`#!/bin/sh
# BrowserOS launcher script
export LD_LIBRARY_PATH=/usr/lib/browseros:$LD_LIBRARY_PATH
exec /usr/lib/browseros/%s "$@"
`, appBinary)
	path := filepath.Join(binDir, "browseros")
	if err := os.WriteFile(path, []byte(content), 0o755); err != nil {
		return err
	}
	return nil
}

func createLinuxControlFile(ctx *Context, debianDir string) error {
	if err := os.MkdirAll(debianDir, 0o755); err != nil {
		return err
	}
	version := strings.TrimSpace(ctx.BrowserOSChromiumVersion)
	version = strings.TrimPrefix(version, "v")
	version = strings.ReplaceAll(version, " ", "")
	version = strings.ReplaceAll(version, "_", ".")
	if version == "" {
		version = strings.TrimSpace(ctx.SemanticVersion)
	}
	if version == "" {
		version = "0.0.0"
	}

	debArch := "arm64"
	if strings.TrimSpace(ctx.Architecture) == "x64" {
		debArch = "amd64"
	}

	content := fmt.Sprintf(`Package: browseros
Version: %s
Section: web
Priority: optional
Architecture: %s
Depends: libc6 (>= 2.31), libglib2.0-0, libnss3, libnspr4, libx11-6, libatk1.0-0, libatk-bridge2.0-0, libcups2, libasound2, libdrm2, libgbm1, libpango-1.0-0, libcairo2, libudev1, libxcomposite1, libxdamage1, libxrandr2, libxkbcommon0, libgtk-3-0
Maintainer: BrowserOS Team <support@browseros.com>
Homepage: https://www.browseros.com/
Description: BrowserOS - The open source agentic browser
 BrowserOS is a privacy-focused web browser built on Chromium,
 designed for modern web browsing with AI capabilities.
`, version, debArch)
	return os.WriteFile(filepath.Join(debianDir, "control"), []byte(content), 0o644)
}

func createLinuxPostinstFile(debianDir string) error {
	content := `#!/bin/sh
# Post-installation script for BrowserOS
set -e

# Set SUID bit on chrome_sandbox for sandboxing support
if [ -f /usr/lib/browseros/chrome_sandbox ]; then
    chmod 4755 /usr/lib/browseros/chrome_sandbox
fi

exit 0
`
	path := filepath.Join(debianDir, "postinst")
	if err := os.WriteFile(path, []byte(content), 0o755); err != nil {
		return err
	}
	return nil
}
