package cmd

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"sync"
	"syscall"
	"time"

	"browseros-dogfood/config"
	"browseros-dogfood/ipc"
	"browseros-dogfood/pipeline"
	"browseros-dogfood/proc"
	"browseros-dogfood/runlog"
	dogfoodruntime "browseros-dogfood/runtime"

	"github.com/spf13/cobra"
)

type runPaths struct {
	Dir    string
	Lock   string
	State  string
	Socket string
	Log    string
	RawLog string
}

var daemonHeadless bool
var daemonRefreshProfile bool

const (
	serverHealthAttempts = 120
	serverHealthInterval = 500 * time.Millisecond
)

var daemonCmd = &cobra.Command{
	Use:    "daemon",
	Short:  "Run the browseros-dogfood background daemon",
	Hidden: true,
	RunE:   runDaemon,
}

func init() {
	daemonCmd.Flags().BoolVar(&daemonHeadless, "headless", false, "Run BrowserOS headless")
	daemonCmd.Flags().BoolVar(&daemonRefreshProfile, "refresh-profile", false, "Refresh copied BrowserOS profile before launch")
	rootCmd.AddCommand(daemonCmd)
}

func newRunPaths(configPath string) runPaths {
	dir := filepath.Dir(configPath)
	return runPaths{
		Dir:    dir,
		Lock:   filepath.Join(dir, "run.lock"),
		State:  filepath.Join(dir, "state.json"),
		Socket: filepath.Join(dir, "daemon.sock"),
		Log:    filepath.Join(dir, "daemon.jsonl"),
		RawLog: filepath.Join(dir, "daemon.log"),
	}
}

func defaultRunPaths() (runPaths, error) {
	path, err := config.Path()
	if err != nil {
		return runPaths{}, err
	}
	return newRunPaths(path), nil
}

func daemonArgs(target config.Target, headless bool) []string {
	targetFlag, err := selectedTargetFlag(target)
	if err != nil {
		targetFlag = "--browseros"
	}
	args := []string{targetFlag, "daemon"}
	if headless {
		args = append(args, "--headless")
	}
	return args
}

func daemonArgsWithOptions(target config.Target, headless bool, refreshProfile bool) []string {
	args := daemonArgs(target, headless)
	if refreshProfile {
		args = append(args, "--refresh-profile")
	}
	return args
}

func acquireRunLock(paths runPaths, mode string) (*dogfoodruntime.Lock, error) {
	lock, err := dogfoodruntime.AcquireLock(paths.Lock)
	if err != nil {
		if errors.Is(err, dogfoodruntime.ErrAlreadyRunning) {
			return nil, runningError(paths)
		}
		return nil, err
	}
	if err := dogfoodruntime.CleanupStaleRunFiles(paths.State); err != nil {
		lock.Close()
		return nil, err
	}
	socketPath := ""
	logPath := ""
	if mode == "background" {
		socketPath = paths.Socket
		logPath = paths.Log
	}
	if err := dogfoodruntime.WriteRunState(paths.State, dogfoodruntime.RunState{
		PID:        os.Getpid(),
		Mode:       mode,
		StartedAt:  time.Now(),
		SocketPath: socketPath,
		LogPath:    logPath,
	}); err != nil {
		lock.Close()
		return nil, err
	}
	return lock, nil
}

func runningError(paths runPaths) error {
	state, err := dogfoodruntime.ReadRunState(paths.State)
	if err == nil {
		if state.Mode == "background" {
			return fmt.Errorf("browseros-dogfood background daemon is already running (pid %d)", state.PID)
		}
		return fmt.Errorf("browseros-dogfood is already running in foreground mode (pid %d)", state.PID)
	}
	return fmt.Errorf("browseros-dogfood is already running")
}

