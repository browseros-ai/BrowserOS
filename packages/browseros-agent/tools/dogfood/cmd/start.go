package cmd

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"os"
	"os/signal"
	"path/filepath"
	"strings"
	"sync"
	"syscall"
	"time"

	"browseros-dogfood/browser"
	"browseros-dogfood/config"
	"browseros-dogfood/pipeline"
	"browseros-dogfood/proc"
	"browseros-dogfood/profile"
	dogfoodruntime "browseros-dogfood/runtime"

	"github.com/spf13/cobra"
)

var startRefreshProfile bool
var startHeadless bool
var startBackgroundRefreshProfile bool
var startBackgroundHeadless bool

const (
	serverLogName     = "server.log"
	chromiumLogName   = "chromium.log"
	clawAppLogName    = "claw-app.log"
	clawServerLogName = "claw-server.log"
)

func init() {
	startCmd.Flags().BoolVar(&startRefreshProfile, "refresh-profile", false, "Refresh copied BrowserOS profile before launch")
	startCmd.Flags().BoolVar(&startHeadless, "headless", false, "Run BrowserOS headless")
	startBackgroundCmd.Flags().BoolVar(&startBackgroundRefreshProfile, "refresh-profile", false, "Refresh copied BrowserOS profile before launch")
	startBackgroundCmd.Flags().BoolVar(&startBackgroundHeadless, "headless", false, "Run BrowserOS headless")
	rootCmd.AddCommand(startCmd)
	rootCmd.AddCommand(startBackgroundCmd)
}

var startCmd = &cobra.Command{
	Use:     "start",
	Short:   "Start dogfooding environment",
	GroupID: groupRun,
	RunE: func(cmd *cobra.Command, args []string) error {
		target, cfg, err := loadSelectedTargetConfig()
		if err != nil {
			return err
		}
		if err := promptIfSourceProfileInUse(cmd.OutOrStdout(), bufio.NewReader(os.Stdin), cfg, startRefreshProfile); err != nil {
			return err
		}
		paths, err := defaultTargetRunPaths(target)
		if err != nil {
			return err
		}
		lock, err := acquireRunLock(paths, "foreground")
		if err != nil {
			return err
		}
		defer lock.Close()
		defer dogfoodruntime.CleanupStaleRunFiles(paths.State)
		return runEnvironment(cfg, environmentOptions{
			RefreshProfile: startRefreshProfile,
			Headless:       startHeadless,
			RestartBrowser: false,
			Runner:         pipeline.ExecRunner{},
		})
	},
}

var startBackgroundCmd = &cobra.Command{
	Use:     "start-background",
	Short:   "Start dogfooding environment in the background",
	GroupID: groupRun,
	RunE: func(cmd *cobra.Command, args []string) error {
		target, cfg, err := loadSelectedTargetConfig()
		if err != nil {
			return err
		}
		if err := promptIfSourceProfileInUse(cmd.OutOrStdout(), bufio.NewReader(os.Stdin), cfg, startBackgroundRefreshProfile); err != nil {
			return err
		}
		paths, err := defaultTargetRunPaths(target)
		if err != nil {
			return err
		}
		if lock, err := dogfoodruntime.AcquireLock(paths.Lock); err == nil {
			_ = lock.Close()
			if err := dogfoodruntime.CleanupStaleRunFiles(paths.State); err != nil {
				return err
			}
		} else if errors.Is(err, dogfoodruntime.ErrAlreadyRunning) {
			return runningError(paths)
		} else {
			return err
		}
		return startBackgroundProcess(paths, target, startBackgroundHeadless, startBackgroundRefreshProfile)
	},
}

type environmentOptions struct {
	RefreshProfile bool
	Headless       bool
	RestartBrowser bool
	LineHandler    proc.LineHandler
	Progress       func(string)
	Runner         pipeline.Runner
}

