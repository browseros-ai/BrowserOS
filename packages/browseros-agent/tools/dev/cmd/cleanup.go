package cmd

import (
	"bufio"
	"fmt"
	"io"
	"os"
	"time"

	"browseros-dev/proc"

	"github.com/spf13/cobra"
)

var cleanupCmd = &cobra.Command{
	Use:   "cleanup",
	Short: "Kill target processes and remove orphaned temp directories",
	Long:  "Stops target BrowserOS processes, clears target ports, and removes target temp directories.",
	RunE:  runCleanup,
}

var (
	cleanupOnlyPorts          bool
	cleanupOnlyTemps          bool
	cleanupQuick              bool
	cleanupYes                bool
	cleanupTarget             string
	cleanupBrowserOSDir       string
	cleanupPortsValue         string
	cleanupBrowserUserDataDir string
)

type safeCleanupOptions struct {
	ports bool
	temps bool
}

func init() {
	cleanupCmd.Flags().StringVar(&cleanupTarget, "target", targetDev, "Cleanup target: dev, dogfood, or prod")
	cleanupCmd.Flags().StringVar(&cleanupBrowserOSDir, "browseros-dir", "", "Override target BrowserOS state directory")
	cleanupCmd.Flags().StringVar(&cleanupPortsValue, "ports", "", "Override ports as cdp,server,extension")
	cleanupCmd.Flags().StringVar(&cleanupBrowserUserDataDir, "browser-user-data-dir", "", "Override BrowserOS user-data dir to stop")
	cleanupCmd.Flags().BoolVar(&cleanupOnlyPorts, "only-ports", false, "Only kill port processes")
	cleanupCmd.Flags().BoolVar(&cleanupOnlyTemps, "only-temps", false, "Only remove temp directories")
	cleanupCmd.Flags().BoolVar(&cleanupQuick, "quick", false, "Run safe cleanup only")
	cleanupCmd.Flags().BoolVar(&cleanupYes, "yes", false, "Answer yes to the safe cleanup prompt")
	rootCmd.AddCommand(cleanupCmd)
}

// runCleanup performs the non-destructive daily cleanup path for local dev.
func runCleanup(cmd *cobra.Command, args []string) error {
	out := cmd.OutOrStdout()
	root, err := proc.FindMonorepoRoot()
	if err != nil {
		return err
	}
	target, err := resolveResetTarget(root, resetTargetOptions{
		Target:             cleanupTarget,
		BrowserOSDir:       cleanupBrowserOSDir,
		Ports:              cleanupPortsValue,
		BrowserUserDataDir: cleanupBrowserUserDataDir,
	})
	if err != nil {
		return err
	}
	if !cleanupYes && !cleanupQuick {
		ok, err := confirmYesNo(out, bufio.NewReader(os.Stdin), resetPrompt{
			Title:  "Run safe cleanup?",
			Body:   fmt.Sprintf("Stops %s processes, clears target ports, and removes target temp profiles. This does not touch saved BrowserOS data, Lima, containers, or images.", target.Name),
			Action: "Run safe cleanup for " + target.Name,
		})
		if err != nil {
			return err
		}
		if !ok {
			fmt.Fprintln(out, dimStyle.Sprint("Skipped."))
			return nil
		}
	}
	if err := ensureTargetStopped(out, target); err != nil {
		return err
	}
	return runSafeCleanup(out, target, safeCleanupOptions{
		ports: !cleanupOnlyTemps || cleanupOnlyPorts,
		temps: !cleanupOnlyPorts || cleanupOnlyTemps,
	})
}

// runSafeCleanup is shared by cleanup and reset before any destructive repair steps.
func runSafeCleanup(out io.Writer, target resetTarget, opts safeCleanupOptions) error {
	if opts.ports {
		if target.WatchRunStateDir != "" {
			stopped, err := proc.StopAllWatchProcessesInDir(target.WatchRunStateDir, 3*time.Second)
			if err != nil {
				return err
			}
			if stopped > 0 {
				fmt.Fprintf(out, "%s stopped %d old %s watch process group(s)\n", successStyle.Sprint("Stopped:"), stopped, target.Name)
			}
		}
		if len(target.BrowserUserDataDirs) > 0 {
			killedBrowsers, err := proc.KillBrowserProcessesForUserDataDirs(target.BrowserUserDataDirs, 3*time.Second)
			if err != nil {
				return err
			}
			if killedBrowsers > 0 {
				fmt.Fprintf(out, "%s stopped %d BrowserOS %s profile process(es)\n", successStyle.Sprint("Stopped:"), killedBrowsers, target.Name)
			}
		}
		if target.Ports != nil {
			ports := *target.Ports
			fmt.Fprintf(out, "%s ports %d, %d, %d\n", labelStyle.Sprint("Clearing:"), ports.CDP, ports.Server, ports.Extension)
			if err := proc.KillPortsAndWait(ports, 3*time.Second); err != nil {
				return err
			}
			fmt.Fprintln(out, successStyle.Sprint("Ports cleared."))
		}
	}

	if opts.temps {
		n := proc.CleanupTempDirs(target.TempPrefixes...)
		if n > 0 {
			fmt.Fprintf(out, "%s removed %d temp directories\n", successStyle.Sprint("Removed:"), n)
		} else if len(target.TempPrefixes) > 0 {
			fmt.Fprintln(out, dimStyle.Sprint("No orphaned temp directories found."))
		}
	}

	fmt.Fprintln(out)
	fmt.Fprintln(out, successStyle.Sprint("Cleanup complete."))
	return nil
}