func startBackgroundProcess(paths runPaths, target config.Target, headless bool, refreshProfile bool) error {
	exe, err := os.Executable()
	if err != nil {
		return err
	}
	if resolved, err := filepath.EvalSymlinks(exe); err == nil {
		exe = resolved
	}
	if err := os.MkdirAll(paths.Dir, 0755); err != nil {
		return err
	}
	rawLog, err := os.OpenFile(paths.RawLog, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0644)
	if err != nil {
		return err
	}
	defer rawLog.Close()

	cmd := exec.Command(exe, daemonArgsWithOptions(target, headless, refreshProfile)...)
	cmd.Stdout = rawLog
	cmd.Stderr = rawLog
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	if err := cmd.Start(); err != nil {
		return err
	}

	done := make(chan error, 1)
	go func() { done <- cmd.Wait() }()

	deadline := time.After(5 * time.Second)
	tick := time.NewTicker(100 * time.Millisecond)
	defer tick.Stop()
	for {
		select {
		case err := <-done:
			if err != nil {
				return fmt.Errorf("background daemon exited during startup: %w; see %s", err, paths.RawLog)
			}
			return fmt.Errorf("background daemon exited during startup; see %s", paths.RawLog)
		case <-deadline:
			return fmt.Errorf("background daemon did not open its control socket; see %s", paths.RawLog)
		case <-tick.C:
			if resp, err := ipc.NewClient(paths.Socket).Send(ipc.Request{Command: ipc.CmdStatus}); err == nil && resp.OK {
				fmt.Printf("%s %s background daemon %s\n", successStyle.Sprint("Started:"), targetLabel(target), dimStyle.Sprintf("(pid %d)", cmd.Process.Pid))
				fmt.Fprintln(os.Stdout, dimStyle.Sprint("Streaming startup logs until healthy..."))
				detach, cleanup := newInterruptDetach()
				defer cleanup()
				detached := false
				if err := monitorDaemonUntilRunning(context.Background(), daemonMonitor{
					Paths:     paths,
					Target:    target,
					Out:       os.Stdout,
					FromStart: true,
					Detach:    detach,
					Detached:  &detached,
				}); err != nil {
					return err
				}
				if detached {
					return nil
				}
				targetFlag, _ := selectedTargetFlag(target)
				fmt.Printf("%s %s background environment is healthy\n", successStyle.Sprint("Ready:"), targetLabel(target))
				fmt.Printf("  %s %s\n", labelStyle.Sprint("Status:"), commandStyle.Sprintf("browseros-dogfood %s status", targetFlag))
				fmt.Printf("  %s   %s\n", labelStyle.Sprint("Logs:"), commandStyle.Sprintf("browseros-dogfood %s logs tail", targetFlag))
				fmt.Printf("  %s   %s\n", labelStyle.Sprint("Stop:"), commandStyle.Sprintf("browseros-dogfood %s stop", targetFlag))
				return nil
			}
		}
	}
}

func newInterruptDetach() (<-chan struct{}, func()) {
	sigCh := make(chan os.Signal, 1)
	done := make(chan struct{})
	detach := make(chan struct{})
	var detachOnce sync.Once
	signal.Notify(sigCh, os.Interrupt)
	go func() {
		select {
		case <-sigCh:
			detachOnce.Do(func() { close(detach) })
		case <-done:
		}
	}()
	return detach, func() {
		signal.Stop(sigCh)
		close(done)
	}
}

type dogfoodDaemon struct {
	ctx       context.Context
	cancel    context.CancelFunc
	paths     runPaths
	logWriter *runlog.Writer

	opMu sync.Mutex
	mu   sync.RWMutex

	env          *environment
	target       config.Target
	state        string
	operation    string
	lastError    string
	ports        config.Ports
	startedAt    time.Time
	headless     bool
	browserOSDir string
}

type daemonStatus struct {
	Target       string       `json:"target"`
	State        string       `json:"state"`
	Operation    string       `json:"operation,omitempty"`
	LastError    string       `json:"last_error,omitempty"`
	PID          int          `json:"pid"`
	Uptime       string       `json:"uptime"`
	Ports        config.Ports `json:"ports"`
	BrowserOSDir string       `json:"browseros_dir"`
	StateDir     string       `json:"state_dir"`
	LogPath      string       `json:"log_path"`
}

type healthResponse struct {
	CDPConnected *bool `json:"cdpConnected"`
}

