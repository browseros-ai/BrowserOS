package engine

import (
	"fmt"

	"bros/internal/config"
	"bros/internal/git"
	"bros/internal/patch"
)

type StatusResult struct {
	CheckoutName    string
	BaseCommit      string
	ChromiumVersion string
	PatchesRepo     string
	Ahead           int
	Behind          int
	Synced          int
	AheadFiles      []string
	BehindFiles     []string
	SyncedFiles     []string
}

func Status(ctx *config.Context, showFiles bool) (*StatusResult, error) {
	result := &StatusResult{
		CheckoutName:    ctx.Config.Name,
		BaseCommit:      ctx.BaseCommit,
		ChromiumVersion: ctx.ChromiumVersion,
		PatchesRepo:     ctx.PatchesRepo,
	}

	// Read repo patches
	repoPatchSet, err := patch.ReadPatchSet(ctx.PatchesDir)
	if err != nil {
		return nil, fmt.Errorf("status: reading repo patches: %w", err)
	}

	// Read local state (working tree vs BASE)
	diffOutput, err := git.DiffFull(ctx.ChromiumDir, ctx.BaseCommit)
	if err != nil {
		return nil, fmt.Errorf("status: reading local diffs: %w", err)
	}

	localPatchSet, err := patch.ParseUnifiedDiff(diffOutput)
	if err != nil {
		return nil, fmt.Errorf("status: parsing local diffs: %w", err)
	}

	delta := patch.Compare(localPatchSet, repoPatchSet)
	actionableDeleted := existingPaths(ctx.ChromiumDir, delta.Deleted)
	deletedSynced := len(delta.Deleted) - len(actionableDeleted)

	result.Ahead = len(delta.Orphaned)
	result.Behind = len(delta.NeedsApply) + len(delta.NeedsUpdate) + len(actionableDeleted)
	result.Synced = len(delta.UpToDate) + deletedSynced

	if showFiles {
		result.AheadFiles = delta.Orphaned
		result.BehindFiles = append(result.BehindFiles, delta.NeedsApply...)
		result.BehindFiles = append(result.BehindFiles, delta.NeedsUpdate...)
		result.BehindFiles = append(result.BehindFiles, actionableDeleted...)
		result.SyncedFiles = delta.UpToDate
	}

	return result, nil
}