type environment struct {
	cancel  context.CancelFunc
	wg      sync.WaitGroup
	managed []*proc.ManagedProc
	cfg     config.Config
}

func runEnvironment(cfg config.Config, opts environmentOptions) error {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	env, err := buildAndStartEnvironment(ctx, cfg, opts)
	if err != nil {
		return err
	}
	defer env.Stop()

	sigCh := make(chan os.Signal, 2)
	signal.Notify(sigCh, syscall.SIGINT, syscall.SIGTERM, syscall.SIGQUIT)
	<-sigCh
	fmt.Println()
	proc.LogMsg(proc.TagInfo, proc.WarnColor.Sprint("Shutting down (Ctrl+C again to force)..."))
	cancel()
	done := make(chan struct{})
	go func() {
		env.Wait()
		close(done)
	}()
	go func() {
		select {
		case <-sigCh:
			env.ForceKill()
			os.Exit(1)
		case <-done:
		}
	}()
	env.Stop()
	select {
	case <-done:
	case <-time.After(10 * time.Second):
		env.ForceKill()
	}
	return nil
}

// buildAndStartEnvironment prepares the selected target and starts its supervised processes.
func buildAndStartEnvironment(ctx context.Context, cfg config.Config, opts environmentOptions) (*environment, error) {
	if opts.Runner == nil {
		opts.Runner = pipeline.ExecRunner{}
	}
	agentRoot := cfg.AgentRoot()
	reportProgress(opts, "checking repo")
	if dirty, err := prepareStartCheckout(ctx, cfg, opts.Runner); err != nil {
		return nil, err
	} else if dirty {
		fmt.Fprintln(os.Stderr, warnStyle.Sprint("warning: checkout has uncommitted changes; start will use current files"))
	}
	switch cfg.Target {
	case config.TargetBrowserOS:
		return buildAndStartBrowserOSEnvironment(ctx, cfg, agentRoot, opts)
	case config.TargetClaw:
		return buildAndStartClawEnvironment(ctx, cfg, agentRoot, opts)
	default:
		return nil, fmt.Errorf("unknown dogfood target %q", cfg.Target)
	}
}

func buildAndStartBrowserOSEnvironment(ctx context.Context, cfg config.Config, agentRoot string, opts environmentOptions) (*environment, error) {
	reportProgress(opts, "preparing profile")
	if err := prepareBrowserOSEnvironment(&cfg, agentRoot, opts); err != nil {
		return nil, err
	}
	reportProgress(opts, "building agent")
	if err := pipeline.Build(ctx, agentRoot, opts.Runner); err != nil {
		return nil, err
	}
	return startBrowserOSEnvironment(ctx, cfg, agentRoot, opts)
}

func buildAndStartClawEnvironment(ctx context.Context, cfg config.Config, agentRoot string, opts environmentOptions) (*environment, error) {
	reportProgress(opts, "preparing profile")
	if err := prepareClawEnvironment(&cfg, opts); err != nil {
		return nil, err
	}
	reportProgress(opts, "preparing Claw apps")
	if err := pipeline.Setup(ctx, agentRoot, opts.Runner); err != nil {
		return nil, err
	}
	return startClawEnvironment(ctx, cfg, agentRoot, opts)
}

// prepareStartCheckout ensures start builds the configured branch without discarding local edits.
func prepareStartCheckout(ctx context.Context, cfg config.Config, runner pipeline.Runner) (bool, error) {
	dirty, err := pipeline.Dirty(cfg.RepoPath, runner)
	if err != nil {
		return false, err
	}
	if !dirty {
		if err := pipeline.EnsureBranch(ctx, cfg.RepoPath, cfg.Branch, runner, false); err != nil {
			return false, fmt.Errorf("switch to configured branch %s failed; run `browseros-dogfood %s pull` first if the branch is not available locally: %w", cfg.Branch, targetFlagOrDefault(cfg.Target), err)
		}
		return false, nil
	}
	current := pipeline.Branch(cfg.RepoPath, runner)
	if current != cfg.Branch {
		return true, fmt.Errorf("checkout has uncommitted changes on %s; cannot switch to configured branch %s", current, cfg.Branch)
	}
	return true, nil
}

