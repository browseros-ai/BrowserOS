package engine

import (
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"bros/internal/config"
	"bros/internal/git"
	"bros/internal/patch"
)

type PullOpts struct {
	DryRun bool
	Files  []string
}

func Pull(ctx *config.Context, opts PullOpts) (*patch.PullResult, error) {
	repoPatchSet, err := patch.ReadPatchSet(ctx.PatchesDir)
	if err != nil {
		return nil, fmt.Errorf("pull: reading repo patches: %w", err)
	}

	// Incremental mode: when we know the last pulled patches repo revision,
	// apply only changed chromium patch paths in that revision range.
	if len(opts.Files) == 0 {
		repoRev, err := git.HeadRev(ctx.PatchesRepo)
		if err == nil {
			if fromRev, ok := incrementalFromRev(ctx, repoRev); ok {
				changed, err := changedChromiumPathsFromPatchRange(ctx.PatchesRepo, fromRev, repoRev)
				if err != nil {
					return nil, fmt.Errorf("pull: reading changed patch files: %w", err)
				}
				return pullChangedPaths(ctx, repoPatchSet, changed, opts.DryRun)
			}
		}
	}

	return pullReconcile(ctx, repoPatchSet, opts)
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
	for _, f := range d.Orphaned {
		if fileSet[f] {
			filtered.Orphaned = append(filtered.Orphaned, f)
		}
	}
	return filtered
}

