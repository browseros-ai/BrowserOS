package cmd

import (
	"os/exec"
	"path/filepath"
	"strings"
)

// signedBundleVersionPrefix is the update-feed epoch that release signing stamps in front of
// the BrowserOS version in CFBundleVersion ("10000.0.51.1.3"). Unsigned local builds keep a
// Chromium-shaped CFBundleVersion instead.
const signedBundleVersionPrefix = "10000."

// nativeBrowserOSVersion reports the BrowserOS release of the app bundle that contains
// binary, for the sidecar's instance.browseros_version. claw-server gates native features
// (such as profile targeting) on that version, so only a signed BrowserOS stamp counts:
// anything else returns "", which the server treats as unknown and offers no gated feature.
func nativeBrowserOSVersion(binary string) string {
	plist := bundleInfoPlist(binary)
	if plist == "" {
		return ""
	}
	out, err := exec.Command("plutil", "-extract", "CFBundleVersion", "raw", "-o", "-", plist).Output()
	if err != nil {
		return ""
	}
	return browserOSVersionFromBundleVersion(strings.TrimSpace(string(out)))
}

func browserOSVersionFromBundleVersion(bundleVersion string) string {
	if !strings.HasPrefix(bundleVersion, signedBundleVersionPrefix) {
		return ""
	}
	return strings.TrimPrefix(bundleVersion, signedBundleVersionPrefix)
}

// bundleInfoPlist finds Contents/Info.plist of the nearest enclosing .app directory.
func bundleInfoPlist(binary string) string {
	if binary == "" {
		return ""
	}
	for dir := filepath.Dir(binary); dir != filepath.Dir(dir); dir = filepath.Dir(dir) {
		if strings.HasSuffix(dir, ".app") {
			return filepath.Join(dir, "Contents", "Info.plist")
		}
	}
	return ""
}