func prepareProfile(cfg *config.Config, opts environmentOptions) error {
	if profileImportNeeded(*cfg, opts.RefreshProfile) {
		if err := profile.Import(profile.ImportConfig{
			SourceUserDataDir: cfg.SourceUserDataDir,
			SourceProfileDir:  cfg.SourceProfileDir,
			DevUserDataDir:    cfg.DevUserDataDir,
			DevProfileDir:     cfg.DevProfileDir,
		}); err != nil {
			return err
		}
	} else if err := profile.CleanupSingletons(cfg.DevUserDataDir); err != nil {
		return err
	}
	return nil
}

func prepareBrowserOSEnvironment(cfg *config.Config, agentRoot string, opts environmentOptions) error {
	if err := prepareProfile(cfg, opts); err != nil {
		return err
	}
	if err := pipeline.WriteProductionEnvFiles(agentRoot, *cfg); err != nil {
		return err
	}
	return resolveEnvironmentPorts(cfg, true)
}

func prepareClawEnvironment(cfg *config.Config, opts environmentOptions) error {
	if err := prepareProfile(cfg, opts); err != nil {
		return err
	}
	return resolveEnvironmentPorts(cfg, false)
}

func resolveEnvironmentPorts(cfg *config.Config, includeExtension bool) error {
	resolvedPorts, changed, err := proc.ResolveTargetPorts(cfg.Ports, includeExtension)
	if err != nil {
		return err
	}
	cfg.Ports = resolvedPorts
	if changed {
		path, err := config.Path()
		if err != nil {
			return err
		}
		if err := config.Save(path, *cfg); err != nil {
			return err
		}
		proc.LogMsgf(proc.TagInfo, "Busy ports detected; using %s", formatPortsForTarget(*cfg))
	} else {
		proc.LogMsgf(proc.TagInfo, "Using ports %s", formatPortsForTarget(*cfg))
	}
	return nil
}

func reportProgress(opts environmentOptions, message string) {
	if opts.Progress != nil {
		opts.Progress(message)
	}
}

func startBrowserOSEnvironment(parent context.Context, cfg config.Config, agentRoot string, opts environmentOptions) (*environment, error) {
	ctx, cancel := context.WithCancel(parent)
	e := &environment{cancel: cancel, cfg: cfg}
	reportProgress(opts, "launching Chromium")
	e.managed = append(e.managed, proc.StartManaged(ctx, &e.wg, proc.ProcConfig{
		Tag:     proc.TagBrowser,
		Dir:     agentRoot,
		Restart: opts.RestartBrowser,
		LogPath: cfg.LogPath(chromiumLogName),
		Cmd: browser.BuildArgs(browser.ArgsConfig{
			Binary:      cfg.BrowserOSAppPath,
			AgentRoot:   agentRoot,
			UserDataDir: cfg.DevUserDataDir,
			ProfileDir:  cfg.DevProfileDir,
			Ports:       cfg.Ports,
			Headless:    opts.Headless,
		}),
		LineHandler: opts.LineHandler,
	}))
	reportProgress(opts, "waiting for CDP")
	proc.LogMsg(proc.TagServer, "Waiting for CDP...")
	if browser.WaitForCDP(ctx, cfg.Ports.CDP, 60) {
		reportProgress(opts, "CDP ready")
		proc.LogMsg(proc.TagServer, "CDP ready")
	} else {
		reportProgress(opts, "CDP not available, starting server anyway")
		proc.LogMsg(proc.TagServer, proc.WarnColor.Sprint("CDP not available, starting server anyway"))
	}
	runtimeEnv := serverRuntimeEnv(os.Environ(), cfg)
	serverDir := filepath.Join(agentRoot, "apps/server")
	sidecarPath := dogfoodSidecarConfigPath(cfg)
	if err := writeDogfoodSidecarConfig(sidecarPath, cfg, agentRoot); err != nil {
		e.Stop()
		e.Wait()
		return nil, fmt.Errorf("write server config: %w", err)
	}
	reportProgress(opts, "starting server")
	e.managed = append(e.managed, proc.StartManaged(ctx, &e.wg, proc.ProcConfig{
		Tag:         proc.TagServer,
		Dir:         serverDir,
		Env:         runtimeEnv,
		Restart:     true,
		LogPath:     cfg.LogPath(serverLogName),
		Cmd:         serverCommand(sidecarPath),
		LineHandler: opts.LineHandler,
	}))
	printSummary(cfg, agentRoot)
	return e, nil
}

