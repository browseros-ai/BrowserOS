package proc

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

func writeBundle(t *testing.T, bundleVersion string) string {
	t.Helper()
	app := filepath.Join(t.TempDir(), "BrowserOS neo.app")
	contents := filepath.Join(app, "Contents")
	if err := os.MkdirAll(filepath.Join(contents, "MacOS"), 0o755); err != nil {
		t.Fatal(err)
	}
	plist := `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>CFBundleVersion</key><string>` + bundleVersion + `</string></dict></plist>`
	if err := os.WriteFile(filepath.Join(contents, "Info.plist"), []byte(plist), 0o644); err != nil {
		t.Fatal(err)
	}
	return filepath.Join(contents, "MacOS", "BrowserOS neo")
}

func TestNativeBrowserOSVersionTrustsOnlyReleaseStamps(t *testing.T) {
	if _, err := exec.LookPath("plutil"); err != nil {
		t.Skip("plutil is only available on macOS")
	}
	cases := map[string]string{
		"10000.0.51.1.3": "0.51.1.3",
		"10000.0.51.1":   "0.51.1",
		"8309.26":        "",
		"":               "",
	}
	for bundleVersion, want := range cases {
		if got := NativeBrowserOSVersion(writeBundle(t, bundleVersion)); got != want {
			t.Fatalf("CFBundleVersion %q: got %q, want %q", bundleVersion, got, want)
		}
	}
	if got := NativeBrowserOSVersion(filepath.Join(t.TempDir(), "not-a-bundle", "chrome")); got != "" {
		t.Fatalf("binary outside a bundle reported %q", got)
	}
	if got := NativeBrowserOSVersion(""); got != "" {
		t.Fatalf("empty binary reported %q", got)
	}
}

func TestWriteSidecarConfigRecordsTheLaunchedBundleVersion(t *testing.T) {
	if _, err := exec.LookPath("plutil"); err != nil {
		t.Skip("plutil is only available on macOS")
	}
	path := filepath.Join(t.TempDir(), "sidecar.json")
	if err := WriteSidecarConfig(path, SidecarConfigOptions{
		Ports:         Ports{CDP: 9000, Server: 9100},
		ResourcesDir:  "/repo/resources",
		ExecutionDir:  "/repo",
		BrowserBinary: writeBundle(t, "10000.0.51.1.3"),
	}); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if want := `"browseros_version": "0.51.1.3"`; !strings.Contains(string(data), want) {
		t.Fatalf("sidecar missing %s:\n%s", want, data)
	}
}
