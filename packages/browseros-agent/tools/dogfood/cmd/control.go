package cmd

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"strings"
	"time"

	"browseros-dogfood/config"
	"browseros-dogfood/ipc"
	"browseros-dogfood/proc"
	"browseros-dogfood/runlog"
	dogfoodruntime "browseros-dogfood/runtime"

	"github.com/spf13/cobra"
)

var restartPull bool
var restartForce bool
var logsFilter string
var logsLines int

const defaultMonitorPollInterval = 250 * time.Millisecond

type daemonMonitor struct {
	Paths        runPaths
	Target       config.Target
	Out          io.Writer
	Filter       string
	FromStart    bool
	Detach       <-chan struct{}
	Detached     *bool
	PollInterval time.Duration
	Status       func() (ipc.Response, error)
	Follow       func(context.Context, func(runlog.Entry)) error
}

var statusCmd = &cobra.Command{
	Use:     "status",
	Short:   "Show dogfood background daemon status",
	GroupID: groupInspect,
	RunE: func(cmd *cobra.Command, args []string) error {
		target, err := selectedRequiredTarget()
		if err != nil {
			return err
		}
		resp, err := sendControl(target, ipc.Request{Command: ipc.CmdStatus})
		if err != nil {
			return err
		}
		printStatus(resp.Data)
		return nil
	},
}

var stopCmd = &cobra.Command{
	Use:     "stop",
	Short:   "Stop the dogfood background daemon",
	GroupID: groupRun,
	RunE: func(cmd *cobra.Command, args []string) error {
		target, err := selectedRequiredTarget()
		if err != nil {
			return err
		}
		if _, err := sendControl(target, ipc.Request{Command: ipc.CmdStop}); err != nil {
			return err
		}
		fmt.Printf("%s %s background daemon\n", successStyle.Sprint("Stopping:"), targetLabel(target))
		return nil
	},
}

var restartCmd = &cobra.Command{
	Use:     "restart",
	Short:   "Rebuild/restart current checkout; --pull updates, --pull --force resets",
	GroupID: groupRun,
	RunE: func(cmd *cobra.Command, args []string) error {
		target, err := selectedRequiredTarget()
		if err != nil {
			return err
		}
		request, err := buildRestartRequest(restartPull, restartForce)
		if err != nil {
			return err
		}
		paths, err := defaultTargetRunPaths(target)
		if err != nil {
			return err
		}
		if _, err := sendControlWithPaths(paths, target, request); err != nil {
			return err
		}
		fmt.Fprintln(os.Stdout, successStyle.Sprint("Restart requested."))
		detach, cleanup := newInterruptDetach()
		defer cleanup()
		detached := false
		if err := monitorDaemonUntilRunning(cmd.Context(), daemonMonitor{
			Paths:    paths,
			Target:   target,
			Out:      os.Stdout,
			Detach:   detach,
			Detached: &detached,
		}); err != nil {
			return err
		}
		if !detached {
			fmt.Printf("%s %s background environment is healthy\n", successStyle.Sprint("Ready:"), targetLabel(target))
		}
		return nil
	},
}

var logsTailCmd = &cobra.Command{
	Use:   "tail",
	Short: "Tail daemon, chromium, and server logs from the background daemon",
	RunE: func(cmd *cobra.Command, args []string) error {
		target, err := selectedRequiredTarget()
		if err != nil {
			return err
		}
		paths, err := defaultTargetRunPaths(target)
		if err != nil {
			return err
		}
		resp, err := sendControlWithPaths(paths, target, ipc.Request{Command: ipc.CmdStatus})
		if err != nil {
			return err
		}
		logPath := logPathFromStatusData(resp.Data, paths.Log)
		entries, err := runlog.ReadLast(logPath, logsLines, logsFilter)
		if err != nil && !os.IsNotExist(err) {
			return err
		}
		for _, entry := range entries {
			fmt.Println(formatRunLogEntry(entry))
		}
		return followRunLogFromEnd(cmd.Context(), logPath, logsFilter, func(entry runlog.Entry) {
			fmt.Println(formatRunLogEntry(entry))
		})
	},
}

func init() {
	restartCmd.Flags().BoolVar(&restartPull, "pull", false, "Pull latest upstream changes before rebuilding and restarting")
	restartCmd.Flags().BoolVar(&restartForce, "force", false, "With --pull, reset to upstream before rebuilding and restarting")
	logsTailCmd.Flags().StringVar(&logsFilter, "filter", "", "Only show daemon, chromium, or server logs")
	logsTailCmd.Flags().IntVarP(&logsLines, "lines", "n", 80, "Number of existing log lines to show before following")
	logsCmd.AddCommand(logsTailCmd)
	rootCmd.AddCommand(statusCmd, stopCmd, restartCmd)
}

