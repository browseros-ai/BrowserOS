package cmd

import (
	"bufio"
	"errors"
	"fmt"
	"os"

	"browseros-dogfood/config"
	"browseros-dogfood/profile"
	dogfoodruntime "browseros-dogfood/runtime"

	"github.com/spf13/cobra"
)

func init() {
	rootCmd.AddCommand(refreshProfileCmd)
}

var refreshProfileCmd = &cobra.Command{
	Use:     "refresh-profile",
	Short:   "Copy the configured BrowserOS profile into the selected dogfood profile",
	GroupID: groupRun,
	RunE: func(cmd *cobra.Command, args []string) error {
		target, cfg, err := loadSelectedTargetConfig()
		if err != nil {
			return err
		}
		paths, err := defaultTargetRunPaths(target)
		if err != nil {
			return err
		}
		lock, err := acquireRefreshProfileLock(paths, target)
		if err != nil {
			return err
		}
		defer lock.Close()
		if err := ensureDevProfileNotInUse(cfg); err != nil {
			return err
		}
		if err := promptIfSourceProfileInUse(cmd.OutOrStdout(), bufio.NewReader(os.Stdin), cfg, true); err != nil {
			return err
		}
		if err := profile.Import(profile.ImportConfig{
			SourceUserDataDir: cfg.SourceUserDataDir,
			SourceProfileDir:  cfg.SourceProfileDir,
			DevUserDataDir:    cfg.DevUserDataDir,
			DevProfileDir:     cfg.DevProfileDir,
		}); err != nil {
			return err
		}
		fmt.Printf("%s %s\n", successStyle.Sprintf("%s profile refreshed:", targetLabel(target)), pathStyle.Sprint(cfg.DevUserDataDir))
		return nil
	},
}

func acquireRefreshProfileLock(paths runPaths, target config.Target) (*dogfoodruntime.Lock, error) {
	lock, err := dogfoodruntime.AcquireLock(paths.Lock)
	if err == nil {
		if cleanupErr := dogfoodruntime.CleanupStaleRunFiles(paths.State); cleanupErr != nil {
			lock.Close()
			return nil, cleanupErr
		}
		return lock, nil
	}
	if errors.Is(err, dogfoodruntime.ErrAlreadyRunning) {
		return nil, refreshProfileRunningError(paths, target)
	}
	return nil, err
}

func refreshProfileRunningError(paths runPaths, target config.Target) error {
	targetFlag := targetFlagOrDefault(target)
	state, err := dogfoodruntime.ReadRunState(paths.State)
	if err == nil {
		if state.Mode == "background" {
			return fmt.Errorf("cannot refresh profile while browseros-dogfood background daemon is running (pid %d); run `browseros-dogfood %s stop` first", state.PID, targetFlag)
		}
		return fmt.Errorf("cannot refresh profile while browseros-dogfood is running in foreground mode (pid %d); stop it first", state.PID)
	}
	return fmt.Errorf("cannot refresh profile while browseros-dogfood is running; run `browseros-dogfood %s stop` first", targetFlag)
}

func ensureDevProfileNotInUse(cfg config.Config) error {
	inUse, err := profile.HasSingletons(cfg.DevUserDataDir)
	if err != nil {
		return err
	}
	if inUse {
		targetFlag := targetFlagOrDefault(cfg.Target)
		return fmt.Errorf("cannot refresh profile because the dogfood dev profile is in use at %s; run `browseros-dogfood %s stop` first", cfg.DevUserDataDir, targetFlag)
	}
	return nil
}
