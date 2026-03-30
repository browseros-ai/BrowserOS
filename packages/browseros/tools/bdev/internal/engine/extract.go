package engine

import (
	"context"
	"time"

	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/git"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/patch"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/repo"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/workspace"
)

type ExtractOptions struct {
	Workspace  workspace.Entry
	Repo       *repo.Info
	Commit     string
	RangeStart string
	RangeEnd   string
	Squash     bool
	Base       string
	Filters    []string
}

type ExtractResult struct {
	Workspace  string   `json:"workspace"`
	Mode       string   `json:"mode"`
	BaseCommit string   `json:"base_commit"`
	Written    []string `json:"written"`
	Deleted    []string `json:"deleted"`
}

func Extract(ctx context.Context, opts ExtractOptions) (*ExtractResult, error) {
	base := opts.Base
	if base == "" {
		base = opts.Repo.BaseCommit
	}
	var (
		set   patch.PatchSet
		scope []string
		err   error
		mode  string
	)
	switch {
	case opts.Commit != "":
		mode = "commit"
		set, err = patch.BuildCommitPatchSet(ctx, opts.Workspace.Path, opts.Commit, opts.Base, opts.Filters)
		if err == nil {
			if opts.Base != "" {
				changes, err := git.DiffTreeNameStatus(ctx, opts.Workspace.Path, opts.Commit, opts.Filters)
				if err != nil {
					return nil, err
				}
				scope = changedScope(changes)
			} else {
				scope = patch.ScopeFromSet(set)
			}
		}
	case opts.RangeStart != "" && opts.RangeEnd != "":
		mode = "range"
		set, err = patch.BuildRangePatchSet(ctx, opts.Workspace.Path, opts.RangeStart, opts.RangeEnd, opts.Base, opts.Squash, opts.Filters)
		if err == nil {
			if opts.Base != "" || opts.Squash {
				changes, err := git.DiffNameStatusBetween(ctx, opts.Workspace.Path, opts.RangeStart, opts.RangeEnd, opts.Filters)
				if err != nil {
					return nil, err
				}
				scope = changedScope(changes)
			} else {
				scope = patch.ScopeFromSet(set)
			}
		}
	default:
		mode = "working-tree"
		set, err = patch.BuildWorkingTreePatchSet(ctx, opts.Workspace.Path, base, opts.Filters)
		if err == nil && len(opts.Filters) > 0 {
			scope = opts.Filters
		}
	}
	if err != nil {
		return nil, err
	}
	written, deleted, err := patch.WriteRepoPatchSet(opts.Repo.PatchesDir, set, scope)
	if err != nil {
		return nil, err
	}
	state, err := workspace.LoadState(opts.Workspace.Path)
	if err != nil {
		return nil, err
	}
	head, err := git.HeadRev(ctx, opts.Workspace.Path)
	if err != nil {
		return nil, err
	}
	state.BaseCommit = opts.Repo.BaseCommit
	state.LastExtractRev = head
	state.LastExtractAt = time.Now().UTC()
	if err := workspace.SaveState(opts.Workspace.Path, state); err != nil {
		return nil, err
	}
	return &ExtractResult{
		Workspace:  opts.Workspace.Name,
		Mode:       mode,
		BaseCommit: base,
		Written:    written,
		Deleted:    deleted,
	}, nil
}

func changedScope(changes []git.FileChange) []string {
	scope := make([]string, 0, len(changes))
	for _, change := range changes {
		rel := patch.NormalizeChromiumPath(change.Path)
		if patch.IsInternalPath(rel) {
			continue
		}
		scope = append(scope, rel)
	}
	return scope
}
