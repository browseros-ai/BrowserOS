package browser

import (
	"fmt"
	"os"
	"path/filepath"
	"runtime"

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

	// macOS app-bundle binary paths. These double as the canonical install
	// locations documented for each product; Linux resolves its own layout in
	// linuxBinaryPaths below.
	BrowserOSBinaryPath   = "/Applications/BrowserOS.app/Contents/MacOS/BrowserOS"
	BrowserClawBinaryPath = "/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo"
)

// linuxBinaryPaths lists candidate Chromium binaries per product on Linux,
// mirroring the Debian (lib_dir) and AppImage (appimage_dir) layouts produced
// by bos_build's linux_packaging for each product id.
func linuxBinaryPaths(product string) []string {
	if product == ProductBrowserClaw {
		return []string{
			"/usr/lib/browserclaw/browserclaw",
			"/opt/browserclaw/browserclaw",
		}
	}
	return []string{
		"/usr/lib/browseros/browseros",
		"/opt/browseros/browseros",
	}
}

// EnvBinaryPath returns the developer override for the browser binary, if set.
// BROWSEROS_BINARY already flows through WXT (web-ext.config.ts) and the test
// runtime, so the supervisor honors the same variable.
func EnvBinaryPath() string {
	return os.Getenv("BROWSEROS_BINARY")
}

type BinaryResolution struct {
	Product       string
	Path          string
	PreferredPath string
	Fallback      bool
}

// ResolveBinary chooses the Chromium app binary for a product with an injectable existence check.
func ResolveBinary(product string, exists func(string) bool) BinaryResolution {
	return resolveBinaryFor(product, runtime.GOOS, exists)
}

// resolveBinaryFor is ResolveBinary with an injectable GOOS so tests can cover
// every platform from one host.
func resolveBinaryFor(product string, goos string, exists func(string) bool) BinaryResolution {
	product = normalizeProduct(product)
	if goos == "linux" {
		candidates := linuxBinaryPaths(product)
		for _, candidate := range candidates {
			if exists != nil && exists(candidate) {
				return BinaryResolution{
					Product:       product,
					Path:          candidate,
					PreferredPath: candidates[0],
				}
			}
		}
		if product != ProductBrowserClaw {
			return BinaryResolution{
				Product:       product,
				Path:          candidates[0],
				PreferredPath: candidates[0],
			}
		}
		// Mirror the macOS fallback: neo reuses an installed BrowserOS build
		// when the neo install is absent.
		return BinaryResolution{
			Product:       product,
			Path:          linuxBinaryPaths(ProductBrowserOS)[0],
			PreferredPath: candidates[0],
			Fallback:      true,
		}
	}

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

// ResolveInstalledBinary resolves the product binary against the local
// platform's install paths. BROWSEROS_BINARY wins over everything.
func ResolveInstalledBinary(product string) BinaryResolution {
	if env := EnvBinaryPath(); env != "" {
		return BinaryResolution{
			Product:       normalizeProduct(product),
			Path:          env,
			PreferredPath: env,
		}
	}
	return ResolveBinary(product, binaryExists)
}

// BuildArgs returns the BrowserOS Chromium command for non-WXT dev/test launches.
func BuildArgs(cfg ArgsConfig) []string {
	return buildArgsForGOOS(cfg, runtime.GOOS, ResolveInstalledBinary)
}

func buildArgs(cfg ArgsConfig, resolveBinary func(string) BinaryResolution) []string {
	return buildArgsForGOOS(cfg, runtime.GOOS, resolveBinary)
}

func buildArgsForGOOS(cfg ArgsConfig, goos string, resolveBinary func(string) BinaryResolution) []string {
	product := cfg.Product
	if product == "" {
		product = ProductBrowserOS
	}
	resolution := resolveBinary(product)

	args := []string{resolution.Path}

	if cfg.LoadDevExtensions {
		args = append(args, "--no-first-run", "--no-default-browser-check")
	}

	// --use-mock-keychain is a macOS keychain-testing flag; keep it off other
	// platforms so dev launches stay identical to documented Chromium ones.
	if goos == "darwin" {
		args = append(args, "--use-mock-keychain")
	}

	args = append(args,
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
