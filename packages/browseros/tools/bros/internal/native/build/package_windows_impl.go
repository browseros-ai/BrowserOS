package build

import (
	"archive/zip"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"runtime"
)

func runPackageWindows(ctx *Context) error {
	if runtime.GOOS != "windows" {
		return fmt.Errorf("Windows packaging requires Windows")
	}

	buildOutputDir := filepath.Join(ctx.ChromiumSrc, ctx.OutDir)
	miniInstaller := filepath.Join(buildOutputDir, "mini_installer.exe")
	if !fileExists(miniInstaller) {
		return fmt.Errorf("mini_installer.exe not found: %s", miniInstaller)
	}

	outputDir := ctx.DistDir()
	if err := os.MkdirAll(outputDir, 0o755); err != nil {
		return fmt.Errorf("creating dist dir: %w", err)
	}

	installerName, err := ctx.ArtifactName("installer")
	if err != nil {
		return err
	}
	installerPath := filepath.Join(outputDir, installerName)
	if err := copyFile(miniInstaller, installerPath); err != nil {
		return fmt.Errorf("creating installer: %w", err)
	}

	zipName, err := ctx.ArtifactName("installer_zip")
	if err != nil {
		return err
	}
	zipPath := filepath.Join(outputDir, zipName)
	if err := writeInstallerZip(zipPath, miniInstaller, installerName); err != nil {
		return fmt.Errorf("creating installer zip: %w", err)
	}

	return nil
}

func writeInstallerZip(zipPath, sourceInstallerPath, zipEntryName string) error {
	f, err := os.Create(zipPath)
	if err != nil {
		return err
	}
	defer f.Close()

	zw := zip.NewWriter(f)
	entry, err := zw.Create(zipEntryName)
	if err != nil {
		_ = zw.Close()
		return err
	}

	src, err := os.Open(sourceInstallerPath)
	if err != nil {
		_ = zw.Close()
		return err
	}
	defer src.Close()

	if _, err := io.Copy(entry, src); err != nil {
		_ = zw.Close()
		return err
	}

	if err := zw.Close(); err != nil {
		return err
	}
	return nil
}
