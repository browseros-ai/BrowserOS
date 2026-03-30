package analytics

import (
	"encoding/json"
	"os"
	"path/filepath"
	"regexp"
	"testing"
	"time"
)

func TestGenerateUUID(t *testing.T) {
	id := generateUUID()
	uuidRe := regexp.MustCompile(`^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$`)
	if !uuidRe.MatchString(id) {
		t.Errorf("generateUUID() = %q, does not match UUID v4 pattern", id)
	}

	id2 := generateUUID()
	if id == id2 {
		t.Error("generateUUID() returned the same value twice")
	}
}

func TestLoadBrowserosID(t *testing.T) {
	tmp := t.TempDir()
	t.Setenv("HOME", tmp)

	// No server.json → empty
	if got := loadBrowserosID(); got != "" {
		t.Errorf("loadBrowserosID() = %q, want empty", got)
	}

	// server.json without browseros_id → empty
	dir := filepath.Join(tmp, ".browseros")
	os.MkdirAll(dir, 0755)
	data, _ := json.Marshal(map[string]any{"server_port": 9100, "url": "http://127.0.0.1:9100"})
	os.WriteFile(filepath.Join(dir, "server.json"), data, 0644)

	if got := loadBrowserosID(); got != "" {
		t.Errorf("loadBrowserosID() = %q, want empty (no browseros_id field)", got)
	}

	// server.json with browseros_id → returns it
	data, _ = json.Marshal(map[string]any{
		"server_port":  9100,
		"url":          "http://127.0.0.1:9100",
		"browseros_id": "test-uuid-1234",
	})
	os.WriteFile(filepath.Join(dir, "server.json"), data, 0644)

	if got := loadBrowserosID(); got != "test-uuid-1234" {
		t.Errorf("loadBrowserosID() = %q, want %q", got, "test-uuid-1234")
	}
}

func TestLoadOrCreateInstallID(t *testing.T) {
	tmp := t.TempDir()
	configDir := filepath.Join(tmp, "browseros-cli")
	t.Setenv("XDG_CONFIG_HOME", tmp)

	// First call creates the file
	id := loadOrCreateInstallID()
	if id == "" {
		t.Fatal("loadOrCreateInstallID() returned empty string")
	}

	// File was persisted
	data, err := os.ReadFile(filepath.Join(configDir, "install_id"))
	if err != nil {
		t.Fatalf("install_id file not created: %v", err)
	}
	if string(data) != id {
		t.Errorf("persisted id = %q, want %q", string(data), id)
	}

	// Second call returns the same ID
	id2 := loadOrCreateInstallID()
	if id2 != id {
		t.Errorf("loadOrCreateInstallID() = %q, want stable %q", id2, id)
	}
}

func TestResolveDistinctID_PrefersBrowserosID(t *testing.T) {
	tmp := t.TempDir()
	t.Setenv("HOME", tmp)
	t.Setenv("XDG_CONFIG_HOME", tmp)

	// Write server.json with browseros_id
	dir := filepath.Join(tmp, ".browseros")
	os.MkdirAll(dir, 0755)
	data, _ := json.Marshal(map[string]any{"browseros_id": "server-uuid"})
	os.WriteFile(filepath.Join(dir, "server.json"), data, 0644)

	got := resolveDistinctID()
	if got != "server-uuid" {
		t.Errorf("resolveDistinctID() = %q, want %q", got, "server-uuid")
	}
}

func TestResolveDistinctID_FallsBackToInstallID(t *testing.T) {
	tmp := t.TempDir()
	t.Setenv("HOME", tmp)
	t.Setenv("XDG_CONFIG_HOME", tmp)

	// No server.json → should generate install_id
	got := resolveDistinctID()
	if got == "" {
		t.Error("resolveDistinctID() returned empty string")
	}
}

func TestInitNoopsWithoutAPIKey(t *testing.T) {
	old := posthogAPIKey
	posthogAPIKey = ""
	defer func() { posthogAPIKey = old }()

	Init("1.0.0")
	if svc != nil {
		t.Error("Init() created service without API key")
	}
}

func TestTrackAndCloseNoopWithoutInit(t *testing.T) {
	old := svc
	svc = nil
	defer func() { svc = old }()

	// Should not panic
	Track("test", true, time.Second)
	Close()
}
