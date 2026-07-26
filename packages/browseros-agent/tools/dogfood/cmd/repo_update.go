package cmd

import (
	"context"
	"fmt"

	"browseros-dogfood/config"
	"browseros-dogfood/pipeline"
)

type repoUpdateOptions struct {
	Force           bool
	ResetToUpstream bool
}

// updateConfiguredRepo refreshes the dogfood checkout and first moves it to the configured branch.
func updateConfiguredRepo(ctx context.Context, cfg config.Config, runner pipeline.Runner, opts repoUpdateOptions) error {
	if runner == nil {
		runner = pipeline.ExecRunner{}
	}
	if !opts.Force {
		dirty, err := pipeline.Dirty(cfg.RepoPath, runner)
		if err != nil {
			return err
		}
		if dirty {
			return fmt.Errorf("checkout has uncommitted changes; commit/stash them or use --force")
		}
	}
	if err := pipeline.Fetch(ctx, cfg.RepoPath, runner); err != nil {
		return err
	}
	if err := pipeline.EnsureBranch(ctx, cfg.RepoPath, cfg.Branch, runner, opts.Force); err != nil {
		return err
	}
	if opts.ResetToUpstream {
		return pipeline.ResetHardToUpstream(ctx, cfg.RepoPath, runner)
	}
	return pipeline.Pull(ctx, cfg.RepoPath, runner)
}
