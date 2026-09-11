package browser

import (
	"strings"
	"testing"

	"browseros-dev/proc"
)

func TestBuildArgsUsesDevDockIcon(t *testing.T) {
	args := BuildArgs(ArgsConfig{
		Root:              "/repo/packages/browseros-agent",
		Ports:             proc.Ports{CDP: 9005, Server: 9105, Extension: 9305},
		UserDataDir:       "/tmp/browseros-dev",
		LoadDevExtensions: true,
	})
	joined := strings.Join(args, "\n")
	if !strings.Contains(joined, "--browseros-dock-icon=dev") {
		t.Fatalf("missing dev dock icon arg in\n%s", joined)
	}
}

func TestBuildArgsUsesProductFlag(t *testing.T) {
	args := buildArgs(ArgsConfig{
		Root:              "/repo/packages/browseros-agent",
		Ports:             proc.Ports{CDP: 9005, Server: 9105, Extension: 9305},
		UserDataDir:       "/tmp/browseros-dev",
		LoadDevExtensions: true,
		Product:           ProductBrowserClaw,
	}, func(product string) BinaryResolution {
		return BinaryResolution{Product: product, Path: BrowserClawBinaryPath, PreferredPath: BrowserClawBinaryPath}
	})
	joined := strings.Join(args, "\n")
	if args[0] != BrowserClawBinaryPath {
		t.Fatalf("got binary %q want %q", args[0], BrowserClawBinaryPath)
	}
	if !strings.Contains(joined, "--browseros-product=browserclaw") {
		t.Fatalf("missing BrowserClaw product arg in\n%s", joined)
	}
}

func TestBuildArgsSkipsMockKeychainOutsideMacOS(t *testing.T) {
	build := func(goos string) string {
		args := buildArgsForGOOS(ArgsConfig{
			Root:        "/repo/packages/browseros-agent",
			Ports:       proc.Ports{CDP: 9005, Server: 9105, Extension: 9305},
			UserDataDir: "/tmp/browseros-dev",
		}, goos, func(product string) BinaryResolution {
			return BinaryResolution{Product: product, Path: "/browser", PreferredPath: "/browser"}
		})
		return strings.Join(args, "\n")
	}

	if !strings.Contains(build("darwin"), "--use-mock-keychain") {
		t.Fatal("expected --use-mock-keychain on darwin")
	}
	if strings.Contains(build("linux"), "--use-mock-keychain") {
		t.Fatal("did not expect --use-mock-keychain on linux")
	}
	if strings.Contains(build("windows"), "--use-mock-keychain") {
		t.Fatal("did not expect --use-mock-keychain on windows")
	}
}

func TestResolveBinary(t *testing.T) {
	tests := []struct {
		name          string
		product       string
		existingPaths map[string]bool
		wantProduct   string
		wantPath      string
		wantPreferred string
		wantFallback  bool
	}{
		{
			name:          "default product uses BrowserOS",
			wantProduct:   ProductBrowserOS,
			wantPath:      BrowserOSBinaryPath,
			wantPreferred: BrowserOSBinaryPath,
		},
		{
			name:          "BrowserOS product ignores BrowserClaw install",
			product:       ProductBrowserOS,
			existingPaths: map[string]bool{BrowserClawBinaryPath: true},
			wantProduct:   ProductBrowserOS,
			wantPath:      BrowserOSBinaryPath,
			wantPreferred: BrowserOSBinaryPath,
		},
		{
			name:          "BrowserClaw product uses BrowserClaw when installed",
			product:       ProductBrowserClaw,
			existingPaths: map[string]bool{BrowserClawBinaryPath: true},
			wantProduct:   ProductBrowserClaw,
			wantPath:      BrowserClawBinaryPath,
			wantPreferred: BrowserClawBinaryPath,
		},
		{
			name:          "BrowserClaw product falls back to BrowserOS when absent",
			product:       ProductBrowserClaw,
			wantProduct:   ProductBrowserClaw,
			wantPath:      BrowserOSBinaryPath,
			wantPreferred: BrowserClawBinaryPath,
			wantFallback:  true,
		},
		{
			name:          "unknown product keeps product flag but uses BrowserOS",
			product:       "custom",
			wantProduct:   "custom",
			wantPath:      BrowserOSBinaryPath,
			wantPreferred: BrowserOSBinaryPath,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := resolveBinaryFor(tt.product, "darwin", func(path string) bool {
				return tt.existingPaths[path]
			})
			if got.Product != tt.wantProduct || got.Path != tt.wantPath || got.PreferredPath != tt.wantPreferred || got.Fallback != tt.wantFallback {
				t.Fatalf("ResolveBinary got %#v", got)
			}
		})
	}
}

