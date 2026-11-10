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
		cfg, err := loadConfig()
		if err != nil {
			return err
		}
		if err := promptIfSourceProfileInUse(cmd.OutOrStdout(), bufio.NewReader(os.Stdin), cfg, startRefreshProfile); err != nil {
			return err
		}
		paths, err := defaultRunPaths()
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
		cfg, err := loadConfig()
		if err != nil {
			return err
		}
		if err := promptIfSourceProfileInUse(cmd.OutOrStdout(), bufio.NewReader(os.Stdin), cfg, startBackgroundRefreshProfile); err != nil {
			return err
		}
		paths, err := defaultRunPaths()
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
		return startBackgroundProcess(paths, startBackgroundHeadless, startBackgroundRefreshProfile)
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

// buildAndStartEnvironment prepares the checkout and profile, then starts the supervised processes.
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
	reportProgress(opts, "preparing profile")
	if err := prepareProfile(&cfg, opts); err != nil {
		return nil, err
	}
	if err := resolveEnvironmentPorts(&cfg); err != nil {
		return nil, err
	}
	if err := resolveStartBrowserApp(&cfg); err != nil {
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
			return false, fmt.Errorf("switch to configured branch %s failed; run `browseros-dogfood pull` first if the branch is not available locally: %w", cfg.Branch, err)
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

func resolveEnvironmentPorts(cfg *config.Config) error {
	resolvedPorts, changed, err := proc.ResolvePorts(cfg.Ports)
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
		proc.LogMsgf(proc.TagInfo, "Busy ports detected; using %s", formatPorts(*cfg))
	} else {
		proc.LogMsgf(proc.TagInfo, "Using ports %s", formatPorts(*cfg))
	}
	return nil
}

func reportProgress(opts environmentOptions, message string) {
	if opts.Progress != nil {
		opts.Progress(message)
	}
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
	if err := writeDogfoodSidecarConfig(sidecarPath, cfg, filepath.Join(agentRoot, "apps/claw-server/resources")); err != nil {
		e.Stop()
		e.Wait()
		return nil, fmt.Errorf("write Claw server config: %w", err)
	}
	reportProgress(opts, "starting Claw server")
	e.managed = append(e.managed, proc.StartManaged(ctx, &e.wg, proc.ProcConfig{
		Tag:         proc.TagServer,
		Dir:         agentRoot,
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

func clawAppCommand() []string {
	return []string{"bun", "--env-file=../../.env.development", "wxt"}
}

func clawServerCommand(configPath string) []string {
	return []string{"cargo", "run", "-p", "claw-server", "--", "--config", configPath}
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

func resolveStartBrowserApp(cfg *config.Config) error {
	resolution := resolveConfiguredBrowserApp(cfg, browserAppExecutableExists)
	if err := validateBrowserAppExecutable(resolution.Path); err != nil {
		return err
	}
	if resolution.Fallback {
		proc.LogMsg(proc.TagInfo, proc.WarnColor.Sprintf("Browser app unavailable at %s; using %s", resolution.MissingPath, resolution.Path))
	}
	return nil
}

func resolveConfiguredBrowserApp(cfg *config.Config, exists func(string) bool) config.BrowserAppResolution {
	resolution := config.ResolveBrowserAppPath(cfg.BrowserOSAppPath, exists)
	cfg.BrowserOSAppPath = resolution.Path
	return resolution
}

func browserAppExecutableExists(path string) bool {
	info, err := os.Stat(path)
	return err == nil && !info.IsDir() && info.Mode()&0111 != 0
}

func validateBrowserAppExecutable(path string) error {
	info, err := os.Stat(path)
	if err != nil {
		return fmt.Errorf("browseros_app_path: %w", err)
	}
	if info.IsDir() || info.Mode()&0111 == 0 {
		return fmt.Errorf("browseros_app_path is not an executable file: %s", path)
	}
	return nil
}

func printSummary(cfg config.Config, agentRoot string) {
	fmt.Println()
	proc.LogMsgf(proc.TagInfo, "Target: %s", productLabel)
	proc.LogMsgf(proc.TagInfo, "App: %s", cfg.BrowserOSAppPath)
	proc.LogMsgf(proc.TagInfo, "Repo: %s", cfg.RepoPath)
	proc.LogMsgf(proc.TagInfo, "Agent root: %s", agentRoot)
	proc.LogMsgf(proc.TagInfo, "Profile: %s", cfg.DevUserDataDir)
	proc.LogMsgf(proc.TagInfo, "Claw state: %s", cfg.BrowserOSDir)
	proc.LogMsgf(proc.TagInfo, "Logs: %s", cfg.LogDir())
	proc.LogMsgf(proc.TagInfo, "Ports: %s", formatPorts(cfg))
	fmt.Println()
}

func formatPorts(cfg config.Config) string {
	return fmt.Sprintf("CDP=%d API=%d", cfg.Ports.CDP, cfg.Ports.Server)
}
