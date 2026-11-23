package cmd

import (
	"context"
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"browseros-dev/browser"
	"browseros-dev/proc"
)

func reserveRustServerTestPort(t *testing.T) *net.TCPListener {
	t.Helper()
	listener, err := net.ListenTCP("tcp4", &net.TCPAddr{IP: net.IPv4(127, 0, 0, 1)})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { listener.Close() })
	return listener
}

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

func TestBrowserOSServerProcConfigPassesSidecarAndCargoEnv(t *testing.T) {
	t.Setenv("CARGO_TARGET_DIR", "")
	t.Setenv("CARGO_BUILD_BUILD_DIR", "")
	t.Setenv("BROWSEROS_BINARY", "")
	root, profile := t.TempDir(), t.TempDir()
	listener := reserveRustServerTestPort(t)
	ports := proc.Ports{CDP: 9012, Server: listener.Addr().(*net.TCPAddr).Port, Extension: 9321}
	listener.Close()
	env := buildWatchEnvWithResolvedState(ports, profile, false, browser.BinaryResolution{}, "BROWSEROS_DIR", filepath.Join(profile, "state"))
	sidecar := watchSidecarConfigPath(profile, browserOSServerBackend.SidecarName)
	cfg := rustServerProcConfig(browserOSServerBackend, root, env, ports, profile, sidecar)
	wantPath := filepath.Join(profile, "sidecars", "browseros-server.json")
	wantCmd := []string{"rustup", "run", "1.98.0", "cargo", "run", "-p", "browseros-server", "--", "--config", wantPath}
	if !slices.Equal(cfg.Cmd, wantCmd) || cfg.Dir != root || !cfg.Restart {
		t.Fatalf("expected supervised BrowserOS command %v from %q, got %v from %q (restart %v)", wantCmd, root, cfg.Cmd, cfg.Dir, cfg.Restart)
	}
	t.Logf("command from %s: %s", cfg.Dir, strings.Join(cfg.Cmd, " "))
	for key, want := range map[string]string{
		"BROWSEROS_CDP_PORT":       "9012",
		"BROWSEROS_SERVER_PORT":    fmt.Sprint(ports.Server),
		"BROWSEROS_EXTENSION_PORT": "9321",
		"BROWSEROS_DIR":            filepath.Join(profile, "state"),
		"BROWSEROS_USER_DATA_DIR":  profile,
		"BROWSEROS_PRODUCT":        "browseros",
		"NODE_ENV":                 "development",
		"CARGO_TARGET_DIR":         filepath.Join(root, "target", "dev-watch"),
		"CARGO_BUILD_BUILD_DIR":    filepath.Join(root, "target", "dev-watch"),
	} {
		got := envValue(cfg.Env, key)
		if got != want {
			t.Errorf("%s = %q, want %q", key, got, want)
		}
		t.Logf("%s=%s", key, got)
	}

	if err := cfg.BeforeStart(); err != nil {
		t.Fatal(err)
	}
	raw, err := os.ReadFile(wantPath)
	if err != nil {
		t.Fatal(err)
	}
	t.Logf("launch file:\n%s", raw)
	var launch struct {
		Ports struct {
			Server int `json:"server"`
			CDP    int `json:"cdp"`
			Proxy  int `json:"proxy"`
		} `json:"ports"`
		Directories struct {
			Resources string `json:"resources"`
			Execution string `json:"execution"`
		} `json:"directories"`
		Flags struct {
			DevMode          bool  `json:"devMode"`
			AllowRemoteInMCP *bool `json:"allow_remote_in_mcp"`
		} `json:"flags"`
	}
	if err := json.Unmarshal(raw, &launch); err != nil {
		t.Fatal(err)
	}
	if launch.Ports.Server != ports.Server || launch.Ports.CDP != ports.CDP || launch.Ports.Proxy != ports.Server {
		t.Errorf("unexpected BrowserOS launch ports: %+v", launch.Ports)
	}
	if launch.Directories.Resources != filepath.Join(root, "resources") || launch.Directories.Execution != filepath.Join(profile, ".browseros") {
		t.Errorf("unexpected BrowserOS launch directories: %+v", launch.Directories)
	}
	if !launch.Flags.DevMode || launch.Flags.AllowRemoteInMCP == nil || *launch.Flags.AllowRemoteInMCP {
		t.Errorf("unexpected BrowserOS launch flags: %+v", launch.Flags)
	}
}

func TestRustServerLaunchReportsBusyPort(t *testing.T) {
	listener := reserveRustServerTestPort(t)
	profile := t.TempDir()
	port := listener.Addr().(*net.TCPAddr).Port
	cfg := rustServerProcConfig(browserOSServerBackend, t.TempDir(), nil,
		proc.Ports{CDP: 9012, Server: port}, profile, watchSidecarConfigPath(profile, browserOSServerBackend.SidecarName))

	if err := cfg.BeforeStart(); err == nil || err.Error() != fmt.Sprintf("server port %d is already in use", port) {
		t.Fatalf("BeforeStart conflict = %v", err)
	}
	if proc.IsPortAvailable(port) {
		t.Fatal("launch displaced the existing listener")
	}
}

func TestBrowserOSServerWatchesServerAndSDKInputs(t *testing.T) {
	root := t.TempDir()
	paths := []string{
		"crates/browseros-server/src/main.rs",
		"crates/browseros-server/Cargo.toml",
		"crates/neo-sdk/neo-playwright/assets/script.js",
		"crates/browseros-cdp/build.rs",
		"Cargo.toml",
		"Cargo.lock",
	}
	for _, path := range paths {
		path = filepath.Join(root, path)
		if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, nil, 0o644); err != nil {
			t.Fatal(err)
		}
	}

	snapshot, err := snapshotRustWatchInputs(browserOSServerBackend.RustWatchInputs(root))
	if err != nil {
		t.Fatal(err)
	}
	for _, path := range paths {
		if _, watched := snapshot[filepath.Join(root, path)]; !watched {
			t.Errorf("BrowserOS source watcher omitted %s", path)
		}
	}
}

func TestSdkServerProcConfigPassesSidecarAndCargoEnv(t *testing.T) {
	root := t.TempDir()
	profile := t.TempDir()
	listener := reserveRustServerTestPort(t)
	ports := proc.Ports{CDP: 9012, Server: listener.Addr().(*net.TCPAddr).Port, Extension: 9321}
	listener.Close()
	sidecar := watchSidecarConfigPath(profile, sdkServerBackend.SidecarName)
	cfg := rustServerProcConfig(sdkServerBackend, root, nil, ports, profile, sidecar)
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

	inputs := rustWatchInputsWithSDK(root)
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
