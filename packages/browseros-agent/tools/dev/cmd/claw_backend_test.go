package cmd

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"slices"
	"sync/atomic"
	"testing"
	"time"

	"browseros-dev/proc"
)

func TestSelectClawBackend(t *testing.T) {
	for _, tc := range []struct {
		value       string
		wantPackage string
		wantError   string
	}{
		{value: "", wantPackage: "claw-server"},
		{value: " ", wantPackage: "claw-server"},
		{value: "1", wantPackage: "neo-server"},
		{value: "true", wantError: `BROWSERCLAW_USE_SDK must be "1" or unset, got "true"`},
		{value: "0", wantError: `BROWSERCLAW_USE_SDK must be "1" or unset, got "0"`},
		{value: " 1 ", wantError: `BROWSERCLAW_USE_SDK must be "1" or unset, got " 1 "`},
	} {
		t.Run(tc.value, func(t *testing.T) {
			backend, err := selectClawBackend(tc.value)
			if tc.wantError != "" {
				if err == nil || err.Error() != tc.wantError {
					t.Fatalf("selectClawBackend(%q) error = %v, want %q", tc.value, err, tc.wantError)
				}
				return
			}
			if err != nil || backend.Package != tc.wantPackage {
				t.Fatalf("selectClawBackend(%q) = %q, %v; want %q", tc.value, backend.Package, err, tc.wantPackage)
			}
		})
	}
}

func TestSdkServerProcConfigPassesSidecarAndCargoEnv(t *testing.T) {
	root := t.TempDir()
	profile := t.TempDir()
	ports := proc.Ports{CDP: 9012, Server: 9123, Extension: 9321}
	sidecar := watchSidecarConfigPath(profile, sdkServerBackend.SidecarName)
	cfg := clawBackendProcConfig(sdkServerBackend, root, nil, ports, profile, sidecar, func(port int, _ time.Duration) error {
		if port != ports.Server {
			t.Errorf("cleared port %d, want %d", port, ports.Server)
		}
		return nil
	})
	wantPath := filepath.Join(profile, "sidecars", "claw-sdk.json")
	wantCmd := []string{"cargo", "run", "-p", "neo-server", "--bin", "browseros-claw-sdk", "--", "--config", wantPath}
	if !slices.Equal(cfg.Cmd, wantCmd) || cfg.Dir != root || !cfg.Restart {
		t.Fatalf("expected supervised SDK command %v from %q, got %v from %q (restart %v)", wantCmd, root, cfg.Cmd, cfg.Dir, cfg.Restart)
	}
	for _, key := range []string{"CARGO_TARGET_DIR", "CARGO_BUILD_BUILD_DIR"} {
		if got, want := envValue(cfg.Env, key), filepath.Join(root, "target", "dev-watch"); got != want {
			t.Errorf("%s = %q, want %q", key, got, want)
		}
	}

	if err := cfg.BeforeStart(); err != nil {
		t.Fatal(err)
	}
	raw, err := os.ReadFile(wantPath)
	if err != nil {
		t.Fatal(err)
	}
	var launch struct {
		Ports struct {
			Server int `json:"server"`
			CDP    int `json:"cdp"`
		} `json:"ports"`
		Directories struct {
			Resources string `json:"resources"`
			Execution string `json:"execution"`
		} `json:"directories"`
	}
	if err := json.Unmarshal(raw, &launch); err != nil {
		t.Fatal(err)
	}
	if launch.Ports.Server != ports.Server || launch.Ports.CDP != ports.CDP {
		t.Errorf("sidecar ports = %+v, want server %d and CDP %d", launch.Ports, ports.Server, ports.CDP)
	}
	if launch.Directories.Execution != filepath.Join(profile, ".browseros") ||
		launch.Directories.Resources != filepath.Join(root, "crates", "neo-sdk", "neo-server") {
		t.Errorf("unexpected SDK launch directories: %+v", launch.Directories)
	}
}

func TestRustSdkWatchInputsCoverNeoSdkCratesAndSharedCrates(t *testing.T) {
	root := t.TempDir()
	dirs := []string{
		"crates/neo-sdk/neo-server/src",
		"crates/neo-sdk/neo-playwright/assets",
		"crates/browseros-cdp/src",
		"crates/browseros-cdp/protocol",
		"apps/claw-server/src",
	}
	for _, dir := range dirs {
		if err := os.MkdirAll(filepath.Join(root, dir), 0o755); err != nil {
			t.Fatal(err)
		}
	}
	manifest := filepath.Join(root, "crates/neo-sdk/neo-server/Cargo.toml")
	if err := os.WriteFile(manifest, []byte("[package]\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	inputs := rustSdkWatchInputs(root)
	for _, path := range dirs[:len(dirs)-1] {
		if !slices.Contains(inputs, filepath.Join(root, path)) {
			t.Errorf("SDK watch inputs omit %s", path)
		}
	}
	if !slices.Contains(inputs, manifest) {
		t.Errorf("SDK watch inputs omit %s", manifest)
	}
	if slices.Contains(inputs, filepath.Join(root, "apps/claw-server/src")) {
		t.Errorf("SDK watch includes legacy server sources")
	}
	wantTail := []string{filepath.Join(root, "Cargo.toml"), filepath.Join(root, "Cargo.lock")}
	if len(inputs) < len(wantTail) || !slices.Equal(inputs[len(inputs)-len(wantTail):], wantTail) {
		t.Errorf("SDK watch inputs %v do not end with %v", inputs, wantTail)
	}
}

func TestWatchReadinessReportsTransitions(t *testing.T) {
	statuses := []int{503, 503, 200, 200, 503, 200}
	var requests atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		index := int(requests.Add(1)) - 1
		if index >= len(statuses) {
			index = len(statuses) - 1
		}
		w.WriteHeader(statuses[index])
	}))
	defer server.Close()
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	var got []bool
	watchReadiness(ctx, server.URL, time.Millisecond, func(ready bool) {
		got = append(got, ready)
		if len(got) == 3 {
			cancel()
		}
	})
	if want := []bool{true, false, true}; !slices.Equal(got, want) {
		t.Fatalf("readiness transitions = %v, want %v", got, want)
	}
	if got := requests.Load(); got != int32(len(statuses)) {
		t.Fatalf("watch did not poll through all status changes, got %d requests", got)
	}
}
