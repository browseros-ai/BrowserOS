package ota

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestParseExistingAppcast(t *testing.T) {
	dir := t.TempDir()
	appcastPath := filepath.Join(dir, "appcast.xml")
	content := `<?xml version="1.0" encoding="utf-8"?>
<rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle" version="2.0">
  <channel>
    <item>
      <sparkle:version>0.0.41</sparkle:version>
      <pubDate>Fri, 16 Jan 2026 23:16:56 +0000</pubDate>
      <enclosure
        url="https://cdn.browseros.com/server/browseros_server_0.0.41_linux_x64.zip"
        sparkle:os="linux"
        sparkle:arch="x86_64"
        sparkle:edSignature="abc"
        length="123"
        type="application/zip"/>
    </item>
  </channel>
</rss>`
	if err := os.WriteFile(appcastPath, []byte(content), 0o644); err != nil {
		t.Fatalf("write appcast: %v", err)
	}

	parsed, err := parseExistingAppcast(appcastPath)
	if err != nil {
		t.Fatalf("parseExistingAppcast error: %v", err)
	}
	if parsed == nil {
		t.Fatalf("expected parsed appcast")
	}
	if parsed.Version != "0.0.41" {
		t.Fatalf("unexpected version: %s", parsed.Version)
	}
	if parsed.PubDate != "Fri, 16 Jan 2026 23:16:56 +0000" {
		t.Fatalf("unexpected pubDate: %s", parsed.PubDate)
	}
	artifact, ok := parsed.Artifacts["linux_x64"]
	if !ok {
		t.Fatalf("expected linux_x64 artifact")
	}
	if artifact.Length != 123 {
		t.Fatalf("unexpected artifact length: %d", artifact.Length)
	}
}

func TestGenerateServerAppcast_MergesSameVersion(t *testing.T) {
	existing := &ExistingAppcast{
		Version: "1.2.3",
		PubDate: "Fri, 16 Jan 2026 23:16:56 +0000",
		Artifacts: map[string]SignedArtifact{
			"linux_x64": {
				Platform:  "linux_x64",
				Signature: "oldsig",
				Length:    100,
				OS:        "linux",
				Arch:      "x86_64",
			},
		},
	}
	newArtifacts := []SignedArtifact{
		{
			Platform:  "darwin_arm64",
			Signature: "newsig",
			Length:    200,
			OS:        "macos",
			Arch:      "arm64",
		},
	}

	xml := generateServerAppcast("1.2.3", newArtifacts, "alpha", existing)
	if !strings.Contains(xml, "<pubDate>Fri, 16 Jan 2026 23:16:56 +0000</pubDate>") {
		t.Fatalf("expected existing pubDate to be preserved")
	}
	if !strings.Contains(xml, "browseros_server_1.2.3_linux_x64.zip") {
		t.Fatalf("expected existing platform to remain in merged appcast")
	}
	if !strings.Contains(xml, "browseros_server_1.2.3_darwin_arm64.zip") {
		t.Fatalf("expected new platform in merged appcast")
	}
}

func TestGenerateServerAppcast_ReplacesOnVersionChange(t *testing.T) {
	existing := &ExistingAppcast{
		Version: "1.2.2",
		PubDate: "Fri, 16 Jan 2026 23:16:56 +0000",
		Artifacts: map[string]SignedArtifact{
			"linux_x64": {
				Platform:  "linux_x64",
				Signature: "oldsig",
				Length:    100,
				OS:        "linux",
				Arch:      "x86_64",
			},
		},
	}
	newArtifacts := []SignedArtifact{
		{
			Platform:  "darwin_arm64",
			Signature: "newsig",
			Length:    200,
			OS:        "macos",
			Arch:      "arm64",
		},
	}

	xml := generateServerAppcast("1.2.3", newArtifacts, "prod", existing)
	if strings.Contains(xml, "browseros_server_1.2.3_linux_x64.zip") {
		t.Fatalf("expected old platform to be removed after version change")
	}
	if !strings.Contains(xml, "browseros_server_1.2.3_darwin_arm64.zip") {
		t.Fatalf("expected new platform in appcast")
	}
	if !strings.Contains(xml, "https://cdn.browseros.com/appcast-server.xml") {
		t.Fatalf("expected production appcast URL")
	}
}
