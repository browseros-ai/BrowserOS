package cmd

import (
	"fmt"
	"time"

	"bros/internal/config"
	"bros/internal/engine"
	"bros/internal/git"
	"bros/internal/log"
	"bros/internal/ui"

	"github.com/spf13/cobra"
)

var pullCmd = &cobra.Command{
	Use:   "pull [-- file1 file2 ...]",
	Short: "Pull patches from repo to checkout",
	Long: `Apply patches from the patches repository to the current Chromium
checkout. Resets changed files to BASE then applies new patches.`,
	RunE: runPull,
}

var pullDryRun bool

func init() {
	pullCmd.Flags().BoolVar(&pullDryRun, "dry-run", false, "show what would change")
	rootCmd.AddCommand(pullCmd)
}

func runPull(cmd *cobra.Command, args []string) error {
	ctx, err := config.LoadContext()
	if err != nil {
		return err
	}

	opts := engine.PullOpts{
		DryRun: pullDryRun,
		Files:  args,
	}

	if pullDryRun {
		fmt.Println(ui.MutedStyle.Render("dry run — no files will be modified"))
		fmt.Println()
	}

	result, err := engine.Pull(ctx, opts)
	if err != nil {
		return err
	}

	fmt.Print(ui.RenderPullResult(result))

	if len(result.Conflicts) > 0 {
		fmt.Print(ui.RenderConflictReport(result.Conflicts))
	}

	if !pullDryRun {
		repoRev, _ := git.HeadRev(ctx.PatchesRepo)
		ctx.State.LastPull = &config.SyncEvent{
			PatchesRepoRev: repoRev,
			Timestamp:      time.Now(),
			FileCount:      len(result.Applied) + len(result.Skipped),
		}
		_ = config.WriteState(ctx.BrosDir, ctx.State)

		logger := log.New(ctx.BrosDir)
		_ = logger.LogPull(ctx.BaseCommit, repoRev, result)
	}

	if len(result.Conflicts) > 0 {
		return fmt.Errorf("%d conflicts — see above for details", len(result.Conflicts))
	}

	return nil
}
