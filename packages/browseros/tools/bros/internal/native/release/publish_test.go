package release

import "testing"

func TestBuildPublishOperations(t *testing.T) {
	metadata := map[string]PlatformMetadata{
		PlatformMacOS: {
			Artifacts: map[string]Artifact{
				"arm64":     {Filename: "BrowserOS-arm64.dmg"},
				"universal": {Filename: "BrowserOS.dmg"},
				"extra":     {Filename: "ignore.zip"},
			},
		},
	}

	ops := BuildPublishOperations("0.31.0", metadata, nil)
	if len(ops) != 2 {
		t.Fatalf("expected 2 publish operations, got %d", len(ops))
	}

	if ops[0].SourceKey != "releases/0.31.0/macos/BrowserOS-arm64.dmg" {
		t.Fatalf("unexpected first source key: %q", ops[0].SourceKey)
	}
	if ops[0].DestinationKey != "download/BrowserOS-arm64.dmg" {
		t.Fatalf("unexpected first destination key: %q", ops[0].DestinationKey)
	}
}
