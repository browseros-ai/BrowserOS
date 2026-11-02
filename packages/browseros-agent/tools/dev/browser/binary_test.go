package browser

import "testing"

func TestResolveBinary(t *testing.T) {
	tests := []struct {
		name          string
		existingPaths map[string]bool
		wantPath      string
		wantFallback  bool
	}{
		{
			name:          "uses BrowserOS neo when installed",
			existingPaths: map[string]bool{BrowserClawBinaryPath: true, BrowserOSBinaryPath: true},
			wantPath:      BrowserClawBinaryPath,
		},
		{
			name:         "falls back to BrowserOS when BrowserOS neo is absent",
			wantPath:     BrowserOSBinaryPath,
			wantFallback: true,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := ResolveBinary(func(path string) bool {
				return tt.existingPaths[path]
			})
			if got.Path != tt.wantPath || got.Fallback != tt.wantFallback {
				t.Fatalf("ResolveBinary got %#v", got)
			}
		})
	}
}
