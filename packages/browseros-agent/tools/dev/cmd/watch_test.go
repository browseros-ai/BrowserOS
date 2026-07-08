package cmd

import (
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"

	"browseros-dev/browser"
	"browseros-dev/proc"
)

func TestWatchModeRejectsManualClawCombination(t *testing.T) {
	oldManual, oldClaw := watchManual, watchClaw
	watchManual = true
	watchClaw = true
	t.Cleanup(func() {
		watchManual = oldManual
		watchClaw = oldClaw
	})

	_, err := watchMode()
	if err == nil {
		t.Fatal("expected incompatible watch flags to return an error")
	}
	if !strings.Contains(err.Error(), "--manual cannot be combined with --claw") {
		t.Fatalf("unexpected error: %v", err)
	}
}

func TestWatchModeRejectsRustWithoutClaw(t *testing.T) {
	oldManual, oldClaw, oldRust := watchManual, watchClaw, watchRust
	watchManual = false
	watchClaw = false
	watchRust = true
	t.Cleanup(func() {
		watchManual = oldManual
		watchClaw = oldClaw
		watchRust = oldRust
	})

	_, err := watchMode()
	if err == nil {
		t.Fatal("expected incompatible watch flags to return an error")
	}
	if !strings.Contains(err.Error(), "--rust can only be combined with --claw") {
		t.Fatalf("unexpected error: %v", err)
	}
}

func TestWatchModeSelectsBrowserClawRust(t *testing.T) {
	oldManual, oldClaw, oldRust := watchManual, watchClaw, watchRust
	watchManual = false
	watchClaw = true
	watchRust = true
	t.Cleanup(func() {
		watchManual = oldManual
		watchClaw = oldClaw
		watchRust = oldRust
	})

	mode, err := watchMode()
	if err != nil {
		t.Fatalf("watchMode returned error: %v", err)
	}
	if mode != "BrowserClaw Rust" {
		t.Fatalf("expected BrowserClaw Rust mode, got %q", mode)
	}
}

func TestWatchRunLockModeIsSharedAcrossWatchVariants(t *testing.T) {
	if watchRunLockMode != "watch" {
		t.Fatalf("expected shared watch lock mode, got %q", watchRunLockMode)
	}
}

func TestResolveWatchDefaultPortsKeepsBrowserOSServerPort(t *testing.T) {
	root := writeWatchEnvExample(t, "BROWSEROS_CDP_PORT=9001\nBROWSEROS_SERVER_PORT=9101\nBROWSEROS_EXTENSION_PORT=9301\n")

	ports, err := resolveWatchDefaultPorts(root, false)
	if err != nil {
		t.Fatalf("resolveWatchDefaultPorts returned error: %v", err)
	}

	want := proc.Ports{CDP: 9001, Server: 9101, Extension: 9301}
	if ports != want {
		t.Fatalf("expected BrowserOS watch ports %+v, got %+v", want, ports)
	}
}

func TestResolveWatchDefaultPortsUsesStandaloneClawServerPort(t *testing.T) {
	root := writeWatchEnvExample(t, "BROWSEROS_CDP_PORT=9001\nBROWSEROS_SERVER_PORT=9101\nBROWSEROS_EXTENSION_PORT=9301\n")

	ports, err := resolveWatchDefaultPorts(root, true)
	if err != nil {
		t.Fatalf("resolveWatchDefaultPorts returned error: %v", err)
	}

	want := proc.Ports{CDP: 9001, Server: defaultClawWatchServerPort, Extension: 9301}
	if ports != want {
		t.Fatalf("expected Claw watch ports %+v, got %+v", want, ports)
	}
}

func TestBuildWatchEnvSelectsBrowserOSProduct(t *testing.T) {
	env := buildWatchEnv(proc.Ports{
		CDP:       9012,
		Server:    9123,
		Extension: 9321,
	}, "/tmp/browseros-dev", false)

	for _, want := range []string{
		"BROWSEROS_PRODUCT=browseros",
		"BROWSEROS_USER_DATA_DIR=/tmp/browseros-dev",
	} {
		if !hasEnvEntry(env, want) {
			t.Fatalf("expected env to contain %q, got %#v", want, env)
		}
	}
}

func TestBuildWatchEnvSelectsBrowserClawProduct(t *testing.T) {
	env := buildWatchEnvWithBinaryResolution(proc.Ports{
		CDP:       9012,
		Server:    9123,
		Extension: 9321,
	}, "/tmp/browseros-dev", true, browser.BinaryResolution{
		Product:       browser.ProductBrowserClaw,
		Path:          browser.BrowserClawBinaryPath,
		PreferredPath: browser.BrowserClawBinaryPath,
	})

	for _, want := range []string{
		"BROWSEROS_PRODUCT=browserclaw",
		"BROWSEROS_BINARY=/Applications/BrowserClaw.app/Contents/MacOS/BrowserClaw",
		"BROWSEROS_CLAW_CDP_PORT=9012",
		"VITE_BROWSEROS_CLAW_API_URL=http://127.0.0.1:9123",
	} {
		if !hasEnvEntry(env, want) {
			t.Fatalf("expected env to contain %q, got %#v", want, env)
		}
	}
}

