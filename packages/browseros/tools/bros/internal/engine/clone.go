package engine

import (
	"fmt"
	"os"
	"path/filepath"

	"bros/internal/config"
	"bros/internal/git"
	"bros/internal/patch"
)

type CloneOpts struct {
	VerifyBase bool
	Clean      bool
	DryRun     bool
}

func Clone(ctx *config.Context, opts CloneOpts) (*patch.PullResult, error) {
	result := &patch.PullResult{}

	// Verify HEAD matches BASE if requested
	if opts.VerifyBase {
		head, err := git.RevParse(ctx.ChromiumDir, "HEAD")
		if err != nil {
			return nil, fmt.Errorf("clone: getting HEAD: %w", err)
		}
		base, err := git.RevParse(ctx.ChromiumDir, ctx.BaseCommit)
		if err != nil {
			return nil, fmt.Errorf("clone: resolving BASE_COMMIT %s: %w", ctx.BaseCommit, err)
		}
		if head != base {
			return nil, fmt.Errorf("clone: HEAD (%s) does not match BASE_COMMIT (%s) — use --verify-base=false to skip", head[:12], base[:12])
		}
	}

	// Clean: reset all modified files to base before applying
	if opts.Clean && !opts.DryRun {
		nameStatus, err := git.DiffNameStatus(ctx.ChromiumDir, ctx.BaseCommit)
		if err != nil {
			return nil, fmt.Errorf("clone: discovering local changes: %w", err)
		}

		if len(nameStatus) > 0 {
			var checkoutFiles []string
			for path := range nameStatus {
				if git.FileExistsInCommit(ctx.ChromiumDir, ctx.BaseCommit, path) {
					checkoutFiles = append(checkoutFiles, path)
				} else {
					// File doesn't exist in base — remove it
					_ = os.Remove(filepath.Join(ctx.ChromiumDir, path))
				}
			}
			if len(checkoutFiles) > 0 {
				if err := git.CheckoutFiles(ctx.ChromiumDir, ctx.BaseCommit, checkoutFiles); err != nil {
					return nil, fmt.Errorf("clone: resetting to base: %w", err)
				}
			}
		}
	}

	// Read all patches from repo
	repoPatchSet, err := patch.ReadPatchSet(ctx.PatchesDir)
	if err != nil {
		return nil, fmt.Errorf("clone: reading patches: %w", err)
	}

	if opts.DryRun {
		for path, fp := range repoPatchSet.Patches {
			if fp.Op == patch.OpDeleted {
				result.Deleted = append(result.Deleted, path)
			} else {
				result.Applied = append(result.Applied, path)
			}
		}
		return result, nil
	}

	// Apply all patches
	for path, fp := range repoPatchSet.Patches {
		switch fp.Op {
		case patch.OpDeleted:
			target := filepath.Join(ctx.ChromiumDir, path)
			if _, err := os.Stat(target); err == nil {
				if err := os.Remove(target); err != nil {
					return nil, fmt.Errorf("clone: deleting %s: %w", path, err)
				}
				result.Deleted = append(result.Deleted, path)
			}

		case patch.OpBinary:
			// Skip binary files with no content
			continue

		default:
			if fp.Content == nil {
				continue
			}
			// Remove existing file if it's not in BASE (untracked new-file).
			// git diff can't see untracked files, so --clean misses them.
			if !git.FileExistsInCommit(ctx.ChromiumDir, ctx.BaseCommit, path) {
				_ = os.Remove(filepath.Join(ctx.ChromiumDir, path))
			}
			patchFile := filepath.Join(ctx.PatchesDir, path)
			conflict, err := git.Apply(ctx.ChromiumDir, fp.Content, patchFile)
			if err != nil {
				return nil, fmt.Errorf("clone: applying %s: %w", path, err)
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

	return result, nil
}
