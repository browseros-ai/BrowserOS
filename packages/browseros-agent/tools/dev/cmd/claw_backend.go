package cmd

import (
	"context"
	"fmt"
	"net/http"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"

	"browseros-dev/proc"
)

// clawBackend holds the differences between the two Rust servers supervised
// by Claw watch. Both use the same browser, launch JSON writer and process lifecycle.
type clawBackend struct {
	Label           string
	Package         string
	Bin             string
	SidecarName     string
	ResourcesDir    string
	ExecutionDir    func(profile string) string
	RustWatchInputs func(root string) []string
	ReadyPath       string
}

var clawServerBackend = clawBackend{
	Label:           "claw-server",
	Package:         "claw-server",
	SidecarName:     "claw-server",
	ResourcesDir:    "apps/claw-server/resources",
	ExecutionDir:    func(profile string) string { return profile },
	RustWatchInputs: rustClawWatchInputs,
}

var sdkServerBackend = clawBackend{
	Label:        "neo-server (browseros-claw-sdk)",
	Package:      "neo-server",
	Bin:          "browseros-claw-sdk",
	SidecarName:  "claw-sdk",
	ResourcesDir: "crates/neo-sdk/neo-server",
	// Neo derives the upload deny root from the execution directory's parent.
	// Passing the profile itself would put the entire temp directory off limits.
	ExecutionDir:    func(profile string) string { return filepath.Join(profile, ".browseros") },
	RustWatchInputs: rustSdkWatchInputs,
	ReadyPath:       "/system/ready",
}

func selectClawBackend(value string) (clawBackend, error) {
	switch {
	case strings.TrimSpace(value) == "":
		return clawServerBackend, nil
	case value == "1":
		return sdkServerBackend, nil
	default:
		return clawBackend{}, fmt.Errorf("BROWSERCLAW_USE_SDK must be %q or unset, got %q", "1", value)
	}
}

func clawBackendProcConfig(backend clawBackend, root string, env []string, p proc.Ports, userDataDir string, sidecarPath string, killPort func(int, time.Duration) error) proc.ProcConfig {
	command := []string{"cargo", "run", "-p", backend.Package}
	if backend.Bin != "" {
		command = append(command, "--bin", backend.Bin)
	}
	return proc.ProcConfig{
		Tag:     proc.TagServer,
		Dir:     root,
		Env:     clawServerCargoEnv(root, env),
		Restart: true,
		Cmd:     append(command, "--", "--config", sidecarPath),
		// ManagedProc invokes this before every launch, including restarts.
		// The sidecar and the released server port must match the running browser.
		BeforeStart: func() error {
			if err := proc.WriteSidecarConfig(sidecarPath, proc.SidecarConfigOptions{
				Ports:         p,
				ResourcesDir:  filepath.Join(root, backend.ResourcesDir),
				ExecutionDir:  backend.ExecutionDir(userDataDir),
				BrowserBinary: envValue(env, "BROWSEROS_BINARY"),
			}); err != nil {
				return err
			}
			return killPort(p.Server, 3*time.Second)
		},
	}
}

func rustClawWatchInputs(root string) []string {
	inputs := []string{
		filepath.Join(root, "apps/claw-server/src"),
		filepath.Join(root, "apps/claw-server/Cargo.toml"),
		filepath.Join(root, "apps/claw-server/tests/fixtures/legacy-drizzle"),
	}
	return append(inputs, sharedRustWatchInputs(root)...)
}

func rustSdkWatchInputs(root string) []string {
	// The nested SDK crates are outside the shared crate glob. Their embedded
	// JavaScript is compiled into the Rust binary, so asset changes restart it too.
	inputs := rustWatchGlobs(filepath.Join(root, "crates", "neo-sdk", "*"), "src", "Cargo.toml", "assets")
	return append(inputs, sharedRustWatchInputs(root)...)
}

func sharedRustWatchInputs(root string) []string {
	inputs := rustWatchGlobs(filepath.Join(root, "crates", "*"), "src", "Cargo.toml", "build.rs", "protocol")
	return append(inputs, filepath.Join(root, "Cargo.toml"), filepath.Join(root, "Cargo.lock"))
}

func rustWatchGlobs(cratePattern string, entries ...string) []string {
	var inputs []string
	for _, entry := range entries {
		matches, err := filepath.Glob(filepath.Join(cratePattern, entry))
		if err == nil {
			sort.Strings(matches)
			inputs = append(inputs, matches...)
		}
	}
	return inputs
}

func startClawReadinessWatcher(ctx context.Context, wg *sync.WaitGroup, backend clawBackend, port int) {
	if backend.ReadyPath == "" {
		return
	}
	origin := fmt.Sprintf("http://127.0.0.1:%d", port)
	// The watch context owns the probe across server restarts and cancels its
	// in-flight HTTP request before the watch releases its profile and port locks.
	wg.Add(1)
	go func() {
		defer wg.Done()
		watchReadiness(ctx, origin+backend.ReadyPath, 500*time.Millisecond, func(ready bool) {
			if ready {
				proc.LogMsgf(proc.TagServer, "%s ready: MCP at %s/mcp (one tool: run)", backend.Package, origin)
			} else {
				proc.LogMsgf(proc.TagServer, "%s not ready; waiting", backend.Package)
			}
		})
	}()
}

// watchReadiness reports transitions from an initially unready server. The SDK
// can listen before its CDP connection is usable, so only HTTP 200 means ready.
func watchReadiness(ctx context.Context, url string, interval time.Duration, onChange func(ready bool)) {
	client := &http.Client{
		Timeout: time.Second,
		CheckRedirect: func(*http.Request, []*http.Request) error {
			return http.ErrUseLastResponse
		},
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return
	}
	ticker := time.NewTicker(interval)
	defer ticker.Stop()

	lastReady := false
	for {
		resp, err := client.Do(req)
		ready := false
		if err == nil {
			ready = resp.StatusCode == http.StatusOK
			_ = resp.Body.Close()
		}
		if ctx.Err() != nil {
			return
		}
		if ready != lastReady {
			onChange(ready)
			lastReady = ready
		}
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
		}
	}
}
