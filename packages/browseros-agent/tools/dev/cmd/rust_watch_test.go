package cmd

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"browseros-dev/proc"
)

func TestWatchModeDefaultsToRustServerAndKeepsTypeScriptSelectable(t *testing.T) {
	oldManual, oldClaw, oldRust, oldTS := watchManual, watchClaw, watchRust, watchTS
	t.Cleanup(func() { watchManual, watchClaw, watchRust, watchTS = oldManual, oldClaw, oldRust, oldTS })
	for _, tc := range []struct {
		manual, claw, rust, ts bool
		want                   string
	}{
		{false, false, false, false, "BrowserOS"},
		{false, false, true, false, "BrowserOS"},
		{true, false, false, false, "BrowserOS manual"},
		{true, false, true, false, "BrowserOS manual"},
		{false, false, false, true, "BrowserOS (TypeScript server)"},
		{true, false, false, true, "BrowserOS manual (TypeScript server)"},
		{false, true, false, false, "BrowserOS neo"},
		{false, true, true, false, "BrowserOS neo"},
	} {
		watchManual, watchClaw, watchRust, watchTS = tc.manual, tc.claw, tc.rust, tc.ts
		got, err := watchMode()
		if err != nil || got != tc.want {
			t.Fatalf("watchMode(%+v) = %q, %v", tc, got, err)
		}
	}
}

func TestWatchModeRejectsTypeScriptWithRustOrClaw(t *testing.T) {
	oldManual, oldClaw, oldRust, oldTS := watchManual, watchClaw, watchRust, watchTS
	t.Cleanup(func() { watchManual, watchClaw, watchRust, watchTS = oldManual, oldClaw, oldRust, oldTS })
	for _, tc := range []struct {
		claw, rust bool
		want       string
	}{
		{false, true, "--ts cannot be combined with --rust"},
		{true, false, "--ts cannot be combined with --claw"},
	} {
		watchManual, watchClaw, watchRust, watchTS = false, tc.claw, tc.rust, true
		if _, err := watchMode(); err == nil || !strings.Contains(err.Error(), tc.want) {
			t.Fatalf("watchMode(claw=%v rust=%v ts) error = %v, want %q", tc.claw, tc.rust, err, tc.want)
		}
	}
}

func TestRustServerWritesBrowserOSSidecarBeforeEachLaunch(t *testing.T) {
	root, profile := t.TempDir(), t.TempDir()
	path := watchSidecarConfigPath(profile, "browseros-server")
	ports := proc.Ports{CDP: 9012, Server: 9123, Extension: 9321}
	cfg := serverRustProcConfig(root, []string{"NODE_ENV=development"}, ports, profile, path)
	if cfg.Dir != root || !cfg.Restart || strings.Join(cfg.Cmd, " ") != "cargo run -p server-rust --bin browseros-server-rs -- --config "+path {
		t.Fatalf("unexpected Cargo supervisor config: %+v", cfg)
	}
	for range 2 {
		if err := cfg.BeforeStart(); err != nil {
			t.Fatal(err)
		}
		raw, err := os.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		var sidecar struct {
			Ports       struct{ Server, CDP int }
			Directories struct{ Resources, Execution string }
		}
		if err := json.Unmarshal(raw, &sidecar); err != nil {
			t.Fatal(err)
		}
		if sidecar.Ports.Server != ports.Server || sidecar.Ports.CDP != ports.CDP || sidecar.Directories.Resources != filepath.Join(root, "resources") || sidecar.Directories.Execution != profile {
			t.Fatalf("unexpected BrowserOS sidecar: %+v", sidecar)
		}
		if err := os.Remove(path); err != nil {
			t.Fatal(err)
		}
	}
}

func TestRustWatcherRestartsOwnedChildOnEmbeddedResourceChange(t *testing.T) {
	root := t.TempDir()
	resourceDir := filepath.Join(root, "apps/server-rust/resources")
	if err := os.MkdirAll(resourceDir, 0o755); err != nil {
		t.Fatal(err)
	}
	resource := filepath.Join(resourceDir, "prompt.md")
	if err := os.WriteFile(resource, []byte("old"), 0o644); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	launches := make(chan struct{}, 8)
	var wg sync.WaitGroup
	child := proc.StartManaged(ctx, &wg, proc.ProcConfig{
		Dir: root, Tag: proc.TagServer, Restart: true,
		Cmd:         []string{"sh", "-c", "sleep 30"},
		BeforeStart: func() error { launches <- struct{}{}; return nil },
	})
	t.Cleanup(func() {
		cancel()
		done := make(chan struct{})
		go func() { wg.Wait(); close(done) }()
		// A launch notification precedes exec.Start. Re-send stop until the
		// child exits so cleanup also covers cancellation during that handoff.
		for {
			child.Stop()
			select {
			case <-done:
				return
			case <-time.After(25 * time.Millisecond):
			}
		}
	})
	<-launches
	startRustSourceWatcher(ctx, &wg, root, "server-rust", rustServerWatchInputs(root, "server-rust"), child)
	if err := os.WriteFile(resource, []byte("new embedded prompt"), 0o644); err != nil {
		t.Fatal(err)
	}
	select {
	case <-launches:
	case <-time.After(5 * time.Second):
		t.Fatal("resource edit did not restart the owned server child")
	}
}
