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
	Short:   "Copy the configured BrowserOS profile into the browseros-dogfood dev profile",
	GroupID: groupRun,
	RunE: func(cmd *cobra.Command, args []string) error {
		cfg, err := loadConfig()
		if err != nil {
			return err
		}
		paths, err := defaultRunPaths()
		if err != nil {
			return err
		}
		lock, err := acquireRefreshProfileLock(paths)
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
		fmt.Printf("%s %s\n", successStyle.Sprint("Profile refreshed:"), pathStyle.Sprint(cfg.DevUserDataDir))
		return nil
	},
}

func acquireRefreshProfileLock(paths runPaths) (*dogfoodruntime.Lock, error) {
	lock, err := dogfoodruntime.AcquireLock(paths.Lock)
	if err == nil {
		if cleanupErr := dogfoodruntime.CleanupStaleRunFiles(paths.State); cleanupErr != nil {
			lock.Close()
			return nil, cleanupErr
		}
		return lock, nil
	}
	if errors.Is(err, dogfoodruntime.ErrAlreadyRunning) {
		return nil, refreshProfileRunningError(paths)
	}
	return nil, err
}

func refreshProfileRunningError(paths runPaths) error {
	state, err := dogfoodruntime.ReadRunState(paths.State)
	if err == nil {
		if state.Mode == "background" {
			return fmt.Errorf("cannot refresh profile while browseros-dogfood background daemon is running (pid %d); run `browseros-dogfood stop` first", state.PID)
		}
		return fmt.Errorf("cannot refresh profile while browseros-dogfood is running in foreground mode (pid %d); stop it first", state.PID)
	}
	return fmt.Errorf("cannot refresh profile while browseros-dogfood is running; run `browseros-dogfood stop` first")
}

func ensureDevProfileNotInUse(cfg config.Config) error {
	inUse, err := profile.HasSingletons(cfg.DevUserDataDir)
	if err != nil {
		return err
	}
	if inUse {
		return fmt.Errorf("cannot refresh profile because the dogfood dev profile is in use at %s; run `browseros-dogfood stop` first", cfg.DevUserDataDir)
	}
	return nil
}

func loadConfig() (config.Config, error) {
	cfg, err := loadConfigWithoutValidation()
	if err != nil {
		return config.Config{}, err
	}
	if err := cfg.Validate(); err != nil {
		return config.Config{}, err
	}
	return cfg, nil
}

func loadConfigWithoutValidation() (config.Config, error) {
	path, err := config.Path()
	if err != nil {
		return config.Config{}, err
	}
	cfg, err := config.Load(path)
	if err != nil {
		return config.Config{}, fmt.Errorf("missing config at %s; run browseros-dogfood init: %w", path, err)
	}
	return cfg, nil
}