func runDaemon(cmd *cobra.Command, args []string) error {
	target, cfg, err := loadSelectedTargetConfig()
	if err != nil {
		return err
	}
	paths, err := defaultTargetRunPaths(target)
	if err != nil {
		return err
	}
	lock, err := acquireRunLock(paths, "background")
	if err != nil {
		return err
	}
	defer lock.Close()
	defer dogfoodruntime.CleanupStaleRunFiles(paths.State)

	if err := os.Remove(paths.Log); err != nil && !os.IsNotExist(err) {
		return err
	}
	logWriter, err := runlog.NewWriter(paths.Log)
	if err != nil {
		return err
	}
	defer logWriter.Close()

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	d := &dogfoodDaemon{
		ctx:          ctx,
		cancel:       cancel,
		paths:        paths,
		logWriter:    logWriter,
		target:       target,
		state:        "starting",
		startedAt:    time.Now(),
		headless:     daemonHeadless,
		ports:        cfg.Ports,
		browserOSDir: cfg.BrowserOSDir,
	}

	server := ipc.NewServer(paths.Socket, d)
	if err := server.Start(); err != nil {
		return err
	}
	defer server.Stop()

	sigCh := make(chan os.Signal, 1)
	signal.Notify(sigCh, syscall.SIGINT, syscall.SIGTERM, syscall.SIGQUIT)
	go func() {
		select {
		case <-sigCh:
			cancel()
		case <-ctx.Done():
		}
	}()

	if err := d.startLockedOperation("starting", daemonRefreshProfile); err != nil {
		proc.LogMsg(proc.TagInfo, proc.ErrorColor.Sprintf("Startup failed: %v", err))
	}

	<-ctx.Done()
	d.stopEnvironment()
	return nil
}

func (d *dogfoodDaemon) Handle(req ipc.Request) ipc.Response {
	switch req.Command {
	case ipc.CmdStatus:
		return ipc.Response{OK: true, Data: d.status()}
	case ipc.CmdStop:
		go func() {
			time.Sleep(100 * time.Millisecond)
			d.cancel()
		}()
		return ipc.Response{OK: true, Data: map[string]string{"state": "stopping"}}
	case ipc.CmdRestart:
		pull := req.Args["pull"] == "true"
		force := req.Args["force"] == "true"
		if err := d.scheduleRestart(pull, force); err != nil {
			return ipc.Response{Error: err.Error()}
		}
		return ipc.Response{OK: true, Data: map[string]string{"state": "restarting"}}
	default:
		return ipc.Response{Error: fmt.Sprintf("unknown command: %s", req.Command)}
	}
}

func (d *dogfoodDaemon) status() daemonStatus {
	d.mu.RLock()
	defer d.mu.RUnlock()
	return daemonStatus{
		Target:       string(d.target),
		State:        d.state,
		Operation:    d.operation,
		LastError:    d.lastError,
		PID:          os.Getpid(),
		Uptime:       time.Since(d.startedAt).Round(time.Second).String(),
		Ports:        d.ports,
		BrowserOSDir: d.browserOSDir,
		StateDir:     d.browserOSDir,
		LogPath:      d.paths.Log,
	}
}

func (d *dogfoodDaemon) scheduleRestart(pull bool, force bool) error {
	if force && !pull {
		return fmt.Errorf("--force requires --pull")
	}
	return d.scheduleOperation("restarting", func() error {
		if pull {
			cfg, err := loadTargetConfig(d.target)
			if err != nil {
				return err
			}
			runner := pipeline.ExecRunner{}
			if err := updateConfiguredRepo(d.ctx, cfg, runner, repoUpdateOptions{
				Force:           force,
				ResetToUpstream: force,
			}); err != nil {
				return err
			}
		}
		return d.startLocked(false)
	})
}

func (d *dogfoodDaemon) startLockedOperation(name string, refreshProfile bool) error {
	return d.withOperation(name, func() error {
		return d.startLocked(refreshProfile)
	})
}

func (d *dogfoodDaemon) withOperation(name string, fn func() error) error {
	if !d.opMu.TryLock() {
		return fmt.Errorf("daemon is already %s", d.currentOperation())
	}
	defer d.opMu.Unlock()

	d.setState(name, name, "")
	err := fn()
	if err != nil {
		d.logLifecycle("%s failed: %v", name, err)
		d.setState("error", "", err.Error())
		return err
	}
	d.setState("running", "", "")
	return nil
}

