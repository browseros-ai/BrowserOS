package proc

import (
	"os/exec"
	"path/filepath"
	"strings"
)

// releaseBundleVersionPrefix is the update-feed epoch that the release pipeline stamps in
// front of the BrowserOS version in CFBundleVersion ("10000.0.51.1.3") just before signing.
// Unstamped local builds keep a Chromium-shaped CFBundleVersion instead.
const releaseBundleVersionPrefix = "10000."

// NativeBrowserOSVersion reports the BrowserOS release of the app bundle that contains
// binary, for the sidecar's instance.browseros_version. claw-server gates native features
// (such as profile targeting) on that version, so only the release stamp counts: anything
// else returns "", which the server treats as unknown and offers no gated feature. The code
// signature itself is not verified; like the rest of this launcher, it trusts the local
// install, and a wrong stamp can only over-advertise gated features to that developer.
func NativeBrowserOSVersion(binary string) string {
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
	if !strings.HasPrefix(bundleVersion, releaseBundleVersionPrefix) {
		return ""
	}
	return strings.TrimPrefix(bundleVersion, releaseBundleVersionPrefix)
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