func TestBuildClawWatchEnvIncludesSelectedPorts(t *testing.T) {
	env := buildClawWatchEnv([]string{"BASE=1"}, proc.Ports{
		CDP:       9012,
		Server:    9123,
		Extension: 9321,
	})

	for _, want := range []string{
		"BASE=1",
		"BROWSEROS_CLAW_CDP_PORT=9012",
		"VITE_BROWSEROS_CLAW_API_URL=http://127.0.0.1:9123",
	} {
		if !hasEnvEntry(env, want) {
			t.Fatalf("expected env to contain %q, got %#v", want, env)
		}
	}
	if hasEnvEntry(env, "CLAW_SERVER_PORT=9123") {
		t.Fatalf("claw server port should be passed through sidecar config, got %#v", env)
	}
}

func TestBuildWatchEnvUsesFallbackBrowserBinaryForClaw(t *testing.T) {
	env := buildWatchEnvWithBinaryResolution(proc.Ports{
		CDP:    9012,
		Server: 9123,
	}, "/tmp/browseros-dev", true, browser.BinaryResolution{
		Product:       browser.ProductBrowserClaw,
		Path:          browser.BrowserOSBinaryPath,
		PreferredPath: browser.BrowserClawBinaryPath,
		Fallback:      true,
	})

	if !hasEnvEntry(env, "BROWSEROS_BINARY=/Applications/BrowserOS.app/Contents/MacOS/BrowserOS") {
		t.Fatalf("expected fallback browser binary env, got %#v", env)
	}
}

func TestClawRustServerProcConfigPassesSidecarAndDevEnv(t *testing.T) {
	root := t.TempDir()
	userDataDir := t.TempDir()
	sidecarPath := watchSidecarConfigPath(userDataDir, "claw-server")
	ports := proc.Ports{
		CDP:       9012,
		Server:    9123,
		Extension: 9321,
	}
	env := buildWatchEnv(ports, userDataDir, true)
	var killedPort int
	cfg := clawServerProcConfig(root, env, ports, userDataDir, sidecarPath, true, func(port int, _ time.Duration) error {
		killedPort = port
		return nil
	})

	wantCmd := []string{"cargo", "run", "-p", "claw-server-rust", "--", "--config", sidecarPath}
	if !reflect.DeepEqual(cfg.Cmd, wantCmd) {
		t.Fatalf("expected rust server command %#v, got %#v", wantCmd, cfg.Cmd)
	}
	if cfg.Dir != root {
		t.Fatalf("expected cargo to run from workspace root %q, got %q", root, cfg.Dir)
	}
	if !cfg.Restart {
		t.Fatal("expected rust claw-server to run under restart supervision")
	}
	for _, want := range []string{
		"NODE_ENV=development",
		"BROWSEROS_PRODUCT=browserclaw",
		"BROWSEROS_CLAW_CDP_PORT=9012",
		"VITE_BROWSEROS_CLAW_API_URL=http://127.0.0.1:9123",
	} {
		if !hasEnvEntry(cfg.Env, want) {
			t.Fatalf("expected env to contain %q, got %#v", want, cfg.Env)
		}
	}

	if err := cfg.BeforeStart(); err != nil {
		t.Fatalf("BeforeStart returned error: %v", err)
	}
	if killedPort != ports.Server {
		t.Fatalf("expected BeforeStart to kill server port %d, got %d", ports.Server, killedPort)
	}

	raw, err := os.ReadFile(sidecarPath)
	if err != nil {
		t.Fatalf("reading sidecar: %v", err)
	}
	var sidecar struct {
		Ports struct {
			Server int `json:"server"`
			CDP    int `json:"cdp"`
			Proxy  int `json:"proxy"`
		} `json:"ports"`
	}
	if err := json.Unmarshal(raw, &sidecar); err != nil {
		t.Fatalf("parsing sidecar: %v", err)
	}
	if sidecar.Ports.Server != ports.Server || sidecar.Ports.CDP != ports.CDP || sidecar.Ports.Proxy != ports.Server {
		t.Fatalf("expected sidecar ports server=%d cdp=%d proxy=%d, got %+v", ports.Server, ports.CDP, ports.Server, sidecar.Ports)
	}
}