func buildRestartRequest(pull bool, force bool) (ipc.Request, error) {
	if force && !pull {
		return ipc.Request{}, fmt.Errorf("--force requires --pull")
	}
	request := ipc.Request{Command: ipc.CmdRestart}
	if pull || force {
		request.Args = map[string]string{}
	}
	if pull {
		request.Args["pull"] = "true"
	}
	if force {
		request.Args["force"] = "true"
	}
	return request, nil
}

func sendControl(target config.Target, req ipc.Request) (ipc.Response, error) {
	paths, err := defaultTargetRunPaths(target)
	if err != nil {
		return ipc.Response{}, err
	}
	return sendControlWithPaths(paths, target, req)
}

func sendControlWithPaths(paths runPaths, target config.Target, req ipc.Request) (ipc.Response, error) {
	resp, err := ipc.NewClient(paths.Socket).Send(req)
	if err != nil {
		return ipc.Response{}, daemonUnavailableError(paths, target, err)
	}
	if resp.Error != "" {
		return ipc.Response{}, errors.New(resp.Error)
	}
	return resp, nil
}

func daemonUnavailableError(paths runPaths, target config.Target, cause error) error {
	lock, err := dogfoodruntime.AcquireLock(paths.Lock)
	if err == nil {
		_ = lock.Close()
		_ = dogfoodruntime.CleanupStaleRunFiles(paths.State)
		if errors.Is(cause, ipc.ErrDaemonNotRunning) {
			return fmt.Errorf("%w; start it with `browseros-dogfood %s start-background`", cause, targetFlagOrDefault(target))
		}
		return cause
	}
	if errors.Is(err, dogfoodruntime.ErrAlreadyRunning) {
		state, stateErr := dogfoodruntime.ReadRunState(paths.State)
		if stateErr == nil && state.Mode == "foreground" {
			return fmt.Errorf("browseros-dogfood is running in foreground mode (pid %d); background daemon commands are unavailable", state.PID)
		}
		targetFlag := targetFlagOrDefault(target)
		return fmt.Errorf("browseros-dogfood background daemon is not responding; try `browseros-dogfood %s stop` if it is stuck, then `browseros-dogfood %s start-background`", targetFlag, targetFlag)
	}
	return err
}

func printStatus(data any) {
	fmt.Print(formatStatus(data))
}

func monitorDaemonUntilRunning(ctx context.Context, monitor daemonMonitor) error {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()

	out := monitor.Out
	if out == nil {
		out = io.Discard
	}
	pollInterval := monitor.PollInterval
	if pollInterval <= 0 {
		pollInterval = defaultMonitorPollInterval
	}
	status := monitor.Status
	if status == nil {
		status = func() (ipc.Response, error) {
			return sendControlWithPaths(monitor.Paths, monitor.Target, ipc.Request{Command: ipc.CmdStatus})
		}
	}
	follow := monitor.Follow
	if follow == nil {
		follow = func(ctx context.Context, onEntry func(runlog.Entry)) error {
			return followRunLog(ctx, monitor.Paths.Log, monitor.Filter, monitor.FromStart, onEntry)
		}
	}

	followErr := make(chan error, 1)
	go func() {
		followErr <- follow(ctx, func(entry runlog.Entry) {
			fmt.Fprintln(out, formatRunLogEntry(entry))
		})
	}()

	if done, err := daemonReachedTerminalState(status, monitor.Target); done || err != nil {
		return err
	}
	ticker := time.NewTicker(pollInterval)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-monitor.Detach:
			if monitor.Detached != nil {
				*monitor.Detached = true
			}
			targetFlag := targetFlagOrDefault(monitor.Target)
			fmt.Fprintf(out, "%s daemon still running. Run %s to reattach.\n", warnStyle.Sprint("Detached;"), commandStyle.Sprintf("browseros-dogfood %s logs tail", targetFlag))
			return nil
		case err := <-followErr:
			if err != nil && ctx.Err() == nil {
				return err
			}
		case <-ticker.C:
			if done, err := daemonReachedTerminalState(status, monitor.Target); done || err != nil {
				return err
			}
		}
	}
}

func daemonReachedTerminalState(status func() (ipc.Response, error), target config.Target) (bool, error) {
	resp, err := status()
	if err != nil {
		return false, err
	}
	state, lastError := monitorStatus(resp.Data)
	switch state {
	case "running":
		return true, nil
	case "error":
		if lastError == "" {
			lastError = "daemon entered error state"
		}
		targetFlag := targetFlagOrDefault(target)
		return true, fmt.Errorf("%s; run `browseros-dogfood %s logs tail` for details", lastError, targetFlag)
	default:
		return false, nil
	}
}

func monitorStatus(data any) (string, string) {
	status, ok := data.(map[string]any)
	if !ok {
		return "", ""
	}
	state, _ := stringValue(status["state"])
	lastError, _ := stringValue(status["last_error"])
	return state, lastError
}

