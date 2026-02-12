package cmd

import (
	"fmt"
	"time"

	"bros/internal/config"
	"bros/internal/engine"
	"bros/internal/git"
	"bros/internal/log"
	"bros/internal/patch"
	"bros/internal/ui"

	"github.com/spf13/cobra"
)

var pushCmd = &cobra.Command{
	Use:   "push [-- file1 file2 ...]",
	Short: "Push local changes to patches repo",
	Long: `Extract diffs from the current Chromium checkout and write them
to the patches repository. All patches are full diffs from BASE_COMMIT.`,
	RunE: runPush,
}

var pushDryRun bool

func init() {
	pushCmd.Flags().BoolVar(&pushDryRun, "dry-run", false, "show what would be pushed")
	rootCmd.AddCommand(pushCmd)
}

func runPush(cmd *cobra.Command, args []string) error {
	ctx, err := config.LoadContext()
	if err != nil {
		return err
	}

	opts := engine.PushOpts{
		DryRun: pushDryRun,
		Files:  args,
	}

	if pushDryRun {
		fmt.Println(ui.MutedStyle.Render("dry run — no files will be written"))
		fmt.Println()
	}

	result, err := engine.Push(ctx, opts)
	if err != nil {
		return err
	}

	renderPushResult(result, pushDryRun)

	if !pushDryRun {
		// Update state
		repoRev, _ := git.HeadRev(ctx.PatchesRepo)
		ctx.State.LastPush = &config.SyncEvent{
			PatchesRepoRev: repoRev,
			Timestamp:      time.Now(),
			FileCount:      result.Total(),
		}
		_ = config.WriteState(ctx.BrosDir, ctx.State)

		// Activity log
		logger := log.New(ctx.BrosDir)
		_ = logger.LogPush(ctx.BaseCommit, result)
	}

	return nil
}

func renderPushResult(r *patch.PushResult, dryRun bool) {
	if r.Total() == 0 && len(r.Stale) == 0 {
		fmt.Println(ui.MutedStyle.Render("Nothing to push — checkout matches patches repo."))
		return
	}

	verb := "Pushed"
	if dryRun {
		verb = "Would push"
	}

	fmt.Println(ui.TitleStyle.Render("bros push"))
	fmt.Println()

	for _, f := range r.Added {
		fmt.Printf("  %s %s\n", ui.AddedPrefix, f)
	}
	for _, f := range r.Modified {
		fmt.Printf("  %s %s\n", ui.ModifiedPrefix, f)
	}
	for _, f := range r.Deleted {
		fmt.Printf("  %s %s\n", ui.DeletedPrefix, f)
	}
	for _, f := range r.Stale {
		fmt.Printf("  %s %s\n", ui.SkippedPrefix, ui.MutedStyle.Render(f+" (stale, removed)"))
	}

	fmt.Println()
	summary := fmt.Sprintf("%s %d patches", verb, r.Total())
	detail := fmt.Sprintf(" (%d modified, %d added, %d deleted)",
		len(r.Modified), len(r.Added), len(r.Deleted))
	fmt.Print(ui.SuccessStyle.Render(summary))
	fmt.Println(ui.MutedStyle.Render(detail))

	if len(r.Stale) > 0 {
		fmt.Println(ui.MutedStyle.Render(fmt.Sprintf("Cleaned %d stale patches", len(r.Stale))))
	}
}
