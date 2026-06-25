package cmd

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"sync"
	"syscall"
	"time"

	"browseros-dev/browser"
	"browseros-dev/proc"

	"github.com/spf13/cobra"
)

var watchCmd = &cobra.Command{
	Use:   "watch",
	Short: "Start the dev environment with process supervision",
	Long:  "Starts the agent (WXT HMR or static), waits for CDP, then starts the server.",
	RunE:  runWatch,
}

var (
	watchNew    bool
	watchManual bool
	watchClaw   bool
)

const (
	watchRunLockMode           = "watch"
	defaultClawWatchServerPort = 9200
)

func init() {
	watchCmd.Flags().BoolVar(&watchNew, "new", false, "Use random available ports in 9000-9999 and create a fresh user-data directory")
	watchCmd.Flags().BoolVar(&watchManual, "manual", false, "Build agent statically instead of WXT HMR mode")
	watchCmd.Flags().BoolVar(&watchClaw, "claw", false, "Run the BrowserClaw UI and standalone server")
	rootCmd.AddCommand(watchCmd)
}

func runWatch(cmd *cobra.Command, args []string) error {
	mode, err := watchMode()
	if err != nil {
		return err
	}

	root, err := proc.FindMonorepoRoot()
	if err != nil {
		return err
	}
	if err := ensureLimactlPresent(); err != nil {
		return err
	}

	defaultPorts, err := resolveWatchDefaultPorts(root, watchClaw)
	if err != nil {
		return err
	}
	p := defaultPorts
	var reservations *proc.PortReservations
	userDataDir, err := proc.DefaultDevUserDataDir(root)
	if err != nil {
		return err
	}
	var runLock *proc.WatchRunLock
	acquireRunLock := func(ports proc.Ports) error {
		lock, stopped, err := proc.AcquireWatchRunLock(proc.WatchRunIdentity{
			// All watch variants share one owner so they cannot supervise the same profile concurrently.
			Mode:    watchRunLockMode,
			Profile: userDataDir,
			Ports:   ports,
		}, 3*time.Second)
		if err != nil {
			return err
		}
		runLock = lock
		if stopped {
			proc.LogMsgf(proc.TagInfo, "Stopped existing dev watch for profile %s", userDataDir)
		}
		return nil
	}

	if watchNew {
		proc.LogMsg(proc.TagInfo, "Selecting random available ports...")
		p, reservations, err = proc.ResolveWatchPorts(true)
		if err != nil {
			return err
		}

		dir, err := os.MkdirTemp("", "browseros-dev-")
		if err != nil {
			return fmt.Errorf("creating temp dir: %w", err)
		}
		userDataDir = dir
		proc.LogMsgf(proc.TagInfo, "Created fresh profile: %s", userDataDir)
		if err := acquireRunLock(p); err != nil {
			return err
		}
	} else {
		if err := os.MkdirAll(userDataDir, 0o755); err != nil {
			return fmt.Errorf("creating user-data dir: %w", err)
		}
		if err := acquireRunLock(p); err != nil {
			return err
		}
		proc.LogMsg(proc.TagInfo, "Killing processes on preferred ports...")
		if err := proc.KillPortsAndWait(defaultPorts, 3*time.Second); err != nil {
			return err
		}
		proc.LogMsg(proc.TagInfo, "Ports cleared")
		killedBrowsers, err := proc.KillBrowserProcessesForUserDataDirs([]string{userDataDir}, 3*time.Second)
		if err != nil {
			return err
		}
		if killedBrowsers > 0 {
			proc.LogMsgf(proc.TagInfo, "Stopped %d BrowserOS process(es) for profile %s", killedBrowsers, userDataDir)
		}

		p, reservations, err = proc.ResolveWatchPortsWithDefaults(defaultPorts, false)
		if err != nil {
			return err
		}
		if p != defaultPorts {
			proc.LogMsgf(proc.TagInfo,
				"Preferred ports unavailable, using fallback ports: CDP=%d Server=%d Extension=%d",
				p.CDP, p.Server, p.Extension)
		}
	}
	defer func() {
		if err := runLock.Close(); err != nil {
			proc.LogMsgf(proc.TagInfo, "Warning: closing run lock: %v", err)
		}
	}()
	defer reservations.ReleaseAll()

	if err := runDevSetup(cmd.Context(), root, setupModeIfNeeded); err != nil {
		return err
	}

	fmt.Println()
	proc.LogMsgf(proc.TagInfo, "Mode: %s", proc.BoldColor.Sprint(mode))
	proc.LogMsgf(proc.TagInfo, "Ports: CDP=%d Server=%d Extension=%d", p.CDP, p.Server, p.Extension)
	proc.LogMsgf(proc.TagInfo, "Profile: %s", userDataDir)
	proc.LogMsg(proc.TagInfo, proc.DimColor.Sprint("Press Ctrl+C to stop, double Ctrl+C to force kill"))
	fmt.Println()

	env := proc.BuildEnv(p, "development")
	env = append(env, fmt.Sprintf("BROWSEROS_USER_DATA_DIR=%s", userDataDir))
	if watchClaw {
		env = buildClawWatchEnv(env, p)
	}

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	sigCh := make(chan os.Signal, 2)
	signal.Notify(sigCh, syscall.SIGINT, syscall.SIGTERM, syscall.SIGQUIT)

	var wg sync.WaitGroup
	var procs []*proc.ManagedProc

	if watchClaw {
		procs = startClawWatch(ctx, &wg, root, env, p, reservations)
	} else {
		procs, err = startBrowserOSWatch(ctx, &wg, root, env, p, reservations, userDataDir, watchManual)
		if err != nil {
			return err
		}
	}

	<-sigCh
	fmt.Println()
	proc.LogMsg(proc.TagInfo, proc.WarnColor.Sprint("Shutting down (Ctrl+C again to force)..."))
	cancel()

	go func() {
		<-sigCh
		fmt.Println()
		proc.LogMsg(proc.TagInfo, proc.ErrorColor.Sprint("Force killing all processes..."))
		for _, p := range procs {
			p.ForceKill()
		}
		os.Exit(1)
	}()

	for _, p := range procs {
		p.Stop()
	}
	wg.Wait()
	proc.LogMsg(proc.TagInfo, "All processes stopped")
	return nil
}

