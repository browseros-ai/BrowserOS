package release

import (
	"reflect"
	"testing"
)

func TestParseReleaseJSON(t *testing.T) {
	data := []byte(`{"build_date":"2026-01-01T00:00:00Z","chromium_version":"120.0","artifacts":{"x64":{"filename":"BrowserOS.dmg","url":"https://cdn/browseros.dmg","size":123}}}`)

	parsed, err := ParseReleaseJSON(data)
	if err != nil {
		t.Fatalf("ParseReleaseJSON returned error: %v", err)
	}

	artifact, ok := parsed.Artifacts["x64"]
	if !ok {
		t.Fatal("expected x64 artifact")
	}
	if artifact.Filename != "BrowserOS.dmg" {
		t.Fatalf("unexpected filename: %q", artifact.Filename)
	}
}

func TestFormatSize(t *testing.T) {
	cases := []struct {
		in   int64
		want string
	}{
		{in: 10, want: "10 B"},
		{in: 1024, want: "1 KB"},
		{in: 1024 * 1024, want: "1 MB"},
		{in: 2 * 1024 * 1024 * 1024, want: "2.0 GB"},
	}

	for _, tc := range cases {
		if got := FormatSize(tc.in); got != tc.want {
			t.Fatalf("FormatSize(%d)=%q want %q", tc.in, got, tc.want)
		}
	}
}

func TestNormalizeOSFilter(t *testing.T) {
	cases := map[string]string{
		"":        "",
		"macos":   PlatformMacOS,
		"mac":     PlatformMacOS,
		"windows": PlatformWin,
		"win":     PlatformWin,
		"linux":   PlatformLinux,
	}

	for in, want := range cases {
		got, err := NormalizeOSFilter(in)
		if err != nil {
			t.Fatalf("NormalizeOSFilter(%q) error: %v", in, err)
		}
		if got != want {
			t.Fatalf("NormalizeOSFilter(%q)=%q want %q", in, got, want)
		}
	}

	if _, err := NormalizeOSFilter("bsd"); err == nil {
		t.Fatal("expected invalid os error")
	}
}

func TestBuildDownloadTargets(t *testing.T) {
	metadata := map[string]PlatformMetadata{
		PlatformMacOS: {
			Artifacts: map[string]Artifact{
				"arm64": {Filename: "BrowserOS-arm64.dmg", URL: "https://cdn/a"},
				"x64":   {Filename: "BrowserOS-x64.dmg", URL: "https://cdn/b"},
			},
		},
	}

	targets := BuildDownloadTargets(metadata, []string{PlatformMacOS}, "/tmp/release")
	if len(targets) != 2 {
		t.Fatalf("expected 2 targets, got %d", len(targets))
	}

	filenames := []string{targets[0].Filename, targets[1].Filename}
	want := []string{"BrowserOS-arm64.dmg", "BrowserOS-x64.dmg"}
	if !reflect.DeepEqual(filenames, want) {
		t.Fatalf("unexpected filenames: got %v want %v", filenames, want)
	}
}
