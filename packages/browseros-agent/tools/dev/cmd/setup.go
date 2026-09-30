package cmd

import (
	"context"
	"fmt"

	"browseros-dev/proc"

	"github.com/spf13/cobra"
)

var setupCmd = &cobra.Command{
	Use:   "setup",
	Short: "Install dev dependencies",
	Long:  "Installs the Bun dependencies the dev environment needs.",
	RunE: func(cmd *cobra.Command, args []string) error {
		root, err := proc.FindMonorepoRoot()
		if err != nil {
			return err
		}
		return runDevSetup(cmd.Context(), root)
	},
}

func init() {
	rootCmd.AddCommand(setupCmd)
}

// runDevSetup prepares the repo for local development. Dependency install always
// runs because Bun is fast and this keeps watch resilient after branch changes.
//
// It used to generate the extension's GraphQL code as well, which is what the
// --if-needed flag skipped when the output was already there. There is no
// generated code left to skip.
func runDevSetup(ctx context.Context, root string) error {
	proc.LogMsg(proc.TagSetup, "Installing dependencies...")
	if err := proc.RunBlocking(ctx, root, proc.TagSetup, "bun", "install", "--frozen-lockfile"); err != nil {
		return fmt.Errorf("installing dependencies: %w", err)
	}

	proc.LogMsg(proc.TagSetup, "Setup ready")
	return nil
}
