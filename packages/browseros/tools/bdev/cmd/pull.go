package cmd

import (
	"fmt"
	"time"

	"bdev/internal/config"
	"bdev/internal/engine"
	"bdev/internal/git"
	"bdev/internal/log"
	"bdev/internal/ui"

	"github.com/spf13/cobra"
)

var pullCmd = &cobra.Command{
	Use:   "pull [remote] [-- file1 file2 ...]",
	Short: "Pull patches from repo to checkout",
	Long: `Apply patches from the patches repository to the current Chromium
checkout. Use an optional remote (for example: 'bdev pull origin')
to fetch/rebase the patches repo before applying changes locally.`,
	RunE: runPull,
}

var (
	pullDryRun        bool
	pullRemote        string
	pullNoSync        bool
	pullRebase        bool
	pullKeepLocalOnly bool
)

func init() {
	pullCmd.Flags().BoolVar(&pullDryRun, "dry-run", false, "show what would change")
	pullCmd.Flags().StringVar(&pullRemote, "remote", "", "patches repo remote to sync before pull")
	pullCmd.Flags().BoolVar(&pullNoSync, "no-sync", false, "skip syncing patches repo from remote")
	pullCmd.Flags().BoolVar(&pullRebase, "rebase", true, "use git pull --rebase when syncing remote")
	pullCmd.Flags().BoolVar(&pullKeepLocalOnly, "keep-local-only", true, "keep local-only checkout changes that are not in patches repo")
	rootCmd.AddCommand(pullCmd)
}

func runPull(cmd *cobra.Command, args []string) error {
	ctx, err := config.LoadContext()
	if err != nil {
		return err
	}

	activity := ui.NewActivity(verbose)
	remote, files, err := resolveRemoteAndFiles(ctx.PatchesRepo, args, pullRemote)
	if err != nil {
		return err
	}

	shouldSync := remote != "" && !pullNoSync && !pullDryRun
	if shouldSync {
		dirty, err := git.IsDirty(ctx.PatchesRepo)
		if err != nil {
			return err
		}
		if dirty {
			return fmt.Errorf("patches repo has local changes; commit/stash before syncing remote %q", remote)
		}

		activity.Step("syncing patches repo from remote %q", remote)
		beforeRev, _ := git.HeadRev(ctx.PatchesRepo)

		if err := git.Fetch(ctx.PatchesRepo, remote); err != nil {
			return err
		}

		branch, detached, err := git.CurrentBranch(ctx.PatchesRepo)
		if err != nil {
			return err
		}
		if detached {
			activity.Warn("patches repo is in detached HEAD; fetched remote but skipped pull/rebase")
		} else {
			if err := git.Pull(ctx.PatchesRepo, remote, branch, pullRebase); err != nil {
				return err
			}
		}

		afterRev, _ := git.HeadRev(ctx.PatchesRepo)
		if beforeRev != "" && afterRev != "" && beforeRev != afterRev {
			activity.Success("patches repo advanced %s -> %s", shortRev(beforeRev), shortRev(afterRev))
		} else {
			activity.Info("patches repo already up to date")
		}

		ctx, err = config.LoadContext()
		if err != nil {
			return err
		}
	} else if remote != "" && pullDryRun {
		activity.Info("dry run enabled — skipping remote sync")
	} else if remote != "" && pullNoSync {
		activity.Info("remote %q provided, but sync is disabled via --no-sync", remote)
	}

	opts := engine.PullOpts{
		DryRun:        pullDryRun,
		Files:         files,
		KeepLocalOnly: pullKeepLocalOnly,
	}

	if pullDryRun {
		activity.Info("dry run enabled — no files will be modified")
		activity.Divider()
	}

	activity.Step("computing patch delta and applying updates")
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
			BaseCommit:     ctx.BaseCommit,
			Timestamp:      time.Now(),
			FileCount:      len(result.Applied) + len(result.Deleted) + len(result.Reverted) + len(result.LocalOnly) + len(result.Skipped),
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
