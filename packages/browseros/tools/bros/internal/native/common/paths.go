package common

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

const BrowserOSPackagesDirEnv = "BROWSEROS_PACKAGES_DIR"

// ResolveBrowserOSPackagesDir finds packages/browseros for native build/release flows.
func ResolveBrowserOSPackagesDir() (string, error) {
	if envPath := strings.TrimSpace(os.Getenv(BrowserOSPackagesDirEnv)); envPath != "" {
		resolved, err := absolutePath(envPath)
		if err != nil {
			return "", err
		}
		if err := validatePackagesDir(resolved); err != nil {
			return "", fmt.Errorf("%s=%q is invalid: %w", BrowserOSPackagesDirEnv, resolved, err)
		}
		return resolved, nil
	}

	cwd, err := os.Getwd()
	if err != nil {
		return "", fmt.Errorf("getting cwd: %w", err)
	}

	for dir := filepath.Clean(cwd); ; dir = filepath.Dir(dir) {
		if err := validatePackagesDir(dir); err == nil {
			return dir, nil
		}
		candidate := filepath.Join(dir, "packages", "browseros")
		if err := validatePackagesDir(candidate); err == nil {
			return candidate, nil
		}

		parent := filepath.Dir(dir)
		if parent == dir {
			break
		}
	}

	return "", fmt.Errorf("could not locate packages/browseros (set %s)", BrowserOSPackagesDirEnv)
}

func validatePackagesDir(dir string) error {
	if dir == "" {
		return fmt.Errorf("path is empty")
	}
	if !isRegularFile(filepath.Join(dir, "CHROMIUM_VERSION")) {
		return fmt.Errorf("missing CHROMIUM_VERSION")
	}
	if !isDirectory(filepath.Join(dir, "chromium_patches")) {
		return fmt.Errorf("missing chromium_patches/")
	}
	if !isDirectory(filepath.Join(dir, "build", "config")) {
		return fmt.Errorf("missing build/config/")
	}
	return nil
}

func absolutePath(p string) (string, error) {
	if filepath.IsAbs(p) {
		return filepath.Clean(p), nil
	}
	cwd, err := os.Getwd()
	if err != nil {
		return "", fmt.Errorf("getting cwd: %w", err)
	}
	return filepath.Clean(filepath.Join(cwd, p)), nil
}

func isRegularFile(path string) bool {
	info, err := os.Stat(path)
	return err == nil && info.Mode().IsRegular()
}

func isDirectory(path string) bool {
	info, err := os.Stat(path)
	return err == nil && info.IsDir()
}
