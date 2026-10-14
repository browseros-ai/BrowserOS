package cmd

import (
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"testing"

	"browseros-dogfood/config"
)

func writeStampedBundle(t *testing.T, bundleVersion string) string {
	t.Helper()
	contents := filepath.Join(t.TempDir(), "BrowserOS neo.app", "Contents")
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

func TestDogfoodSidecarRecordsOnlyReleaseStampedVersions(t *testing.T) {
	if _, err := exec.LookPath("plutil"); err != nil {
		t.Skip("plutil is only available on macOS")
	}
	for bundleVersion, want := range map[string]string{
		"10000.0.51.1.3": "0.51.1.3",
		"8309.26":        "",
	} {
		cfg := config.Config{
			DevUserDataDir:   t.TempDir(),
			BrowserOSDir:     "/tmp/browseros-dogfood",
			BrowserOSAppPath: writeStampedBundle(t, bundleVersion),
			Ports:            config.Ports{CDP: 9015, Server: 9115, Extension: 9315},
		}
		path := dogfoodSidecarConfigPath(cfg)
		if err := writeDogfoodSidecarConfig(path, cfg, "/repo/resources"); err != nil {
			t.Fatal(err)
		}
		data, err := os.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		var got struct {
			Instance struct {
				BrowserOSVersion string `json:"browseros_version"`
			} `json:"instance"`
		}
		if err := json.Unmarshal(data, &got); err != nil {
			t.Fatal(err)
		}
		if got.Instance.BrowserOSVersion != want {
			t.Fatalf("CFBundleVersion %q: got %q, want %q", bundleVersion, got.Instance.BrowserOSVersion, want)
		}
	}
}
