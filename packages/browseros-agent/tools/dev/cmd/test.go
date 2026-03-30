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

	"browseros-dev/browser"
	"browseros-dev/proc"
	"browseros-dev/server"

	"github.com/spf13/cobra"
)

var testCobraCmd = &cobra.Command{
	Use:   "test [-- bun test args...]",
	Short: "Start test environment, run tests, clean up",
	Long:  "Kills conflicting ports, starts server + browser, waits for readiness, runs bun test, then cleans up.",
	RunE:  runTest,
}

var (
	testKeep     bool
	testHeadless bool
)

func init() {
	testCobraCmd.Flags().BoolVar(&testKeep, "keep", false, "Don't clean up after tests (for debugging)")
	testCobraCmd.Flags().BoolVar(&testHeadless, "headless", false, "Run BrowserOS headless")
	rootCmd.AddCommand(testCobraCmd)
}

func runTest(cmd *cobra.Command, args []string) error {
	root, err := proc.FindMonorepoRoot()
	if err != nil {
		return err
	}

	p := proc.DefaultLocalPorts()

	proc.LogMsg(proc.TagInfo, "Killing processes on test ports...")
	proc.KillPorts(p)
	proc.LogMsg(proc.TagInfo, "Ports cleared")

	if n := proc.CleanupTempDirs("browseros-test-"); n > 0 {
		proc.LogMsgf(proc.TagInfo, "Removed %d orphaned temp directories", n)
	}

	fmt.Println()
	proc.LogMsgf(proc.TagInfo, "Ports: CDP=%d Server=%d Extension=%d", p.CDP, p.Server, p.Extension)
	if testHeadless {
		proc.LogMsg(proc.TagInfo, "Mode: headless")
	}
	proc.LogMsg(proc.TagInfo, proc.DimColor.Sprint("Press Ctrl+C to stop, double Ctrl+C to force kill"))
	fmt.Println()

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	sigCh := make(chan os.Signal, 3)
	signal.Notify(sigCh, syscall.SIGINT, syscall.SIGTERM)

	var wg sync.WaitGroup
	var procs []*proc.ManagedProc
	tempDir := ""

	var cleanupOnce sync.Once
	cleanup := func() {
		cleanupOnce.Do(func() {
			cancel()
			for _, mp := range procs {
				mp.Stop()
			}
			wg.Wait()
			if tempDir != "" && !testKeep {
				proc.LogMsgf(proc.TagInfo, "Removing temp profile: %s", tempDir)
				os.RemoveAll(tempDir)
			}
		})
	}

	// Handle Ctrl+C: first = graceful cleanup, second = force kill
	go func() {
		<-sigCh
		fmt.Println()
		proc.LogMsg(proc.TagInfo, proc.WarnColor.Sprint("Interrupted, cleaning up (Ctrl+C again to force)..."))
		go cleanup()

		<-sigCh
		fmt.Println()
		proc.LogMsg(proc.TagInfo, proc.ErrorColor.Sprint("Force killing all processes..."))
		for _, mp := range procs {
			mp.ForceKill()
		}
		os.Exit(1)
	}()

	env := proc.BuildEnv(p, "test")
	serverDir := filepath.Join(root, "apps/server")

	// Start server
	proc.LogMsg(proc.TagServer, "Starting server...")
	procs = append(procs, proc.StartManaged(ctx, &wg, proc.ProcConfig{
		Tag:     proc.TagServer,
		Dir:     root,
		Env:     env,
		Restart: false,
		Cmd: []string{
			"bun", filepath.Join(serverDir, "src/index.ts"),
			"--cdp-port", fmt.Sprintf("%d", p.CDP),
			"--server-port", fmt.Sprintf("%d", p.Server),
		},
	}))

	proc.LogMsg(proc.TagServer, "Waiting for server health...")
	if !server.WaitForHealth(ctx, p.Server, 30) {
		cleanup()
		return fmt.Errorf("server failed to start on port %d", p.Server)
	}
	proc.LogMsg(proc.TagServer, "Server ready")

	// Start browser with temp profile
	tempDir, err = os.MkdirTemp("", "browseros-test-")
	if err != nil {
		cleanup()
		return fmt.Errorf("creating temp dir: %w", err)
	}
	proc.LogMsgf(proc.TagBrowser, "Created temp profile: %s", tempDir)

	proc.LogMsg(proc.TagBrowser, "Starting BrowserOS...")
	procs = append(procs, proc.StartManaged(ctx, &wg, proc.ProcConfig{
		Tag:     proc.TagBrowser,
		Dir:     root,
		Restart: false,
		Cmd: browser.BuildArgs(browser.ArgsConfig{
			Root:              root,
			Ports:             p,
			UserDataDir:       tempDir,
			Headless:          testHeadless,
			LoadDevExtensions: false,
		}),
	}))

	proc.LogMsg(proc.TagBrowser, "Waiting for CDP...")
	if !browser.WaitForCDP(ctx, p.CDP, 60) {
		cleanup()
		return fmt.Errorf("CDP failed to start on port %d", p.CDP)
	}
	proc.LogMsg(proc.TagBrowser, "CDP ready")

	fmt.Println()
	proc.LogMsg(proc.TagInfo, proc.BoldColor.Sprint("Test environment ready"))
	fmt.Println()

	// Run bun test
	bunArgs := []string{"test"}
	bunArgs = append(bunArgs, args...)

	proc.LogMsgf(proc.TagTest, "Running: bun %s", proc.DimColor.Sprint(joinArgs(bunArgs)))

	testExec := exec.CommandContext(ctx, "bun", bunArgs...)
	testExec.Dir = root
	testExec.Env = env
	testExec.Stdout = os.Stdout
	testExec.Stderr = os.Stderr

	testErr := testExec.Run()

	fmt.Println()
	if testKeep {
		proc.LogMsg(proc.TagInfo, "Keeping test environment (--keep)")
		proc.LogMsgf(proc.TagInfo, "Temp profile: %s", tempDir)
	}
	cleanup()

	if testErr != nil {
		proc.LogMsg(proc.TagTest, proc.ErrorColor.Sprint("Tests failed"))
		os.Exit(1)
	}

	proc.LogMsg(proc.TagTest, "Tests passed")
	return nil
}

func joinArgs(args []string) string {
	result := ""
	for i, a := range args {
		if i > 0 {
			result += " "
		}
		result += a
	}
	return result
}