func startClawEnvironment(parent context.Context, cfg config.Config, agentRoot string, opts environmentOptions) (*environment, error) {
	ctx, cancel := context.WithCancel(parent)
	e := &environment{cancel: cancel, cfg: cfg}
	runtimeEnv := clawRuntimeEnv(os.Environ(), cfg)

	reportProgress(opts, "starting Claw WXT")
	e.managed = append(e.managed, proc.StartManaged(ctx, &e.wg, proc.ProcConfig{
		Tag:         proc.TagAgent,
		Dir:         filepath.Join(agentRoot, "apps/claw-app"),
		Env:         runtimeEnv,
		Restart:     true,
		LogPath:     cfg.LogPath(clawAppLogName),
		Cmd:         clawAppCommand(),
		LineHandler: opts.LineHandler,
	}))

	reportProgress(opts, "waiting for CDP")
	proc.LogMsg(proc.TagServer, "Waiting for CDP...")
	if browser.WaitForCDP(ctx, cfg.Ports.CDP, 60) {
		reportProgress(opts, "CDP ready")
		proc.LogMsg(proc.TagServer, "CDP ready")
	} else {
		reportProgress(opts, "CDP not available, starting server anyway")
		proc.LogMsg(proc.TagServer, proc.WarnColor.Sprint("CDP not available, starting server anyway"))
	}

	sidecarPath := dogfoodSidecarConfigPath(cfg)
	if err := writeDogfoodSidecarConfig(sidecarPath, cfg, agentRoot); err != nil {
		e.Stop()
		e.Wait()
		return nil, fmt.Errorf("write Claw server config: %w", err)
	}
	reportProgress(opts, "starting Claw server")
	e.managed = append(e.managed, proc.StartManaged(ctx, &e.wg, proc.ProcConfig{
		Tag:         proc.TagServer,
		Dir:         filepath.Join(agentRoot, "apps/claw-server"),
		Env:         runtimeEnv,
		Restart:     true,
		LogPath:     cfg.LogPath(clawServerLogName),
		Cmd:         clawServerCommand(sidecarPath),
		LineHandler: opts.LineHandler,
	}))
	printSummary(cfg, agentRoot)
	return e, nil
}

func (e *environment) Stop() {
	if e == nil {
		return
	}
	e.cancel()
	for _, p := range e.managed {
		p.Stop()
	}
}

func (e *environment) Wait() {
	if e == nil {
		return
	}
	e.wg.Wait()
}

func (e *environment) ForceKill() {
	if e == nil {
		return
	}
	for _, p := range e.managed {
		p.ForceKill()
	}
}

func serverCommand(configPath string) []string {
	return []string{"bun", "--env-file=.env.development", "src/index.ts", "--config", configPath}
}

func clawAppCommand() []string {
	return []string{"bun", "--env-file=.env.development", "wxt"}
}

func clawServerCommand(configPath string) []string {
	return []string{"bun", "--watch", "--env-file=.env.development", "src/main.ts", "--config", configPath}
}

