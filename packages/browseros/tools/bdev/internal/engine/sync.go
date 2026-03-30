package engine

import (
	"context"
	"fmt"
	"time"

	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/git"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/repo"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/workspace"
)

type SyncOptions struct {
	Workspace workspace.Entry
	Repo      *repo.Info
	Remote    string
	Rebase    bool
}

type SyncResult struct {
	Workspace string   `json:"workspace"`
	Remote    string   `json:"remote"`
	RepoHead  string   `json:"repo_head"`
	StashRef  string   `json:"stash_ref,omitempty"`
	Rebased   bool     `json:"rebased"`
	Fallback  bool     `json:"fallback"`
	Applied   []string `json:"applied,omitempty"`
	Conflicts []string `json:"conflicts,omitempty"`
}

func Sync(ctx context.Context, opts SyncOptions) (*SyncResult, error) {
	if opts.Remote == "" {
		opts.Remote = "origin"
	}
	dirty, err := git.IsDirty(ctx, opts.Repo.Root)
	if err != nil {
		return nil, err
	}
	if dirty {
		return nil, fmt.Errorf("patches repo has uncommitted changes; commit or stash them before syncing")
	}
	branch, err := git.CurrentBranch(ctx, opts.Repo.Root)
	if err != nil {
		return nil, err
	}
	if err := git.PullRebase(ctx, opts.Repo.Root, opts.Remote, branch); err != nil {
		return nil, err
	}
	head, err := git.HeadRev(ctx, opts.Repo.Root)
	if err != nil {
		return nil, err
	}
	state, err := workspace.LoadState(opts.Workspace.Path)
	if err != nil {
		return nil, err
	}
	result := &SyncResult{
		Workspace: opts.Workspace.Name,
		Remote:    opts.Remote,
		RepoHead:  head,
		Rebased:   opts.Rebase,
	}
	status, err := InspectWorkspace(ctx, opts.Workspace, opts.Repo)
	if err != nil {
		return nil, err
	}
	divergent := append([]string{}, status.NeedsUpdate...)
	divergent = append(divergent, status.Orphaned...)
	if len(divergent) > 0 {
		stashRef, err := git.StashPush(ctx, opts.Workspace.Path, "bdev sync stash", true, divergent)
		if err != nil {
			return nil, err
		}
		result.StashRef = stashRef
		state.PendingStash = stashRef
		if err := workspace.SaveState(opts.Workspace.Path, state); err != nil {
			return nil, err
		}
	}
	if state.LastSyncRev == "" || state.BaseCommit != "" && state.BaseCommit != opts.Repo.BaseCommit {
		result.Fallback = true
		applyResult, err := Apply(ctx, ApplyOptions{
			Workspace: opts.Workspace,
			Repo:      opts.Repo,
			Reset:     true,
			Mode:      "sync-reset",
		})
		if err != nil {
			return nil, err
		}
		result.Applied = applyResult.Applied
		if len(applyResult.Conflicts) > 0 {
			for _, conflict := range applyResult.Conflicts {
				result.Conflicts = append(result.Conflicts, conflict.ChromiumPath)
			}
			return result, nil
		}
	} else {
		applyResult, err := Apply(ctx, ApplyOptions{
			Workspace:  opts.Workspace,
			Repo:       opts.Repo,
			ChangedRef: state.LastSyncRev,
			RangeEnd:   head,
			Mode:       "sync",
		})
		if err != nil {
			return nil, err
		}
		result.Applied = applyResult.Applied
		if len(applyResult.Conflicts) > 0 {
			for _, conflict := range applyResult.Conflicts {
				result.Conflicts = append(result.Conflicts, conflict.ChromiumPath)
			}
			return result, nil
		}
	}
	if opts.Rebase && result.StashRef != "" {
		if err := git.StashPop(ctx, opts.Workspace.Path, result.StashRef); err != nil {
			return nil, err
		}
	}
	state.PendingStash = ""
	state.BaseCommit = opts.Repo.BaseCommit
	state.LastSyncRev = head
	state.LastSyncAt = time.Now().UTC()
	if err := workspace.SaveState(opts.Workspace.Path, state); err != nil {
		return nil, err
	}
	return result, nil
}
