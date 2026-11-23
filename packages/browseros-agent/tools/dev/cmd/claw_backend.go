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

// rustServerBackend describes a Cargo server for the shared watch supervisor.
// The product launcher owns the browser and the supervisor owns each server launch.
type rustServerBackend struct {
	Label           string
	Package         string
	Toolchain       string
	Bin             string
	Features        []string
	SidecarName     string
	ResourcesDir    string
	ExecutionDir    func(profile string) string
	RustWatchInputs func(root string) []string
	ReadyPath       string
}

var browserOSServerBackend = rustServerBackend{
	Label:           "browseros-server",
	Package:         "browseros-server",
	Toolchain:       "1.98.0",
	SidecarName:     "browseros-server",
	ResourcesDir:    "resources",
	ExecutionDir:    neoExecutionDir,
	RustWatchInputs: rustWatchInputsWithSDK,
}

// billing-staging points the dev claw-server at the staging billing gateway, so neo sells and
// verifies ultrafast against the staging Stripe sandbox.
var clawServerBackend = rustServerBackend{
	Label:           "claw-server",
	Package:         "claw-server",
	Features:        []string{"billing-staging"},
	SidecarName:     "claw-server",
	ResourcesDir:    "apps/claw-server/resources",
	ExecutionDir:    func(profile string) string { return profile },
	RustWatchInputs: rustClawWatchInputs,
}

var sdkServerBackend = rustServerBackend{
	Label:           "neo-server (browseros-claw-sdk)",
	Package:         "neo-server",
	Bin:             "browseros-claw-sdk",
	SidecarName:     "claw-sdk",
	ResourcesDir:    "crates/neo-sdk/neo-server",
	ExecutionDir:    neoExecutionDir,
	RustWatchInputs: rustWatchInputsWithSDK,
	ReadyPath:       "/system/ready",
}

// Neo uses the parent of a .browseros execution directory as its fallback protected profile.
func neoExecutionDir(profile string) string {
	return filepath.Join(profile, ".browseros")
}

func selectClawBackend(value string) (rustServerBackend, error) {
	switch {
	case strings.TrimSpace(value) == "":
		return clawServerBackend, nil
	case value == "1":
		return sdkServerBackend, nil
	default:
		return rustServerBackend{}, fmt.Errorf("BROWSERCLAW_USE_SDK must be %q or unset, got %q", "1", value)
	}
}

func (backend rustServerBackend) cargoCommand(args ...string) []string {
	if backend.Toolchain != "" {
		// rustup selects Cargo before PATH lookup can pick a system Cargo.
		return append([]string{"rustup", "run", backend.Toolchain, "cargo"}, args...)
	}
	return append([]string{"cargo"}, args...)
}

func rustServerProcConfig(backend rustServerBackend, root string, env []string, p proc.Ports, userDataDir string, sidecarPath string) proc.ProcConfig {
	command := backend.cargoCommand("run", "-p", backend.Package)
	if backend.Bin != "" {
		command = append(command, "--bin", backend.Bin)
	}
	if len(backend.Features) > 0 {
		command = append(command, "--features", strings.Join(backend.Features, ","))
	}
	return proc.ProcConfig{
		Tag:     proc.TagServer,
		Dir:     root,
		Env:     rustServerCargoEnv(root, env),
		Restart: true,
		Cmd:     append(command, "--", "--config", sidecarPath),
		BeforeStart: func() error {
			// ManagedProc waits for its recorded child to exit before relaunching.
			// The browser keeps using this port, so leave other listeners alone and retry it.
			if !proc.IsPortAvailable(p.Server) {
				return fmt.Errorf("server port %d is already in use", p.Server)
			}
			return proc.WriteSidecarConfig(sidecarPath, proc.SidecarConfigOptions{
				Ports:         p,
				ResourcesDir:  filepath.Join(root, backend.ResourcesDir),
				ExecutionDir:  backend.ExecutionDir(userDataDir),
				BrowserBinary: envValue(env, "BROWSEROS_BINARY"),
			})
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

func rustWatchInputsWithSDK(root string) []string {
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

func startClawReadinessWatcher(ctx context.Context, wg *sync.WaitGroup, backend rustServerBackend, port int) {
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
