package proc

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestWriteSidecarConfigWritesChromiumShape(t *testing.T) {
	path := filepath.Join(t.TempDir(), "nested", "sidecar.json")

	if err := WriteSidecarConfig(path, SidecarConfigOptions{
		Ports:        Ports{CDP: 9000, Server: 9100, Extension: 9300},
		ResourcesDir: "/repo/resources",
		ExecutionDir: "/repo",
	}); err != nil {
		t.Fatal(err)
	}

	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	var got map[string]any
	if err := json.Unmarshal(data, &got); err != nil {
		t.Fatal(err)
	}

	ports := got["ports"].(map[string]any)
	if ports["server"] != float64(9100) || ports["cdp"] != float64(9000) || ports["proxy"] != float64(9100) {
		t.Fatalf("unexpected ports: %#v", ports)
	}
	directories := got["directories"].(map[string]any)
	if directories["resources"] != "/repo/resources" || directories["execution"] != "/repo" {
		t.Fatalf("unexpected directories: %#v", directories)
	}
	flags := got["flags"].(map[string]any)
	if flags["allow_remote_in_mcp"] != false {
		t.Fatalf("unexpected flags: %#v", flags)
	}
	if _, ok := got["instance"].(map[string]any); !ok {
		t.Fatalf("missing instance object: %#v", got)
	}
}
