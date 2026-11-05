package browser

import "os"

const (
	// ProductBrowserClaw is BrowserOS neo's --browseros-product value. Watch passes it to
	// WXT as BROWSEROS_PRODUCT, and the claw-app launcher hands it to Chromium.
	ProductBrowserClaw = "browserclaw"

	BrowserOSBinaryPath   = "/Applications/BrowserOS.app/Contents/MacOS/BrowserOS"
	BrowserClawBinaryPath = "/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo"
)

// BinaryResolution is the Chromium app binary that watch hands to WXT as BROWSEROS_BINARY.
// Fallback marks a machine without BrowserOS neo installed: the BrowserOS app binary then
// runs the neo stack, still launched with --browseros-product=browserclaw.
type BinaryResolution struct {
	Path     string
	Fallback bool
}

// ResolveBinary prefers the BrowserOS neo app and falls back to the BrowserOS app, with an
// injectable existence check.
func ResolveBinary(exists func(string) bool) BinaryResolution {
	if exists != nil && exists(BrowserClawBinaryPath) {
		return BinaryResolution{Path: BrowserClawBinaryPath}
	}
	return BinaryResolution{Path: BrowserOSBinaryPath, Fallback: true}
}

// ResolveInstalledBinary resolves against the local macOS app bundle paths.
func ResolveInstalledBinary() BinaryResolution {
	return ResolveBinary(binaryExists)
}

func binaryExists(path string) bool {
	_, err := os.Stat(path)
	return err == nil
}
