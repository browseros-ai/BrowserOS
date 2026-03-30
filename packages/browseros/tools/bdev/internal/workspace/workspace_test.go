package workspace

import (
	"os"
	"path/filepath"
	"testing"
)

func TestConfigRoundTrip(t *testing.T) {
	configHome := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", configHome)

	cfg := &Config{Version: 1, PatchesRepo: "/tmp/browseros"}
	if err := SaveConfig(cfg); err != nil {
		t.Fatalf("SaveConfig: %v", err)
	}

	loaded, err := LoadConfig()
	if err != nil {
		t.Fatalf("LoadConfig: %v", err)
	}
	if loaded.PatchesRepo != cfg.PatchesRepo {
		t.Fatalf("patches repo mismatch: got %q want %q", loaded.PatchesRepo, cfg.PatchesRepo)
	}
}

func TestRegistryDetectsLongestMatchingWorkspace(t *testing.T) {
	configHome := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", configHome)

	root := t.TempDir()
	parent := filepath.Join(root, "chromium")
	child := filepath.Join(parent, "src")
	for _, dir := range []string{parent, child} {
		if err := os.MkdirAll(filepath.Join(dir, ".git"), 0o755); err != nil {
			t.Fatalf("mkdir: %v", err)
		}
	}

	reg := &Registry{Version: 1}
	if _, err := reg.Add("parent", parent); err != nil {
		t.Fatalf("add parent: %v", err)
	}
	if _, err := reg.Add("child", child); err != nil {
		t.Fatalf("add child: %v", err)
	}

	ws, err := Detect(reg, filepath.Join(child, "chrome", "browser"))
	if err != nil {
		t.Fatalf("Detect: %v", err)
	}
	if ws.Name != "child" {
		t.Fatalf("expected child workspace, got %q", ws.Name)
	}
}
