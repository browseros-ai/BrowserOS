package cmd

import (
	"fmt"
	"os"
	"os/exec"

	"browseros-dogfood/config"

	"github.com/spf13/cobra"
)

func init() {
	configCmd.AddCommand(configEditCmd)
	rootCmd.AddCommand(configCmd)
}

// loadConfig loads the dogfood config for a runtime command and validates it.
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

var configCmd = &cobra.Command{
	Use:     "config",
	Short:   "Manage browseros-dogfood config",
	GroupID: groupSetup,
}

var configEditCmd = &cobra.Command{
	Use:   "edit",
	Short: "Open browseros-dogfood config in $EDITOR",
	RunE: func(cmd *cobra.Command, args []string) error {
		path, err := config.Path()
		if err != nil {
			return err
		}
		if _, err := os.Stat(path); os.IsNotExist(err) {
			home, err := os.UserHomeDir()
			if err != nil {
				return err
			}
			cfg := config.Defaults(home)
			if err := config.Save(path, cfg); err != nil {
				return err
			}
		}
		editor := os.Getenv("EDITOR")
		if editor == "" {
			editor = "vi"
		}
		c := exec.Command(editor, path)
		c.Stdin = os.Stdin
		c.Stdout = os.Stdout
		c.Stderr = os.Stderr
		if err := c.Run(); err != nil {
			return fmt.Errorf("editor failed: %w", err)
		}
		return nil
	},
}
