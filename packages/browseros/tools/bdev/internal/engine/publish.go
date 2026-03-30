package engine

import (
	"context"
	"fmt"

	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/git"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/repo"
)

type PublishResult struct {
	Remote  string `json:"remote"`
	Branch  string `json:"branch"`
	Message string `json:"message"`
}

func Publish(ctx context.Context, repoInfo *repo.Info, remote string, message string) (*PublishResult, error) {
	if remote == "" {
		remote = "origin"
	}
	if message == "" {
		message = "chore: update chromium patches"
	}
	dirty, err := git.IsDirtyPaths(ctx, repoInfo.Root, []string{"chromium_patches"})
	if err != nil {
		return nil, err
	}
	if !dirty {
		return nil, fmt.Errorf("nothing to publish: chromium_patches has no uncommitted changes")
	}
	if err := git.AddPaths(ctx, repoInfo.Root, []string{"chromium_patches"}); err != nil {
		return nil, err
	}
	if err := git.Commit(ctx, repoInfo.Root, message); err != nil {
		return nil, err
	}
	branch, err := git.CurrentBranch(ctx, repoInfo.Root)
	if err != nil {
		return nil, err
	}
	if err := git.Push(ctx, repoInfo.Root, remote, branch); err != nil {
		return nil, err
	}
	return &PublishResult{Remote: remote, Branch: branch, Message: message}, nil
}
