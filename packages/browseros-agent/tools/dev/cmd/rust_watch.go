package cmd

// This module connects Cargo's rebuilds to the existing managed child lifecycle.
// Both products share restart/cancellation behavior while keeping their sidecar
// resource roots and application sources separate.

import (
	"context"
	"path/filepath"
	"sort"
	"sync"
	"time"

	"browseros-dev/browser"
	"browseros-dev/proc"
)

func serverRustProcConfig(root string, env []string, p proc.Ports, userDataDir string, sidecarPath string) proc.ProcConfig {
	return proc.ProcConfig{
		Tag:     proc.TagServer,
		Dir:     root,
		Env:     env,
		Restart: true,
		// server-rust also builds a test-only ACP peer binary, so `cargo run` must name the server.
		Cmd: []string{"cargo", "run", "-p", "server-rust", "--bin", "browseros-server-rs", "--", "--config", sidecarPath},
		BeforeStart: func() error {
			// The supervisor reaps its process group before restarting. Do not kill
			// by port here: a new listener could belong to another dev session.
			binary := envValue(env, "BROWSEROS_BINARY")
			if binary == "" {
				binary = browser.ResolveInstalledBinary(browser.ProductBrowserOS).Path
			}
			return writeServerSidecarConfig(sidecarPath, filepath.Join(root, "resources"), userDataDir, p, binary)
		},
	}
}

// rustServerWatchInputs includes inputs Cargo embeds as well as source code.
// Build output is deliberately excluded to prevent rebuild/restart feedback loops.
func rustServerWatchInputs(root string, packageName string) []string {
	app := filepath.Join(root, "apps", packageName)
	inputs := []string{
		filepath.Join(app, "src"),
		filepath.Join(app, "Cargo.toml"),
		filepath.Join(app, "build.rs"),
		filepath.Join(app, "resources"),
		filepath.Join(app, "tests", "fixtures"),
	}
	for _, pattern := range []string{
		filepath.Join(root, "crates", "*", "src"),
		filepath.Join(root, "crates", "*", "Cargo.toml"),
		filepath.Join(root, "crates", "*", "build.rs"),
		filepath.Join(root, "crates", "*", "protocol"),
	} {
		matches, err := filepath.Glob(pattern)
		if err == nil {
			sort.Strings(matches)
			inputs = append(inputs, matches...)
		}
	}
	return append(inputs,
		filepath.Join(root, "Cargo.toml"),
		filepath.Join(root, "Cargo.lock"),
		filepath.Join(root, "contracts", "server-api"),
	)
}

func startRustSourceWatcher(ctx context.Context, wg *sync.WaitGroup, root, packageName string, inputs []string, serverProc *proc.ManagedProc) {
	proc.LogMsgf(proc.TagBuild, "Watching Rust %s sources (%d inputs)", packageName, len(inputs))
	// Establish the baseline before returning so edits immediately after watch
	// starts cannot become part of the baseline and silently miss a rebuild.
	snapshot, err := snapshotRustWatchInputs(inputs)
	if err != nil {
		proc.LogMsgf(proc.TagBuild, "Warning: initial Rust watch scan failed: %v", err)
	}
	wg.Add(1)
	go func() {
		defer wg.Done()
		watchRustSources(ctx, root, packageName, inputs, snapshot, serverProc)
	}()
}

func watchRustSources(ctx context.Context, root, packageName string, inputs []string, snapshot map[string]rustWatchedFile, serverProc *proc.ManagedProc) {
	ticker := time.NewTicker(rustWatchPollInterval)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			next, err := snapshotRustWatchInputs(inputs)
			if err != nil {
				proc.LogMsgf(proc.TagBuild, "Warning: Rust watch scan failed: %v", err)
				continue
			}
			if snapshot == nil {
				snapshot = next
				continue
			}
			changed, path := rustWatchSnapshotChanged(snapshot, next)
			snapshot = next
			if !changed {
				continue
			}
			displayPath, err := filepath.Rel(root, path)
			if err != nil {
				displayPath = path
			}
			proc.LogMsgf(proc.TagBuild, "Rust source changed (%s); restarting %s", displayPath, packageName)
			// Restart signals only the owned Cargo/server process group and waits
			// for it to exit; the supervisor then rebuilds with the same sidecar.
			if !serverProc.Restart() {
				proc.LogMsgf(proc.TagBuild, "Rust %s process is not running yet; restart will happen after the current launch attempt", packageName)
			}
		}
	}
}
