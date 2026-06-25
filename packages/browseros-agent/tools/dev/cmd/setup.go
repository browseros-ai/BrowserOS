package cmd

import (
	"context"
	"fmt"
	"os"
	"path/filepath"

	"browseros-dev/proc"

	"github.com/spf13/cobra"
)

var setupIfNeeded bool

const setupModeIfNeeded = true

var setupCmd = &cobra.Command{
	Use:   "setup",
	Short: "Install dev dependencies and generate required code",
	Long:  "Installs Bun dependencies and generates agent GraphQL code needed by the dev environment.",
	RunE: func(cmd *cobra.Command, args []string) error {
		root, err := proc.FindMonorepoRoot()
		if err != nil {
			return err
		}
		return runDevSetup(cmd.Context(), root, setupIfNeeded)
	},
}

type setupPlan struct {
	RunInstall bool
	RunCodegen bool
}

func init() {
	setupCmd.Flags().BoolVar(&setupIfNeeded, "if-needed", false, "Skip generated code refresh when it already exists")
	rootCmd.AddCommand(setupCmd)
}

func buildSetupPlan(root string, ifNeeded bool) setupPlan {
	return setupPlan{
		RunInstall: true,
		RunCodegen: !ifNeeded || !generatedGraphQLExists(root),
	}
}

func generatedGraphQLExists(root string) bool {
	for _, file := range []string{"gql.ts", "graphql.ts", "schema.graphql"} {
		info, err := os.Stat(filepath.Join(root, "apps/app/generated/graphql", file))
		if err != nil || info.IsDir() {
			return false
		}
	}
	return true
}

// runDevSetup prepares the repo for local development. Dependency install always
// runs because Bun is fast and this keeps watch resilient after branch changes.
func runDevSetup(ctx context.Context, root string, ifNeeded bool) error {
	plan := buildSetupPlan(root, ifNeeded)

	if plan.RunInstall {
		proc.LogMsg(proc.TagSetup, "Installing dependencies...")
		if err := proc.RunBlocking(ctx, root, proc.TagSetup, "bun", "install", "--frozen-lockfile"); err != nil {
			return fmt.Errorf("installing dependencies: %w", err)
		}
	}

	if plan.RunCodegen {
		proc.LogMsg(proc.TagSetup, "Generating agent code...")
		if err := proc.RunBlocking(ctx, root, proc.TagSetup, "bun", "run", "codegen:agent"); err != nil {
			return fmt.Errorf("generating agent code: %w", err)
		}
	} else {
		proc.LogMsg(proc.TagSetup, "Agent code already generated")
	}

	proc.LogMsg(proc.TagSetup, "Setup ready")
	return nil
}