func TestResolveBinaryOnLinuxPrefersDebianLayout(t *testing.T) {
	got := resolveBinaryFor(ProductBrowserClaw, "linux", func(path string) bool {
		return path == "/opt/browserclaw/browserclaw"
	})
	if got.Product != ProductBrowserClaw {
		t.Fatalf("expected browserclaw product, got %q", got.Product)
	}
	if got.PreferredPath != "/usr/lib/browserclaw/browserclaw" {
		t.Fatalf("expected Debian layout preferred path, got %q", got.PreferredPath)
	}
	if got.Path != "/opt/browserclaw/browserclaw" {
		t.Fatalf("expected AppImage-layout install to win, got %q", got.Path)
	}
	if got.Fallback {
		t.Fatal("expected a found neo install not to be a fallback")
	}
}

func TestResolveBinaryOnLinuxFallsBackToBrowserOS(t *testing.T) {
	got := resolveBinaryFor(ProductBrowserClaw, "linux", func(path string) bool {
		return path == "/usr/lib/browseros/browseros"
	})
	if got.Product != ProductBrowserClaw {
		t.Fatalf("expected browserclaw product, got %q", got.Product)
	}
	if got.Path != "/usr/lib/browseros/browseros" {
		t.Fatalf("expected BrowserOS fallback on Linux, got %q", got.Path)
	}
	if got.PreferredPath != "/usr/lib/browserclaw/browserclaw" {
		t.Fatalf("expected neo preferred path, got %q", got.PreferredPath)
	}
	if !got.Fallback {
		t.Fatal("expected BrowserOS reuse to be flagged as fallback")
	}
}

func TestResolveBinaryOnLinuxFallsBackToBrowserOSAppImage(t *testing.T) {
	got := resolveBinaryFor(ProductBrowserClaw, "linux", func(path string) bool {
		return path == "/opt/browseros/browseros"
	})
	if got.Path != "/opt/browseros/browseros" {
		t.Fatalf("expected BrowserOS AppImage fallback on Linux, got %q", got.Path)
	}
	if !got.Fallback {
		t.Fatal("expected BrowserOS reuse to be flagged as fallback")
	}
}

func TestResolveBinaryOnLinuxDefaultsToDebianPathWhenNothingInstalled(t *testing.T) {
	got := resolveBinaryFor(ProductBrowserOS, "linux", func(string) bool { return false })
	if got.Path != "/usr/lib/browseros/browseros" {
		t.Fatalf("expected Debian layout default, got %q", got.Path)
	}
	if got.PreferredPath != got.Path || got.Fallback {
		t.Fatalf("expected unchanged preferred path without fallback, got %#v", got)
	}

	claw := resolveBinaryFor(ProductBrowserClaw, "linux", func(string) bool { return false })
	if claw.Path != "/usr/lib/browseros/browseros" {
		t.Fatalf("expected neo to default to the BrowserOS path like macOS, got %q", claw.Path)
	}
	if claw.PreferredPath != "/usr/lib/browserclaw/browserclaw" || !claw.Fallback {
		t.Fatalf("expected neo preferred path with fallback set, got %#v", claw)
	}
}
