package browser

import "os"

const (
	ProductBrowserClaw = "browserclaw"

	BrowserOSBinaryPath   = "/Applications/BrowserOS.app/Contents/MacOS/BrowserOS"
	BrowserClawBinaryPath = "/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo"
)

type BinaryResolution struct {
	Path     string
	Fallback bool
}

// ResolveBinary chooses the BrowserOS neo app binary, falling back to the BrowserOS app
// when BrowserOS neo is not installed, with an injectable existence check.
func ResolveBinary(exists func(string) bool) BinaryResolution {
	if exists != nil && exists(BrowserClawBinaryPath) {
		return BinaryResolution{Path: BrowserClawBinaryPath}
	}
	return BinaryResolution{Path: BrowserOSBinaryPath, Fallback: true}
}

// ResolveInstalledBinary resolves the app binary against the local macOS app bundle paths.
func ResolveInstalledBinary() BinaryResolution {
	return ResolveBinary(binaryExists)
}

func binaryExists(path string) bool {
	_, err := os.Stat(path)
	return err == nil
}
