package browser

import "testing"

func TestResolveBinary(t *testing.T) {
	tests := []struct {
		name          string
		existingPaths map[string]bool
		want          BinaryResolution
	}{
		{
			name:          "uses BrowserOS neo when installed",
			existingPaths: map[string]bool{BrowserClawBinaryPath: true, BrowserOSBinaryPath: true},
			want:          BinaryResolution{Path: BrowserClawBinaryPath},
		},
		{
			name:          "falls back to BrowserOS when BrowserOS neo is absent",
			existingPaths: map[string]bool{BrowserOSBinaryPath: true},
			want:          BinaryResolution{Path: BrowserOSBinaryPath, Fallback: true},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := ResolveBinary(func(path string) bool {
				return tt.existingPaths[path]
			})
			if got != tt.want {
				t.Fatalf("ResolveBinary got %#v want %#v", got, tt.want)
			}
		})
	}
}
