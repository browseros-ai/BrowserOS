package cmd

import (
	"fmt"
	"time"

	"bdev/internal/config"
	"bdev/internal/engine"
	"bdev/internal/git"
	"bdev/internal/log"
	"bdev/internal/patch"
	"bdev/internal/ui"

	"github.com/spf13/cobra"
)

var pushCmd = &cobra.Command{
	Use:   "push [remote] [-- file1 file2 ...]",
	Short: "Push local changes to patches repo",
	Long: `Extract diffs from the current Chromium checkout and write them
to the patches repository. When a remote is provided (for example:
'bdev push origin'), bdev commits patch changes and pushes upstream.`,
	RunE: runPush,
}

var (
	pushDryRun  bool
	pushRemote  string
	pushNoSync  bool
	pushRebase  bool
	pushMessage string
)

func init() {
	pushCmd.Flags().BoolVar(&pushDryRun, "dry-run", false, "show what would be pushed")
	pushCmd.Flags().StringVar(&pushRemote, "remote", "", "patches repo remote to publish to")
	pushCmd.Flags().BoolVar(&pushNoSync, "no-sync", false, "skip syncing patches repo from remote before publish")
	pushCmd.Flags().BoolVar(&pushRebase, "rebase", true, "use git pull --rebase when syncing before publish")
	pushCmd.Flags().StringVarP(&pushMessage, "message", "m", "", "commit message when publishing to remote")
	rootCmd.AddCommand(pushCmd)
}

func runPush(cmd *cobra.Command, args []string) error {
	ctx, err := config.LoadContext()
	if err != nil {
		return err
	}

	activity := ui.NewActivity(verbose)
	remote, files, err := resolveRemoteAndFiles(ctx.PatchesRepo, args, pushRemote)
	if err != nil {
		return err
	}

	shouldPublish := remote != "" && !pushDryRun
	if shouldPublish {
		dirty, err := git.IsDirty(ctx.PatchesRepo)
		if err != nil {
			return err
		}
		if dirty {
			return fmt.Errorf("patches repo has local changes; commit/stash before publishing to remote %q", remote)
		}
	}

	if shouldPublish && !pushNoSync {
		if err := syncPatchesRepo(activity, ctx.PatchesRepo, remote, pushRebase); err != nil {
			return err
		}
	}

	if remote != "" && pushDryRun {
		activity.Info("dry run enabled — skipping remote sync and publish")
	}

	opts := engine.PushOpts{
		DryRun: pushDryRun,
		Files:  files,
	}

	if pushDryRun {
		activity.Info("dry run enabled — no patch files will be written")
		activity.Divider()
	}

	activity.Step("extracting checkout changes into patches")
	result, err := engine.Push(ctx, opts)
	if err != nil {
		return err
	}

	renderPushResult(result, pushDryRun)

	if !pushDryRun {
		if remote != "" {
			if err := publishPatchChanges(activity, ctx, remote, result, pushMessage); err != nil {
				return err
			}
		}

		// Update state
		repoRev, _ := git.HeadRev(ctx.PatchesRepo)
		ctx.State.LastPush = &config.SyncEvent{
			PatchesRepoRev: repoRev,
			Timestamp:      time.Now(),
			FileCount:      result.Total() + len(result.Stale),
		}
		_ = config.WriteState(ctx.BrosDir, ctx.State)

		// Activity log
		logger := log.New(ctx.BrosDir)
		_ = logger.LogPush(ctx.BaseCommit, result)
	}

	return nil
}

func syncPatchesRepo(activity *ui.Activity, patchesRepo, remote string, rebase bool) error {
	activity.Step("syncing patches repo from remote %q", remote)
	beforeRev, _ := git.HeadRev(patchesRepo)

	if err := git.Fetch(patchesRepo, remote); err != nil {
		return err
	}

	branch, detached, err := git.CurrentBranch(patchesRepo)
	if err != nil {
		return err
	}
	if detached {
		return fmt.Errorf("patches repo is in detached HEAD; cannot sync for publish")
	}

	if err := git.Pull(patchesRepo, remote, branch, rebase); err != nil {
		return err
	}

	afterRev, _ := git.HeadRev(patchesRepo)
	if beforeRev != "" && afterRev != "" && beforeRev != afterRev {
		activity.Success("patches repo advanced %s -> %s", shortRev(beforeRev), shortRev(afterRev))
	} else {
		activity.Info("patches repo already up to date")
	}
	return nil
}

func publishPatchChanges(
	activity *ui.Activity,
	ctx *config.Context,
	remote string,
	result *patch.PushResult,
	commitMessage string,
) error {
	dirty, err := git.IsDirty(ctx.PatchesRepo, "chromium_patches")
	if err != nil {
		return err
	}
	if !dirty {
		activity.Info("no patch repository changes to commit")
		return nil
	}

	branch, detached, err := git.CurrentBranch(ctx.PatchesRepo)
	if err != nil {
		return err
	}
	if detached {
		return fmt.Errorf("patches repo is in detached HEAD; cannot publish")
	}

	message := commitMessage
	if message == "" {
		message = fmt.Sprintf(
			"bdev push: %s (%d modified, %d added, %d deleted, %d stale)",
			ctx.Config.Name,
			len(result.Modified),
			len(result.Added),
			len(result.Deleted),
			len(result.Stale),
		)
	}

	activity.Step("committing patch changes to %s", branch)
	if err := git.Add(ctx.PatchesRepo, "chromium_patches"); err != nil {
		return err
	}
	if err := git.Commit(ctx.PatchesRepo, message); err != nil {
		return err
	}
	activity.Success("created patch commit")

	activity.Step("pushing patch commit to %s/%s", remote, branch)
	if err := git.Push(ctx.PatchesRepo, remote, branch); err != nil {
		return err
	}
	activity.Success("remote publish complete")

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

	fmt.Println(ui.TitleStyle.Render("bdev push"))
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