func (d *dogfoodDaemon) scheduleOperation(name string, fn func() error) error {
	if !d.opMu.TryLock() {
		return fmt.Errorf("daemon is already %s", d.currentOperation())
	}
	d.setState(name, name, "")
	go func() {
		defer d.opMu.Unlock()
		err := fn()
		if err != nil {
			d.logLifecycle("%s failed: %v", name, err)
			d.setState("error", "", err.Error())
			return
		}
		d.setState("running", "", "")
	}()
	return nil
}

func (d *dogfoodDaemon) startLocked(refreshProfile bool) error {
	cfg, err := loadTargetConfig(d.target)
	if err != nil {
		return err
	}
	d.stopEnvironment()
	opts := environmentOptions{
		RefreshProfile: refreshProfile,
		Headless:       d.headless,
		RestartBrowser: true,
		Runner:         pipeline.ExecRunner{},
		Progress: func(message string) {
			d.logLifecycle("%s", message)
		},
		LineHandler: func(tag proc.Tag, stream string, line string) {
			_ = d.logWriter.Append(tag.Name, stream, line)
		},
	}
	env, err := buildAndStartEnvironment(d.ctx, cfg, opts)
	if err != nil {
		return err
	}
	d.mu.Lock()
	d.env = env
	d.ports = env.cfg.Ports
	d.mu.Unlock()
	if err := d.waitUntilHealthy(env.cfg, serverHealthAttempts, serverHealthInterval); err != nil {
		return err
	}
	return nil
}

func (d *dogfoodDaemon) stopEnvironment() {
	d.mu.Lock()
	env := d.env
	d.env = nil
	d.mu.Unlock()
	if env == nil {
		return
	}
	env.Stop()
	done := make(chan struct{})
	go func() {
		env.Wait()
		close(done)
	}()
	select {
	case <-done:
	case <-time.After(10 * time.Second):
		env.ForceKill()
	}
}

func (d *dogfoodDaemon) setState(state string, operation string, lastError string) {
	d.mu.Lock()
	defer d.mu.Unlock()
	d.state = state
	d.operation = operation
	d.lastError = lastError
}

func (d *dogfoodDaemon) currentOperation() string {
	d.mu.RLock()
	defer d.mu.RUnlock()
	if d.operation == "" {
		return "busy"
	}
	return d.operation
}

func (d *dogfoodDaemon) logLifecycle(format string, args ...any) {
	if d == nil || d.logWriter == nil {
		return
	}
	_ = d.logWriter.Append("daemon", "lifecycle", fmt.Sprintf(format, args...))
}

func (d *dogfoodDaemon) waitUntilHealthy(cfg config.Config, maxAttempts int, interval time.Duration) error {
	d.logLifecycle("waiting for server health")
	if err := waitForServerHealth(d.ctx, cfg, maxAttempts, interval); err != nil {
		return err
	}
	d.logLifecycle("server healthy")
	return nil
}

func waitForServerHealth(ctx context.Context, cfg config.Config, maxAttempts int, interval time.Duration) error {
	client := &http.Client{Timeout: time.Second}
	url := healthURL(cfg)
	var lastErr error
	for range maxAttempts {
		if ctx.Err() != nil {
			return ctx.Err()
		}
		resp, err := client.Get(url)
		if err == nil {
			var health healthResponse
			decodeErr := json.NewDecoder(resp.Body).Decode(&health)
			resp.Body.Close()
			if resp.StatusCode == http.StatusOK && decodeErr == nil && (health.CDPConnected == nil || *health.CDPConnected) {
				return nil
			}
			if decodeErr != nil {
				lastErr = decodeErr
			} else {
				lastErr = fmt.Errorf("health endpoint not ready")
			}
		} else {
			lastErr = err
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(interval):
		}
	}
	if lastErr != nil {
		return fmt.Errorf("server health check failed: %w", lastErr)
	}
	return fmt.Errorf("server health check failed")
}

func healthURL(cfg config.Config) string {
	return fmt.Sprintf("http://127.0.0.1:%d/system/health", cfg.Ports.Server)
}
