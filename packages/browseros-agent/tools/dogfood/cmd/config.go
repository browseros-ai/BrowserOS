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
