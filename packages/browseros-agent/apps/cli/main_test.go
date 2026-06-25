package main

import "testing"

func TestResolvedVersion(t *testing.T) {
	tests := []struct {
		name      string
		version   string
		timestamp string
		want      string
	}{
		{
			name:      "dev with timestamp",
			version:   "dev",
			timestamp: "260624-1156",
			want:      "dev-260624-1156",
		},
		{
			name:    "dev without timestamp",
			version: "dev",
			want:    "dev",
		},
		{
			name:      "release with timestamp",
			version:   "0.2.0",
			timestamp: "260624-1156",
			want:      "0.2.0",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := resolvedVersion(tt.version, tt.timestamp)
			if got != tt.want {
				t.Fatalf("resolvedVersion(%q, %q) = %q, want %q", tt.version, tt.timestamp, got, tt.want)
			}
		})
	}
}
