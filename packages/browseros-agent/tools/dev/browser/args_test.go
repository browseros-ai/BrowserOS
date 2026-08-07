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
			got := ResolveBinary(tt.product, func(path string) bool {
				return tt.existingPaths[path]
			})
			if got.Product != tt.wantProduct || got.Path != tt.wantPath || got.PreferredPath != tt.wantPreferred || got.Fallback != tt.wantFallback {
				t.Fatalf("ResolveBinary got %#v", got)
			}
		})
	}
}
