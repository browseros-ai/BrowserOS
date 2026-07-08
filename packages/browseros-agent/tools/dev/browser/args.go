package browser

import (
	"fmt"
	"os"
	"path/filepath"

	"browseros-dev/proc"
)

type ArgsConfig struct {
	Root              string
	Ports             proc.Ports
	UserDataDir       string
	Headless          bool
	LoadDevExtensions bool
	Product           string
}

const (
	ProductBrowserOS   = "browseros"
	ProductBrowserClaw = "browserclaw"

	BrowserOSBinaryPath   = "/Applications/BrowserOS.app/Contents/MacOS/BrowserOS"
	BrowserClawBinaryPath = "/Applications/BrowserClaw.app/Contents/MacOS/BrowserClaw"
)

type BinaryResolution struct {
	Product       string
	Path          string
	PreferredPath string
	Fallback      bool
}

// ResolveBinary chooses the Chromium app binary for a product with an injectable existence check.
func ResolveBinary(product string, exists func(string) bool) BinaryResolution {
	product = normalizeProduct(product)
	resolution := BinaryResolution{
		Product:       product,
		Path:          BrowserOSBinaryPath,
		PreferredPath: BrowserOSBinaryPath,
	}
	if product != ProductBrowserClaw {
		return resolution
	}

	resolution.PreferredPath = BrowserClawBinaryPath
	if exists != nil && exists(BrowserClawBinaryPath) {
		resolution.Path = BrowserClawBinaryPath
		return resolution
	}
	resolution.Fallback = true
	return resolution
}

// ResolveInstalledBinary resolves the product binary against the local macOS app bundle paths.
func ResolveInstalledBinary(product string) BinaryResolution {
	return ResolveBinary(product, binaryExists)
}

// BuildArgs returns the BrowserOS Chromium command for non-WXT dev/test launches.
func BuildArgs(cfg ArgsConfig) []string {
	return buildArgs(cfg, ResolveInstalledBinary)
}

func buildArgs(cfg ArgsConfig, resolveBinary func(string) BinaryResolution) []string {
	product := cfg.Product
	if product == "" {
		product = ProductBrowserOS
	}
	resolution := resolveBinary(product)

	args := []string{resolution.Path}

	if cfg.LoadDevExtensions {
		args = append(args, "--no-first-run", "--no-default-browser-check")
	}

	args = append(args,
		"--use-mock-keychain",
		"--show-component-extension-options",
		"--disable-browseros-server",
		"--browseros-dock-icon=dev",
		fmt.Sprintf("--browseros-product=%s", product),
	)

	if cfg.LoadDevExtensions {
		args = append(args, "--disable-browseros-extensions")
	} else {
		args = append(args, "--enable-logging=stderr")
	}

	if cfg.Headless {
		args = append(args, "--headless=new")
	}

	args = append(args,
		fmt.Sprintf("--remote-debugging-port=%d", cfg.Ports.CDP),
		fmt.Sprintf("--browseros-mcp-port=%d", cfg.Ports.Server),
		fmt.Sprintf("--browseros-extension-port=%d", cfg.Ports.Extension),
		fmt.Sprintf("--user-data-dir=%s", cfg.UserDataDir),
	)

	if cfg.LoadDevExtensions {
		agentExtDir := filepath.Join(cfg.Root, "apps/app/dist/chrome-mv3-dev")
		args = append(args, fmt.Sprintf("--load-extension=%s", agentExtDir))
		args = append(args, "chrome://newtab")
	}

	return args
}

func normalizeProduct(product string) string {
	if product == "" {
		return ProductBrowserOS
	}
	return product
}

func binaryExists(path string) bool {
	_, err := os.Stat(path)
	return err == nil
}