func followRunLogFromStart(ctx context.Context, path string, filter string, onEntry func(runlog.Entry)) error {
	return followRunLog(ctx, path, filter, true, onEntry)
}

func followRunLogFromEnd(ctx context.Context, path string, filter string, onEntry func(runlog.Entry)) error {
	return followRunLog(ctx, path, filter, false, onEntry)
}

func followRunLog(ctx context.Context, path string, filter string, fromStart bool, onEntry func(runlog.Entry)) error {
	for {
		var err error
		if fromStart {
			err = runlog.FollowFromStartWithContext(ctx, path, filter, onEntry)
		} else {
			err = runlog.FollowWithContext(ctx, path, filter, onEntry)
		}
		if err == nil || !os.IsNotExist(err) {
			return err
		}
		select {
		case <-ctx.Done():
			return nil
		case <-time.After(100 * time.Millisecond):
		}
	}
}

func formatRunLogEntry(entry runlog.Entry) string {
	tag := entry.Tag
	if tag == "browser" {
		tag = "chromium"
	}
	style := dimStyle
	switch entry.Tag {
	case "daemon":
		style = warnStyle
	case "agent":
		style = proc.TagAgent.Color
	case "browser":
		style = proc.TagBrowser.Color
	case "server":
		style = proc.TagServer.Color
	}
	return fmt.Sprintf("%s %s %s", entry.Time.Format("15:04:05"), style.Sprintf("[%s]", tag), entry.Line)
}

func logPathFromStatusData(data any, fallback string) string {
	status, ok := data.(map[string]any)
	if !ok {
		return fallback
	}
	logPath, ok := stringValue(status["log_path"])
	if !ok || logPath == "" {
		return fallback
	}
	return logPath
}

func formatStatus(data any) string {
	status, ok := data.(map[string]any)
	if !ok {
		raw, err := json.MarshalIndent(data, "", "  ")
		if err != nil {
			return fmt.Sprintf("%v\n", data)
		}
		return string(raw) + "\n"
	}
	var out strings.Builder
	if target, ok := stringValue(status["target"]); ok && target != "" {
		fmt.Fprintf(&out, "%s %s\n", labelStyle.Sprint("Target:"), target)
	}
	writeStringField(&out, "State", status["state"])
	writeNumberField(&out, "PID", status["pid"])
	writeStringField(&out, "Uptime", status["uptime"])
	if operation, ok := stringValue(status["operation"]); ok && operation != "" {
		fmt.Fprintf(&out, "%s %s\n", labelStyle.Sprint("Operation:"), operation)
	}
	if lastError, ok := stringValue(status["last_error"]); ok && lastError != "" {
		fmt.Fprintf(&out, "%s %s\n", warnStyle.Sprint("Last error:"), lastError)
	}
	if ports, ok := status["ports"].(map[string]any); ok {
		target, _ := stringValue(status["target"])
		if target == string(config.TargetClaw) {
			fmt.Fprintf(&out, "%s CDP=%d API=%d\n", labelStyle.Sprint("Ports:"), intValue(ports["CDP"]), intValue(ports["Server"]))
		} else {
			fmt.Fprintf(
				&out,
				"%s CDP=%d Server=%d Extension=%d\n",
				labelStyle.Sprint("Ports:"),
				intValue(ports["CDP"]),
				intValue(ports["Server"]),
				intValue(ports["Extension"]),
			)
		}
	}
	target, _ := stringValue(status["target"])
	if target == string(config.TargetClaw) {
		writeStringField(&out, "Claw state", firstStatusValue(status["state_dir"], status["browseros_dir"]))
	} else {
		writeStringField(&out, "BrowserOS dir", status["browseros_dir"])
	}
	writeStringField(&out, "Logs", status["log_path"])
	return out.String()
}

func firstStatusValue(values ...any) any {
	for _, value := range values {
		if s, ok := stringValue(value); ok && s != "" {
			return s
		}
	}
	return nil
}

func writeStringField(out *strings.Builder, label string, value any) {
	if s, ok := stringValue(value); ok && s != "" {
		if label == "Logs" {
			fmt.Fprintf(out, "%s %s\n", labelStyle.Sprintf("%s:", label), pathStyle.Sprint(s))
			return
		}
		fmt.Fprintf(out, "%s %s\n", labelStyle.Sprintf("%s:", label), s)
	}
}

func writeNumberField(out *strings.Builder, label string, value any) {
	if n := intValue(value); n != 0 {
		fmt.Fprintf(out, "%s %d\n", labelStyle.Sprintf("%s:", label), n)
	}
}

func stringValue(value any) (string, bool) {
	s, ok := value.(string)
	return s, ok
}

func intValue(value any) int {
	switch v := value.(type) {
	case float64:
		return int(v)
	case int:
		return v
	default:
		return 0
	}
}