// watchMode resolves the user-facing mode label for logs.
func watchMode() (string, error) {
	if watchManual && watchClaw {
		return "", fmt.Errorf("--manual cannot be combined with --claw")
	}
	if watchClaw {
		return "BrowserClaw", nil
	}
	if watchManual {
		return "BrowserOS manual", nil
	}
	return "BrowserOS", nil
}

// resolveWatchDefaultPorts picks preferred watch ports for the selected app stack.
func resolveWatchDefaultPorts(root string, claw bool) (proc.Ports, error) {
	ports, err := resolveTargetPorts(root, "")
	if err != nil {
		return proc.Ports{}, err
	}
	if claw {
		ports.Server = defaultClawWatchServerPort
	}
	return ports, nil
}

// buildClawWatchEnv bridges shared dev ports into the standalone BrowserClaw apps.
func buildClawWatchEnv(env []string, p proc.Ports) []string {
	apiURL := fmt.Sprintf("http://127.0.0.1:%d/cockpit", p.Server)
	return append(env,
		fmt.Sprintf("CLAW_SERVER_PORT=%d", p.Server),
		fmt.Sprintf("BROWSEROS_CLAW_CDP_PORT=%d", p.CDP),
		fmt.Sprintf("VITE_BROWSEROS_CLAW_API_URL=%s", apiURL),
	)
}

