package release

import (
	"strings"
	"testing"
)

func TestParseGitHubRepo(t *testing.T) {
	cases := []struct {
		remote string
		repo   string
		ok     bool
	}{
		{remote: "git@github.com:browseros/browseros.git", repo: "browseros/browseros", ok: true},
		{remote: "https://github.com/browseros/browseros.git", repo: "browseros/browseros", ok: true},
		{remote: "https://gitlab.com/browseros/browseros.git", ok: false},
	}

	for _, tc := range cases {
		repo, ok := ParseGitHubRepo(tc.remote)
		if ok != tc.ok || repo != tc.repo {
			t.Fatalf("ParseGitHubRepo(%q)=(%q,%v) want (%q,%v)", tc.remote, repo, ok, tc.repo, tc.ok)
		}
	}
}

func TestNormalizeVersion(t *testing.T) {
	if got := NormalizeVersion("0.31.0.4"); got != "0.31.0" {
		t.Fatalf("NormalizeVersion returned %q", got)
	}
	if got := NormalizeVersion("0.31"); got != "0.31" {
		t.Fatalf("NormalizeVersion returned %q", got)
	}
}

func TestGenerateReleaseNotes(t *testing.T) {
	metadata := map[string]PlatformMetadata{
		PlatformMacOS: {
			ChromiumVersion: "120.0.1.2",
			Artifacts: map[string]Artifact{
				"arm64": {Filename: "BrowserOS-arm64.dmg", URL: "https://cdn/arm"},
			},
		},
	}

	notes := GenerateReleaseNotes("0.31.0", metadata)
	if !strings.Contains(notes, "Chromium version: 120.0.1.2") {
		t.Fatalf("expected chromium version in notes: %s", notes)
	}
	if !strings.Contains(notes, "[BrowserOS-arm64.dmg](https://cdn/arm)") {
		t.Fatalf("expected artifact link in notes: %s", notes)
	}
}