func TestRustClawWatchInputsUseSourceAndManifestInputs(t *testing.T) {
	root := t.TempDir()
	for _, dir := range []string{
		"apps/claw-server/drizzle",
		"apps/claw-server-rust/src",
		"crates/browseros-core/src",
		"crates/browseros-cdp/src",
		"crates/browseros-cdp/protocol",
		"target/debug",
	} {
		if err := os.MkdirAll(filepath.Join(root, dir), 0o755); err != nil {
			t.Fatal(err)
		}
	}
	for _, file := range []string{
		"apps/claw-server-rust/Cargo.toml",
		"crates/browseros-core/Cargo.toml",
		"crates/browseros-cdp/Cargo.toml",
		"crates/browseros-cdp/build.rs",
	} {
		if err := os.WriteFile(filepath.Join(root, file), []byte("[package]\n"), 0o644); err != nil {
			t.Fatal(err)
		}
	}

	inputs := rustClawWatchInputs(root)
	for _, want := range []string{
		filepath.Join(root, "apps/claw-server-rust/src"),
		filepath.Join(root, "apps/claw-server-rust/Cargo.toml"),
		filepath.Join(root, "apps/claw-server/drizzle"),
		filepath.Join(root, "crates/browseros-cdp/src"),
		filepath.Join(root, "crates/browseros-cdp/Cargo.toml"),
		filepath.Join(root, "crates/browseros-cdp/build.rs"),
		filepath.Join(root, "crates/browseros-cdp/protocol"),
		filepath.Join(root, "crates/browseros-core/src"),
		filepath.Join(root, "crates/browseros-core/Cargo.toml"),
		filepath.Join(root, "Cargo.toml"),
		filepath.Join(root, "Cargo.lock"),
	} {
		if !containsString(inputs, want) {
			t.Fatalf("expected rust watch inputs to contain %q, got %#v", want, inputs)
		}
	}
	for _, input := range inputs {
		if strings.Contains(input, string(filepath.Separator)+"target"+string(filepath.Separator)) {
			t.Fatalf("target directory should not be a rust watch input, got %#v", inputs)
		}
	}
}

func TestRustWatchSnapshotDetectsSourceChangesAndSkipsTargetDirs(t *testing.T) {
	root := t.TempDir()
	srcDir := filepath.Join(root, "apps/claw-server-rust/src")
	targetDir := filepath.Join(srcDir, "target")
	if err := os.MkdirAll(targetDir, 0o755); err != nil {
		t.Fatal(err)
	}
	sourcePath := filepath.Join(srcDir, "main.rs")
	targetPath := filepath.Join(targetDir, "artifact")
	if err := os.WriteFile(sourcePath, []byte("fn main() {}\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(targetPath, []byte("compiled\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	previous, err := snapshotRustWatchInputs([]string{srcDir})
	if err != nil {
		t.Fatalf("snapshot returned error: %v", err)
	}
	if _, ok := previous[targetPath]; ok {
		t.Fatalf("snapshot should skip target artifacts, got %#v", previous)
	}
	if err := os.WriteFile(sourcePath, []byte("fn main() {}\n\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	next, err := snapshotRustWatchInputs([]string{srcDir})
	if err != nil {
		t.Fatalf("snapshot returned error: %v", err)
	}
	changed, path := rustWatchSnapshotChanged(previous, next)
	if !changed {
		t.Fatal("expected source edit to be detected")
	}
	if path != sourcePath {
		t.Fatalf("expected changed path %q, got %q", sourcePath, path)
	}
}

func TestEnsureLimactlPresentMissingMessage(t *testing.T) {
	t.Setenv("PATH", t.TempDir())

	err := ensureLimactlPresent()
	if err == nil {
		t.Fatal("expected missing Lima error")
	}

	msg := err.Error()
	if !strings.Contains(msg, "Lima is not installed.") {
		t.Fatalf("expected missing Lima message, got %q", msg)
	}
	if !strings.Contains(msg, "brew install lima") {
		t.Fatalf("expected brew install hint, got %q", msg)
	}
}

func TestEnsureLimactlPresentFindsPathBinary(t *testing.T) {
	binDir := t.TempDir()
	limactlPath := filepath.Join(binDir, "limactl")
	if err := os.WriteFile(limactlPath, []byte("#!/bin/sh\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", binDir)

	if err := ensureLimactlPresent(); err != nil {
		t.Fatalf("expected limactl to resolve, got %v", err)
	}
}

func TestEnsureCargoPresentMissingMessage(t *testing.T) {
	t.Setenv("PATH", t.TempDir())

	err := ensureCargoPresent()
	if err == nil {
		t.Fatal("expected missing Cargo error")
	}

	msg := err.Error()
	if !strings.Contains(msg, "Cargo is required for --claw --rust") {
		t.Fatalf("expected missing Cargo message, got %q", msg)
	}
	if !strings.Contains(msg, "rustup.rs") {
		t.Fatalf("expected rustup install hint, got %q", msg)
	}
}

func TestEnsureCargoPresentFindsPathBinary(t *testing.T) {
	binDir := t.TempDir()
	cargoPath := filepath.Join(binDir, "cargo")
	if err := os.WriteFile(cargoPath, []byte("#!/bin/sh\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", binDir)

	if err := ensureCargoPresent(); err != nil {
		t.Fatalf("expected cargo to resolve, got %v", err)
	}
}

func hasEnvEntry(env []string, want string) bool {
	for _, got := range env {
		if got == want {
			return true
		}
	}
	return false
}

func containsString(values []string, want string) bool {
	for _, got := range values {
		if got == want {
			return true
		}
	}
	return false
}

func writeWatchEnvExample(t *testing.T, contents string) string {
	t.Helper()
	root := t.TempDir()
	serverDir := filepath.Join(root, "apps/server")
	if err := os.MkdirAll(serverDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(serverDir, ".env.example"), []byte(contents), 0o644); err != nil {
		t.Fatal(err)
	}
	return root
}
