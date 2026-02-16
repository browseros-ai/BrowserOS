package release

import (
	"strings"
	"testing"
)

func TestGenerateAppcastItemUsesSparkleLength(t *testing.T) {
	artifact := Artifact{
		URL:              "https://cdn.browseros.com/a.dmg",
		Size:             100,
		SparkleLength:    200,
		SparkleSignature: "abc123",
	}

	xml := GenerateAppcastItem(artifact, "0.31.0", "120.1", "2026-01-02T03:04:05Z")

	if !strings.Contains(xml, `length="200"`) {
		t.Fatalf("expected sparkle length in xml, got %s", xml)
	}
	if !strings.Contains(xml, "Fri, 02 Jan 2026 03:04:05 +0000") {
		t.Fatalf("expected RFC822 pubDate, got %s", xml)
	}
}

func TestGenerateAppcastSnippetsOrder(t *testing.T) {
	metadata := PlatformMetadata{
		SparkleVersion: "120.1",
		BuildDate:      "2026-01-02T03:04:05Z",
		Artifacts: map[string]Artifact{
			"universal": {URL: "https://cdn/universal", Filename: "u"},
			"arm64":     {URL: "https://cdn/arm", Filename: "a"},
		},
	}

	snippets := GenerateAppcastSnippets("0.31.0", metadata)
	if len(snippets) != 2 {
		t.Fatalf("expected 2 snippets, got %d", len(snippets))
	}
	if snippets[0].Arch != "arm64" {
		t.Fatalf("expected arm64 first, got %q", snippets[0].Arch)
	}
	if snippets[1].Arch != "universal" {
		t.Fatalf("expected universal second, got %q", snippets[1].Arch)
	}
}