func pullReconcile(ctx *config.Context, repoPatchSet *patch.PatchSet, opts PullOpts) (*patch.PullResult, error) {
	result := &patch.PullResult{}

	diffOutput, err := git.DiffFull(ctx.ChromiumDir, ctx.BaseCommit)
	if err != nil {
		return nil, fmt.Errorf("pull: reading local diffs: %w", err)
	}

	localPatchSet, err := patch.ParseUnifiedDiff(diffOutput)
	if err != nil {
		return nil, fmt.Errorf("pull: parsing local diffs: %w", err)
	}

	delta := patch.Compare(localPatchSet, repoPatchSet)
	if len(opts.Files) > 0 {
		delta = filterDelta(delta, opts.Files)
	}

	actionableDeleted := existingPaths(ctx.ChromiumDir, delta.Deleted)

	if opts.DryRun {
		result.Applied = append(result.Applied, delta.NeedsUpdate...)
		result.Applied = append(result.Applied, delta.NeedsApply...)
		result.Reset = append(result.Reset, delta.Orphaned...)
		result.Skipped = append(result.Skipped, delta.UpToDate...)
		result.Deleted = append(result.Deleted, actionableDeleted...)
		sort.Strings(result.Applied)
		sort.Strings(result.Reset)
		sort.Strings(result.Skipped)
		sort.Strings(result.Deleted)
		return result, nil
	}

	// Reset update candidates and orphaned local-only files back to BASE.
	resetCandidates := append([]string{}, delta.NeedsUpdate...)
	resetCandidates = append(resetCandidates, delta.Orphaned...)
	if err := resetFilesToBase(ctx, resetCandidates); err != nil {
		return nil, fmt.Errorf("pull: resetting files to base: %w", err)
	}
	result.Reset = append(result.Reset, delta.Orphaned...)

	// Apply repo patches for changed files.
	filesToApply := make([]string, 0, len(delta.NeedsUpdate)+len(delta.NeedsApply))
	filesToApply = append(filesToApply, delta.NeedsUpdate...)
	filesToApply = append(filesToApply, delta.NeedsApply...)
	sort.Strings(filesToApply)

	for _, path := range filesToApply {
		repoPatch, ok := repoPatchSet.Patches[path]
		if !ok || repoPatch.Content == nil {
			continue
		}

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

	// Delete files for actionable .deleted markers.
	for _, path := range actionableDeleted {
		target := filepath.Join(ctx.ChromiumDir, path)
		if err := os.Remove(target); err != nil {
			return nil, fmt.Errorf("pull: deleting %s: %w", path, err)
		}
		result.Deleted = append(result.Deleted, path)
	}

	result.Skipped = append(result.Skipped, delta.UpToDate...)
	sort.Strings(result.Applied)
	sort.Strings(result.Reset)
	sort.Strings(result.Skipped)
	sort.Strings(result.Deleted)

	return result, nil
}

func pullChangedPaths(
	ctx *config.Context,
	repoPatchSet *patch.PatchSet,
	changedPaths []string,
	dryRun bool,
) (*patch.PullResult, error) {
	result := &patch.PullResult{}
	sort.Strings(changedPaths)

	for _, path := range changedPaths {
		repoPatch, exists := repoPatchSet.Patches[path]

		// Patch removed in repo: reset Chromium file to BASE.
		if !exists {
			if dryRun {
				if git.FileExistsInCommit(ctx.ChromiumDir, ctx.BaseCommit, path) {
					result.Reset = append(result.Reset, path)
				} else if fileExistsInWorkingTree(ctx.ChromiumDir, path) {
					result.Deleted = append(result.Deleted, path)
				}
				continue
			}

			wasDeleted, err := resetFileToBase(ctx, path)
			if err != nil {
				return nil, fmt.Errorf("pull: resetting removed patch %s: %w", path, err)
			}
			if wasDeleted {
				result.Deleted = append(result.Deleted, path)
			} else {
				result.Reset = append(result.Reset, path)
			}
			continue
		}

		switch repoPatch.Op {
		case patch.OpDeleted:
			if dryRun {
				if fileExistsInWorkingTree(ctx.ChromiumDir, path) {
					result.Deleted = append(result.Deleted, path)
				}
				continue
			}
			target := filepath.Join(ctx.ChromiumDir, path)
			if fileExistsInWorkingTree(ctx.ChromiumDir, path) {
				if err := os.Remove(target); err != nil {
					return nil, fmt.Errorf("pull: deleting %s: %w", path, err)
				}
				result.Deleted = append(result.Deleted, path)
			}
		case patch.OpBinary:
			// No binary payload to apply.
			result.Skipped = append(result.Skipped, path)
		default:
			if dryRun {
				result.Applied = append(result.Applied, path)
				continue
			}

			if _, err := resetFileToBase(ctx, path); err != nil {
				return nil, fmt.Errorf("pull: resetting %s to base: %w", path, err)
			}

			if repoPatch.Content == nil {
				result.Skipped = append(result.Skipped, path)
				continue
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
	}

	sort.Strings(result.Applied)
	sort.Strings(result.Reset)
	sort.Strings(result.Skipped)
	sort.Strings(result.Deleted)
	return result, nil
}

func incrementalFromRev(ctx *config.Context, repoRev string) (string, bool) {
	if ctx.State == nil || ctx.State.LastPull == nil {
		return "", false
	}
	fromRev := strings.TrimSpace(ctx.State.LastPull.PatchesRepoRev)
	if fromRev == "" || fromRev == repoRev {
		return "", false
	}
	if !git.CommitExists(ctx.PatchesRepo, fromRev) {
		return "", false
	}
	return fromRev, true
}

func changedChromiumPathsFromPatchRange(patchesRepo, fromRev, toRev string) ([]string, error) {
	changes, err := git.DiffNameStatusRange(patchesRepo, fromRev, toRev, "chromium_patches")
	if err != nil {
		return nil, err
	}

	return collectChangedChromiumPaths(changes), nil
}

func collectChangedChromiumPaths(changes []git.NameStatusEntry) []string {
	pathSet := make(map[string]bool)
	for _, c := range changes {
		if p, ok := repoPatchPathToChromiumPath(c.Path); ok {
			pathSet[p] = true
		}
		if c.OldPath != "" {
			if p, ok := repoPatchPathToChromiumPath(c.OldPath); ok {
				pathSet[p] = true
			}
		}
	}

	var result []string
	for p := range pathSet {
		result = append(result, p)
	}
	sort.Strings(result)
	return result
}

func repoPatchPathToChromiumPath(path string) (string, bool) {
	const prefix = "chromium_patches/"
	if !strings.HasPrefix(path, prefix) {
		return "", false
	}

	rel := strings.TrimPrefix(path, prefix)
	rel = strings.TrimSuffix(rel, ".deleted")
	rel = strings.TrimSuffix(rel, ".binary")
	rel = strings.TrimSuffix(rel, ".rename")
	if strings.TrimSpace(rel) == "" {
		return "", false
	}

	return rel, true
}

func resetFilesToBase(ctx *config.Context, files []string) error {
	if len(files) == 0 {
		return nil
	}

	seen := make(map[string]bool)
	var checkoutFiles []string
	for _, path := range files {
		if seen[path] {
			continue
		}
		seen[path] = true

		if git.FileExistsInCommit(ctx.ChromiumDir, ctx.BaseCommit, path) {
			checkoutFiles = append(checkoutFiles, path)
			continue
		}

		target := filepath.Join(ctx.ChromiumDir, path)
		if err := os.Remove(target); err != nil && !os.IsNotExist(err) {
			return fmt.Errorf("removing %s: %w", path, err)
		}
	}

	if len(checkoutFiles) > 0 {
		sort.Strings(checkoutFiles)
		if err := git.CheckoutFiles(ctx.ChromiumDir, ctx.BaseCommit, checkoutFiles); err != nil {
			return err
		}
	}

	return nil
}

// resetFileToBase resets one path to BASE. Returns true when the operation
// deleted the working-tree file (because it doesn't exist in BASE).
func resetFileToBase(ctx *config.Context, path string) (bool, error) {
	if git.FileExistsInCommit(ctx.ChromiumDir, ctx.BaseCommit, path) {
		if err := git.CheckoutFiles(ctx.ChromiumDir, ctx.BaseCommit, []string{path}); err != nil {
			return false, err
		}
		return false, nil
	}

	target := filepath.Join(ctx.ChromiumDir, path)
	if err := os.Remove(target); err != nil && !os.IsNotExist(err) {
		return false, err
	}
	return true, nil
}

func existingPaths(root string, paths []string) []string {
	var result []string
	for _, path := range paths {
		if fileExistsInWorkingTree(root, path) {
			result = append(result, path)
		}
	}
	return result
}

func fileExistsInWorkingTree(root, path string) bool {
	_, err := os.Stat(filepath.Join(root, path))
	return err == nil
}
