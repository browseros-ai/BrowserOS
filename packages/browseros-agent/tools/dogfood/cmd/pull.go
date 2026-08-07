package cmd

import (
	"fmt"

	"browseros-dogfood/pipeline"

	"github.com/spf13/cobra"
)

var pullForce bool

func init() {
	pullCmd.Flags().BoolVar(&pullForce, "force", false, "Pull even when the checkout has uncommitted changes")
	rootCmd.AddCommand(pullCmd)
}

var pullCmd = &cobra.Command{
	Use:     "pull",
	Short:   "Refresh the configured checkout",
	GroupID: groupRun,
	RunE: func(cmd *cobra.Command, args []string) error {
		_, cfg, err := loadSelectedTargetConfig()
		if err != nil {
			return err
		}
		runner := pipeline.ExecRunner{}
		if err := pipeline.WriteProductionEnvFile(cfg.AgentRoot(), cfg); err != nil {
			return err
		}
		branch := pipeline.Branch(cfg.RepoPath, runner)
		head, _ := pipeline.Head(cfg.RepoPath, runner)
		fmt.Printf("%s %s %s %s\n", labelStyle.Sprint("Repo:"), pathStyle.Sprint(cfg.RepoPath), commandStyle.Sprint(branch), dimStyle.Sprint(head))
		if err := updateConfiguredRepo(cmd.Context(), cfg, runner, repoUpdateOptions{Force: pullForce}); err != nil {
			return err
		}
		newBranch := pipeline.Branch(cfg.RepoPath, runner)
		newHead, _ := pipeline.Head(cfg.RepoPath, runner)
		fmt.Printf("%s %s %s\n", successStyle.Sprint("Updated to"), commandStyle.Sprint(newBranch), commandStyle.Sprint(newHead))
		return nil
	},
}
