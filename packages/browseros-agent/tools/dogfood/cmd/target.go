package cmd

import (
	"fmt"
	"path/filepath"

	"browseros-dogfood/config"

	"github.com/spf13/cobra"
)

var targetBrowserOS bool
var targetClaw bool

var targetRequiredCommands = map[string]struct{}{
	"daemon":           {},
	"init":             {},
	"logs":             {},
	"pull":             {},
	"refresh-profile":  {},
	"restart":          {},
	"start":            {},
	"start-background": {},
	"status":           {},
	"stop":             {},
	"tail":             {},
}

func init() {
	rootCmd.PersistentFlags().BoolVarP(&targetBrowserOS, "browseros", "b", false, "Target BrowserOS dogfood")
	rootCmd.PersistentFlags().BoolVarP(&targetClaw, "claw", "c", false, "Target BrowserClaw dogfood")
	rootCmd.PersistentPreRunE = requireTargetForLifecycleCommand
}

// requireTargetForLifecycleCommand enforces explicit target choice for runtime commands.
func requireTargetForLifecycleCommand(cmd *cobra.Command, args []string) error {
	if _, _, err := resolveTargetFlags(targetBrowserOS, targetClaw); err != nil {
		return err
	}
	if !commandRequiresTarget(cmd) {
		return nil
	}
	_, ok, err := selectedTarget()
	if err != nil {
		return err
	}
	if !ok {
		return fmt.Errorf("choose a dogfood target with --browseros or --claw")
	}
	return nil
}

func commandRequiresTarget(cmd *cobra.Command) bool {
	for current := cmd; current != nil; current = current.Parent() {
		if _, ok := targetRequiredCommands[current.Name()]; ok {
			return true
		}
	}
	return false
}

func selectedTarget() (config.Target, bool, error) {
	return resolveTargetFlags(targetBrowserOS, targetClaw)
}

// selectedRequiredTarget returns the flag-selected target without reading config.
func selectedRequiredTarget() (config.Target, error) {
	target, ok, err := selectedTarget()
	if err != nil {
		return "", err
	}
	if !ok {
		return "", fmt.Errorf("choose a dogfood target with --browseros or --claw")
	}
	return target, nil
}

func resolveTargetFlags(browserOS bool, claw bool) (config.Target, bool, error) {
	switch {
	case browserOS && claw:
		return "", false, fmt.Errorf("--browseros and --claw are mutually exclusive")
	case browserOS:
		return config.TargetBrowserOS, true, nil
	case claw:
		return config.TargetClaw, true, nil
	default:
		return "", false, nil
	}
}

func selectedTargetFlag(target config.Target) (string, error) {
	switch target {
	case config.TargetBrowserOS:
		return "--browseros", nil
	case config.TargetClaw:
		return "--claw", nil
	default:
		return "", fmt.Errorf("unknown dogfood target %q", target)
	}
}

func targetFlagOrDefault(target config.Target) string {
	flag, err := selectedTargetFlag(target)
	if err != nil {
		return "--browseros"
	}
	return flag
}

// loadTargetConfig loads config and projects it to the requested dogfood target.
func loadTargetConfig(target config.Target) (config.Config, error) {
	path, err := config.Path()
	if err != nil {
		return config.Config{}, err
	}
	cfg, err := config.Load(path)
	if err != nil {
		return config.Config{}, fmt.Errorf("missing config at %s; run browseros-dogfood --%s init: %w", path, target, err)
	}
	if err := cfg.ApplyTarget(target); err != nil {
		return config.Config{}, err
	}
	if err := cfg.Validate(); err != nil {
		return config.Config{}, err
	}
	return cfg, nil
}

func loadSelectedTargetConfig() (config.Target, config.Config, error) {
	target, err := selectedRequiredTarget()
	if err != nil {
		return "", config.Config{}, err
	}
	cfg, err := loadTargetConfig(target)
	return target, cfg, err
}

func loadTargetConfigWithoutValidation(target config.Target) (config.Config, error) {
	path, err := config.Path()
	if err != nil {
		return config.Config{}, err
	}
	cfg, err := config.Load(path)
	if err != nil {
		return config.Config{}, fmt.Errorf("missing config at %s; run browseros-dogfood --%s init: %w", path, target, err)
	}
	if err := cfg.ApplyTarget(target); err != nil {
		return config.Config{}, err
	}
	return cfg, nil
}

func loadSelectedTargetConfigWithoutValidation() (config.Target, config.Config, error) {
	target, err := selectedRequiredTarget()
	if err != nil {
		return "", config.Config{}, err
	}
	cfg, err := loadTargetConfigWithoutValidation(target)
	return target, cfg, err
}

func defaultTargetRunPaths(target config.Target) (runPaths, error) {
	path, err := config.Path()
	if err != nil {
		return runPaths{}, err
	}
	return newTargetRunPaths(path, target), nil
}

// newTargetRunPaths keeps daemon IPC and logs isolated per dogfood target.
func newTargetRunPaths(configPath string, target config.Target) runPaths {
	dir := filepath.Join(filepath.Dir(configPath), string(target))
	return runPaths{
		Dir:    dir,
		Lock:   filepath.Join(dir, "run.lock"),
		State:  filepath.Join(dir, "state.json"),
		Socket: filepath.Join(dir, "daemon.sock"),
		Log:    filepath.Join(dir, "daemon.jsonl"),
		RawLog: filepath.Join(dir, "daemon.log"),
	}
}
