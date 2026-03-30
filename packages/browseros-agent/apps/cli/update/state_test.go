package update

import (
	"path/filepath"
	"testing"
	"time"
)

func TestLoadStateMissing(t *testing.T) {
	configRoot := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", configRoot)

	state, err := LoadState()
	if err != nil {
		t.Fatalf("LoadState() error = %v", err)
	}
	if state == nil {
		t.Fatal("LoadState() returned nil state")
	}
}

func TestSaveStateRoundTrip(t *testing.T) {
	configRoot := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", configRoot)

	want := &State{
		LastCheckedAt:     time.Unix(100, 0).UTC(),
		LatestVersion:     "1.2.3",
		LatestPublishedAt: "2026-03-27T19:00:00Z",
		AssetURL:          "https://cdn.example.com/cli/v1.2.3/browseros-cli.tar.gz",
	}
	if err := SaveState(want); err != nil {
		t.Fatalf("SaveState() error = %v", err)
	}

	got, err := LoadState()
	if err != nil {
		t.Fatalf("LoadState() error = %v", err)
	}
	if got.LatestVersion != want.LatestVersion {
		t.Fatalf("LatestVersion = %q, want %q", got.LatestVersion, want.LatestVersion)
	}
	if StatePath() != filepath.Join(configRoot, "browseros-cli", "update-state.json") {
		t.Fatalf("StatePath() = %q", StatePath())
	}
}

func TestStateIsStale(t *testing.T) {
	now := time.Unix(200, 0).UTC()
	state := &State{LastCheckedAt: time.Unix(0, 0).UTC()}
	if !state.IsStale(now, time.Minute) {
		t.Fatal("IsStale() = false, want true")
	}
}