// startBrowserOSWatch supervises the BrowserOS agent extension plus server dev pair.
func startBrowserOSWatch(ctx context.Context, wg *sync.WaitGroup, root string, env []string, p proc.Ports, reservations *proc.PortReservations, userDataDir string, manual bool) ([]*proc.ManagedProc, error) {
	var procs []*proc.ManagedProc
	agentDir := filepath.Join(root, "apps/app")

	if manual {
		proc.LogMsg(proc.TagBuild, "Building agent (dev)...")
		if err := proc.RunBlocking(ctx, agentDir, proc.TagBuild,
			"bun", "--env-file=.env.development", "wxt", "build", "--mode", "development"); err != nil {
			return nil, fmt.Errorf("agent build failed: %w", err)
		}
		proc.LogMsg(proc.TagBuild, "agent built")

		reservations.ReleaseCDP()
		procs = append(procs, proc.StartManaged(ctx, wg, proc.ProcConfig{
			Tag:     proc.TagBrowser,
			Dir:     root,
			Restart: false,
			Cmd: browser.BuildArgs(browser.ArgsConfig{
				Root:              root,
				Ports:             p,
				UserDataDir:       userDataDir,
				LoadDevExtensions: true,
			}),
		}))
	} else {
		reservations.ReleaseCDP()
		procs = append(procs, proc.StartManaged(ctx, wg, proc.ProcConfig{
			Tag:     proc.TagAgent,
			Dir:     agentDir,
			Env:     env,
			Restart: true,
			Cmd:     []string{"bun", "--env-file=.env.development", "wxt"},
		}))
	}

	waitForCDP(ctx, p.CDP)

	reservations.ReleaseServer()
	reservations.ReleaseExtension()
	procs = append(procs, proc.StartManaged(ctx, wg, proc.ProcConfig{
		Tag:     proc.TagServer,
		Dir:     filepath.Join(root, "apps/server"),
		Env:     env,
		Restart: true,
		Cmd:     []string{"bun", "--watch", "--env-file=.env.development", "src/index.ts"},
		BeforeStart: func() error {
			return proc.KillPortAndWait(p.Server, 3*time.Second)
		},
	}))
	return procs, nil
}

// startClawWatch supervises the BrowserClaw UI plus standalone server.
func startClawWatch(ctx context.Context, wg *sync.WaitGroup, root string, env []string, p proc.Ports, reservations *proc.PortReservations) []*proc.ManagedProc {
	var procs []*proc.ManagedProc

	reservations.ReleaseCDP()
	procs = append(procs, proc.StartManaged(ctx, wg, proc.ProcConfig{
		Tag:     proc.TagAgent,
		Dir:     filepath.Join(root, "apps/claw-app"),
		Env:     env,
		Restart: true,
		Cmd:     []string{"bun", "--env-file=.env.development", "wxt"},
	}))

	waitForCDP(ctx, p.CDP)

	reservations.ReleaseServer()
	reservations.ReleaseExtension()
	procs = append(procs, proc.StartManaged(ctx, wg, proc.ProcConfig{
		Tag:     proc.TagServer,
		Dir:     filepath.Join(root, "apps/claw-server"),
		Env:     env,
		Restart: true,
		Cmd:     []string{"bun", "--watch", "--env-file=.env.development", "src/main.ts"},
		BeforeStart: func() error {
			return proc.KillPortAndWait(p.Server, 3*time.Second)
		},
	}))
	return procs
}

func waitForCDP(ctx context.Context, port int) {
	proc.LogMsg(proc.TagServer, "Waiting for CDP...")
	if browser.WaitForCDP(ctx, port, 60) {
		proc.LogMsg(proc.TagServer, "CDP ready")
	} else {
		proc.LogMsg(proc.TagServer, proc.WarnColor.Sprint("CDP not available, starting server anyway"))
	}
}

func ensureLimactlPresent() error {
	if _, err := exec.LookPath("limactl"); err != nil {
		return fmt.Errorf("%s %s",
			proc.ErrorColor.Sprint("Lima is not installed."),
			proc.DimColor.Sprintf("Install with %s.", proc.BoldColor.Sprint("brew install lima")),
		)
	}
	return nil
}
