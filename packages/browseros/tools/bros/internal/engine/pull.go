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

	repoHead, err := git.HeadRev(ctx.PatchesRepo)
	if err != nil {
		return nil, fmt.Errorf("pull: reading patches repo HEAD: %w", err)
	}

	incrementalPaths, shouldUseIncremental, err := resolveIncrementalPaths(ctx, repoHead, opts.Files)
	if err != nil {
		return nil, fmt.Errorf("pull: resolving incremental scope: %w", err)
	}

	if shouldUseIncremental {
		return incrementalPull(ctx, repoPatchSet, incrementalPaths, opts.DryRun)
	}

	return fullPull(ctx, repoPatchSet, opts)
}

func resolveIncrementalPaths(ctx *config.Context, repoHead string, filesFilter []string) ([]string, bool, error) {
	if len(filesFilter) > 0 {
		return nil, false, nil
	}

	if ctx.State == nil || ctx.State.LastPull == nil {
		return nil, false, nil
	}

	lastPull := ctx.State.LastPull
	if strings.TrimSpace(lastPull.PatchesRepoRev) == "" {
		return nil, false, nil
	}

	if lastPull.BaseCommit != ctx.BaseCommit {
		return nil, false, nil
	}

	if !git.CommitExists(ctx.PatchesRepo, lastPull.PatchesRepoRev) {
		return nil, false, nil
	}

	if lastPull.PatchesRepoRev == repoHead {
		return []string{}, true, nil
	}

	repoPaths, err := git.DiffChangedPathsBetween(
		ctx.PatchesRepo,
		lastPull.PatchesRepoRev,
		repoHead,
		"chromium_patches",
	)
	if err != nil {
		return nil, false, err
	}

	seen := make(map[string]bool)
	for _, repoPath := range repoPaths {
		chromiumPath, ok := normalizeRepoPatchPath(repoPath)
		if !ok {
			continue
		}
		seen[chromiumPath] = true
	}

	paths := make([]string, 0, len(seen))
	for p := range seen {
		paths = append(paths, p)
	}
	sort.Strings(paths)
	return paths, true, nil
}

func normalizeRepoPatchPath(repoPath string) (string, bool) {
	p := filepath.ToSlash(strings.TrimSpace(repoPath))
	if !strings.HasPrefix(p, "chromium_patches/") {
		return "", false
	}

	chromiumPath := strings.TrimPrefix(p, "chromium_patches/")
	chromiumPath = strings.TrimSuffix(chromiumPath, ".deleted")
	chromiumPath = strings.TrimSuffix(chromiumPath, ".binary")
	chromiumPath = strings.TrimSuffix(chromiumPath, ".rename")
	if chromiumPath == "" {
		return "", false
	}

	return chromiumPath, true
}

func incrementalPull(
	ctx *config.Context,
	repoPatchSet *patch.PatchSet,
	paths []string,
	dryRun bool,
) (*patch.PullResult, error) {
	result := &patch.PullResult{}

	for _, path := range paths {
		repoPatch, exists := repoPatchSet.Patches[path]
		if !exists {
			if !dryRun {
				if err := resetPathToBase(ctx, path); err != nil {
					return nil, fmt.Errorf("pull: reverting removed patch %s: %w", path, err)
				}
			}
			result.Reverted = append(result.Reverted, path)
			continue
		}

		switch repoPatch.Op {
		case patch.OpDeleted:
			if !dryRun {
				if err := deletePath(ctx, path); err != nil {
					return nil, err
				}
			}
			result.Deleted = append(result.Deleted, path)
		case patch.OpBinary:
			result.Skipped = append(result.Skipped, path)
		default:
			if !dryRun {
				if err := resetPathToBase(ctx, path); err != nil {
					return nil, fmt.Errorf("pull: resetting %s to base: %w", path, err)
				}
				if err := applyRepoPatch(ctx, repoPatch, path, result); err != nil {
					return nil, err
				}
			} else {
				result.Applied = append(result.Applied, path)
			}
		}
	}

	return result, nil
}

