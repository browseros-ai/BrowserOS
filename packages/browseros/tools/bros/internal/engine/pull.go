package engine

import (
	"fmt"
	"os"
	"path/filepath"

	"bros/internal/config"
	"bros/internal/git"
	"bros/internal/patch"
)

type PullOpts struct {
	DryRun bool
	Files  []string
}

func Pull(ctx *config.Context, opts PullOpts) (*patch.PullResult, error) {
	result := &patch.PullResult{}

	// Phase 1: Read repo patches
	repoPatchSet, err := patch.ReadPatchSet(ctx.PatchesDir)
	if err != nil {
		return nil, fmt.Errorf("pull: reading repo patches: %w", err)
	}

	// Phase 2: Read local state (working tree vs BASE)
	diffOutput, err := git.DiffFull(ctx.ChromiumDir, ctx.BaseCommit)
	if err != nil {
		return nil, fmt.Errorf("pull: reading local diffs: %w", err)
	}

	localPatchSet, err := patch.ParseUnifiedDiff(diffOutput)
	if err != nil {
		return nil, fmt.Errorf("pull: parsing local diffs: %w", err)
	}

	// Phase 3: Compare
	delta := patch.Compare(localPatchSet, repoPatchSet)

	// Filter to requested files if specified
	if len(opts.Files) > 0 {
		delta = filterDelta(delta, opts.Files)
	}

	if opts.DryRun {
		// Report what would happen without doing it
		result.Applied = append(delta.NeedsUpdate, delta.NeedsApply...)
		result.Skipped = delta.UpToDate
		result.Deleted = delta.Deleted
		return result, nil
	}

	// Phase 4: Reset NeedsUpdate files to base before reapplying.
	// NeedsApply files are already at base state (no local diff), so skip them.
	// Use git cat-file -e to check if file exists in base before checkout.
	var checkoutFiles []string
	for _, path := range delta.NeedsUpdate {
		if git.FileExistsInCommit(ctx.ChromiumDir, ctx.BaseCommit, path) {
			checkoutFiles = append(checkoutFiles, path)
		} else {
			// File doesn't exist in base — remove it so patch can recreate it
			_ = os.Remove(filepath.Join(ctx.ChromiumDir, path))
		}
	}
	if len(checkoutFiles) > 0 {
		if err := git.CheckoutFiles(ctx.ChromiumDir, ctx.BaseCommit, checkoutFiles); err != nil {
			return nil, fmt.Errorf("pull: resetting files to base: %w", err)
		}
	}

	// Phase 5: Apply patches (NeedsUpdate + NeedsApply)
	filesToApply := make([]string, 0, len(delta.NeedsUpdate)+len(delta.NeedsApply))
	filesToApply = append(filesToApply, delta.NeedsUpdate...)
	filesToApply = append(filesToApply, delta.NeedsApply...)
	for _, path := range filesToApply {
		repoPatch, ok := repoPatchSet.Patches[path]
		if !ok || repoPatch.Content == nil {
			continue
		}

		// Remove existing file if it's not in BASE (untracked new-file).
		// git diff can't see untracked files, so they're invisible to Compare.
		if !git.FileExistsInCommit(ctx.ChromiumDir, ctx.BaseCommit, path) {
			_ = os.Remove(filepath.Join(ctx.ChromiumDir, path))
		}

		patchFile := filepath.Join(ctx.PatchesDir, path)
		conflict, err := git.Apply(ctx.ChromiumDir, repoPatch.Content, patchFile)
		if err != nil {
			return nil, fmt.Errorf("pull: applying %s: %w", path, err)
		}

		if conflict != nil {
			conflict.File = path
			conflict.RejectFile = path + ".rej"
			result.Conflicts = append(result.Conflicts, *conflict)
		} else {
			result.Applied = append(result.Applied, path)
		}
	}

	// Phase 6: Handle .deleted markers
	for _, path := range delta.Deleted {
		target := filepath.Join(ctx.ChromiumDir, path)
		if _, err := os.Stat(target); err == nil {
			if err := os.Remove(target); err != nil {
				return nil, fmt.Errorf("pull: deleting %s: %w", path, err)
			}
			result.Deleted = append(result.Deleted, path)
		}
	}

	result.Skipped = delta.UpToDate

	return result, nil
}

func filterDelta(d *patch.Delta, files []string) *patch.Delta {
	fileSet := make(map[string]bool)
	for _, f := range files {
		fileSet[f] = true
	}

	filtered := &patch.Delta{}
	for _, f := range d.NeedsUpdate {
		if fileSet[f] {
			filtered.NeedsUpdate = append(filtered.NeedsUpdate, f)
		}
	}
	for _, f := range d.NeedsApply {
		if fileSet[f] {
			filtered.NeedsApply = append(filtered.NeedsApply, f)
		}
	}
	for _, f := range d.UpToDate {
		if fileSet[f] {
			filtered.UpToDate = append(filtered.UpToDate, f)
		}
	}
	for _, f := range d.Deleted {
		if fileSet[f] {
			filtered.Deleted = append(filtered.Deleted, f)
		}
	}
	return filtered
}