func serverRuntimeEnv(base []string, cfg config.Config) []string {
	env := make([]string, 0, len(base)+2)
	for _, entry := range base {
		if strings.HasPrefix(entry, "BROWSEROS_DIR=") {
			continue
		}
		env = append(env, entry)
	}
	return append(env,
		"NODE_ENV=development",
		fmt.Sprintf("BROWSEROS_DIR=%s", cfg.BrowserOSDir),
	)
}

// clawRuntimeEnv wires the WXT app and standalone Claw server to the same BrowserClaw state.
func clawRuntimeEnv(base []string, cfg config.Config) []string {
	env := make([]string, 0, len(base)+7)
	for _, entry := range base {
		if strings.HasPrefix(entry, "BROWSEROS_DIR=") ||
			strings.HasPrefix(entry, "BROWSERCLAW_DIR=") ||
			strings.HasPrefix(entry, "BROWSEROS_BINARY=") ||
			strings.HasPrefix(entry, "BROWSEROS_USER_DATA_DIR=") ||
			strings.HasPrefix(entry, "BROWSEROS_CLAW_CDP_PORT=") ||
			strings.HasPrefix(entry, "BROWSEROS_SERVER_PORT=") ||
			strings.HasPrefix(entry, "VITE_BROWSEROS_CLAW_API_URL=") {
			continue
		}
		env = append(env, entry)
	}
	apiURL := fmt.Sprintf("http://127.0.0.1:%d", cfg.Ports.Server)
	return append(env,
		"NODE_ENV=development",
		fmt.Sprintf("BROWSERCLAW_DIR=%s", cfg.BrowserOSDir),
		fmt.Sprintf("BROWSEROS_BINARY=%s", cfg.BrowserOSAppPath),
		fmt.Sprintf("BROWSEROS_USER_DATA_DIR=%s", cfg.DevUserDataDir),
		fmt.Sprintf("BROWSEROS_CLAW_CDP_PORT=%d", cfg.Ports.CDP),
		fmt.Sprintf("BROWSEROS_SERVER_PORT=%d", cfg.Ports.Server),
		fmt.Sprintf("VITE_BROWSEROS_CLAW_API_URL=%s", apiURL),
	)
}

func exists(path string) bool {
	_, err := os.Stat(path)
	return err == nil
}

func printSummary(cfg config.Config, agentRoot string) {
	fmt.Println()
	proc.LogMsgf(proc.TagInfo, "Target: %s", targetLabel(cfg.Target))
	proc.LogMsgf(proc.TagInfo, "App: %s", cfg.BrowserOSAppPath)
	proc.LogMsgf(proc.TagInfo, "Repo: %s", cfg.RepoPath)
	proc.LogMsgf(proc.TagInfo, "Agent root: %s", agentRoot)
	proc.LogMsgf(proc.TagInfo, "Profile: %s", cfg.DevUserDataDir)
	proc.LogMsgf(proc.TagInfo, "%s: %s", stateDirLabel(cfg.Target), cfg.BrowserOSDir)
	proc.LogMsgf(proc.TagInfo, "Logs: %s", cfg.LogDir())
	proc.LogMsgf(proc.TagInfo, "Ports: %s", formatPortsForTarget(cfg))
	fmt.Println()
}

func targetLabel(target config.Target) string {
	switch target {
	case config.TargetClaw:
		return "BrowserClaw"
	default:
		return "BrowserOS"
	}
}

func stateDirLabel(target config.Target) string {
	if target == config.TargetClaw {
		return "Claw state"
	}
	return "BrowserOS dir"
}

func formatPortsForTarget(cfg config.Config) string {
	if cfg.Target == config.TargetClaw {
		return fmt.Sprintf("CDP=%d API=%d", cfg.Ports.CDP, cfg.Ports.Server)
	}
	return fmt.Sprintf("CDP=%d Server=%d Extension=%d", cfg.Ports.CDP, cfg.Ports.Server, cfg.Ports.Extension)
}