func fullPull(ctx *config.Context, repoPatchSet *patch.PatchSet, opts PullOpts) (*patch.PullResult, error) {
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

	if opts.DryRun {
		result.Applied = append(delta.NeedsUpdate, delta.NeedsApply...)
		result.Skipped = delta.UpToDate
		result.Deleted = delta.Deleted
		result.Reverted = delta.Orphaned
		return result, nil
	}

	filesToReset := make([]string, 0, len(delta.NeedsUpdate)+len(delta.Orphaned))
	filesToReset = append(filesToReset, delta.NeedsUpdate...)
	filesToReset = append(filesToReset, delta.Orphaned...)
	for _, path := range filesToReset {
		if err := resetPathToBase(ctx, path); err != nil {
			return nil, fmt.Errorf("pull: resetting %s to base: %w", path, err)
		}
	}
	result.Reverted = append(result.Reverted, delta.Orphaned...)

	filesToApply := make([]string, 0, len(delta.NeedsUpdate)+len(delta.NeedsApply))
	filesToApply = append(filesToApply, delta.NeedsUpdate...)
	filesToApply = append(filesToApply, delta.NeedsApply...)
	for _, path := range filesToApply {
		repoPatch, ok := repoPatchSet.Patches[path]
		if !ok || repoPatch.Op == patch.OpDeleted || repoPatch.Op == patch.OpBinary {
			continue
		}
		if err := applyRepoPatch(ctx, repoPatch, path, result); err != nil {
			return nil, err
		}
	}

	for _, path := range delta.Deleted {
		if err := deletePath(ctx, path); err != nil {
			return nil, err
		}
		result.Deleted = append(result.Deleted, path)
	}

	result.Skipped = delta.UpToDate
	return result, nil
}

func applyRepoPatch(
	ctx *config.Context,
	repoPatch *patch.FilePatch,
	path string,
	result *patch.PullResult,
) error {
	patchContent := repoPatch.Content
	patchFile := filepath.Join(ctx.PatchesDir, path)

	if len(patchContent) == 0 {
		onDiskContent, err := os.ReadFile(patchFile)
		if err == nil {
			patchContent = onDiskContent
		}
	}
	if len(patchContent) == 0 {
		result.Skipped = append(result.Skipped, path)
		return nil
	}

	if !git.FileExistsInCommit(ctx.ChromiumDir, ctx.BaseCommit, path) {
		_ = os.Remove(filepath.Join(ctx.ChromiumDir, path))
	}

	conflict, err := git.Apply(ctx.ChromiumDir, patchContent, patchFile)
	if err != nil {
		return fmt.Errorf("pull: applying %s: %w", path, err)
	}

	if conflict != nil {
		conflict.File = path
		conflict.RejectFile = path + ".rej"
		result.Conflicts = append(result.Conflicts, *conflict)
	} else {
		result.Applied = append(result.Applied, path)
	}

	return nil
}

func resetPathToBase(ctx *config.Context, chromiumPath string) error {
	if git.FileExistsInCommit(ctx.ChromiumDir, ctx.BaseCommit, chromiumPath) {
		return git.CheckoutFiles(ctx.ChromiumDir, ctx.BaseCommit, []string{chromiumPath})
	}

	target := filepath.Join(ctx.ChromiumDir, chromiumPath)
	if err := os.Remove(target); err != nil && !os.IsNotExist(err) {
		return err
	}
	return nil
}

func deletePath(ctx *config.Context, chromiumPath string) error {
	target := filepath.Join(ctx.ChromiumDir, chromiumPath)
	if err := os.Remove(target); err != nil && !os.IsNotExist(err) {
		return fmt.Errorf("pull: deleting %s: %w", chromiumPath, err)
	}
	return nil
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
	for _, f := range d.Orphaned {
		if fileSet[f] {
			filtered.Orphaned = append(filtered.Orphaned, f)
		}
	}
	for _, f := range d.Deleted {
		if fileSet[f] {
			filtered.Deleted = append(filtered.Deleted, f)
		}
	}
	return filtered
}
