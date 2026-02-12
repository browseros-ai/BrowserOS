package engine

import (
	"fmt"

	"bros/internal/config"
	"bros/internal/git"
	"bros/internal/patch"
)

type PushOpts struct {
	DryRun bool
	Files  []string
}

func Push(ctx *config.Context, opts PushOpts) (*patch.PushResult, error) {
	result := &patch.PushResult{}

	// Phase 1: Discover changed files (working tree vs BASE)
	nameStatus, err := git.DiffNameStatus(ctx.ChromiumDir, ctx.BaseCommit)
	if err != nil {
		return nil, fmt.Errorf("push: discovering changes: %w", err)
	}

	if len(nameStatus) == 0 {
		return result, nil
	}

	// Filter to requested files if specified
	if len(opts.Files) > 0 {
		filtered := make(map[string]patch.FileOp)
		for _, f := range opts.Files {
			if op, ok := nameStatus[f]; ok {
				filtered[f] = op
			}
		}
		nameStatus = filtered
	}

	if len(nameStatus) == 0 {
		return result, nil
	}

	// Phase 2: Generate patches
	var diffOutput []byte
	files := make([]string, 0, len(nameStatus))
	for f := range nameStatus {
		files = append(files, f)
	}

	diffOutput, err = git.DiffFiles(ctx.ChromiumDir, ctx.BaseCommit, files)
	if err != nil {
		return nil, fmt.Errorf("push: generating diffs: %w", err)
	}

	patchSet, err := patch.ParseUnifiedDiff(diffOutput)
	if err != nil {
		return nil, fmt.Errorf("push: parsing diffs: %w", err)
	}
	patchSet.Base = ctx.BaseCommit

	// Merge in deleted files that won't appear in diff output
	for path, op := range nameStatus {
		if op == patch.OpDeleted {
			if _, exists := patchSet.Patches[path]; !exists {
				patchSet.Patches[path] = &patch.FilePatch{
					Path: path,
					Op:   patch.OpDeleted,
				}
			}
		}
	}

	// Classify results for reporting
	existingPatches := make(map[string]bool)
	if existing, err := patch.ReadPatchFiles(ctx.PatchesDir); err == nil {
		for p := range existing {
			existingPatches[p] = true
		}
	}

	for path, fp := range patchSet.Patches {
		switch fp.Op {
		case patch.OpDeleted:
			result.Deleted = append(result.Deleted, path)
		case patch.OpAdded:
			result.Added = append(result.Added, path)
		default:
			if existingPatches[path] {
				result.Modified = append(result.Modified, path)
			} else {
				result.Added = append(result.Added, path)
			}
		}
	}

	// Phase 3: Write patches
	if err := patch.WritePatchSet(ctx.PatchesDir, patchSet, opts.DryRun); err != nil {
		return nil, fmt.Errorf("push: writing patches: %w", err)
	}

	// Phase 4: Stale cleanup
	if !opts.DryRun && len(opts.Files) == 0 {
		stale, err := patch.RemoveStale(ctx.PatchesDir, patchSet, opts.DryRun)
		if err != nil {
			return nil, fmt.Errorf("push: stale cleanup: %w", err)
		}
		result.Stale = stale
	}

	return result, nil
}
