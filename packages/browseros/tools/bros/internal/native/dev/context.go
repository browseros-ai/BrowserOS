package dev

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"

	nativecommon "bros/internal/native/common"
)

// Context contains resolved paths used by native dev commands.
type Context struct {
	PackagesDir string
	ChromiumSrc string
	Verbose     bool
	Quiet       bool
}

func newContext(opts Options) (*Context, error) {
	chromiumSrc := strings.TrimSpace(opts.ChromiumSrc)
	if chromiumSrc == "" {
		return nil, fmt.Errorf("Chromium source directory not specified (use --chromium-src)")
	}

	absChromiumSrc, err := filepath.Abs(chromiumSrc)
	if err != nil {
		return nil, fmt.Errorf("resolving chromium source: %w", err)
	}
	info, err := os.Stat(absChromiumSrc)
	if err != nil || !info.IsDir() {
		return nil, fmt.Errorf("Chromium source directory does not exist: %s", absChromiumSrc)
	}

	packagesDir, err := nativecommon.ResolveBrowserOSPackagesDir()
	if err != nil {
		return nil, err
	}
	_ = nativecommon.LoadEnv(packagesDir)

	return &Context{
		PackagesDir: packagesDir,
		ChromiumSrc: absChromiumSrc,
		Verbose:     opts.Verbose,
		Quiet:       opts.Quiet,
	}, nil
}

func (c *Context) PatchesDir() string {
	return filepath.Join(c.PackagesDir, "chromium_patches")
}

func (c *Context) FeaturesFile() string {
	return filepath.Join(c.PackagesDir, "build", "features.yaml")
}

func isRegularFile(path string) bool {
	info, err := os.Stat(path)
	return err == nil && info.Mode().IsRegular()
}

func isDirectory(path string) bool {
	info, err := os.Stat(path)
	return err == nil && info.IsDir()
}
