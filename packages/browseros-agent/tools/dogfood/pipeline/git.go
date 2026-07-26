package pipeline

import (
	"context"
	"strings"
)

type Runner interface {
	Run(ctx context.Context, dir string, args ...string) error
	OutputRun(dir string, args ...string) (string, error)
}

func Dirty(repoPath string, r Runner) (bool, error) {
	out, err := r.OutputRun(repoPath, "git", "status", "--porcelain")
	if err != nil {
		return false, err
	}
	return strings.TrimSpace(out) != "", nil
}

func Pull(ctx context.Context, repoPath string, r Runner) error {
	return r.Run(ctx, repoPath, "git", "pull", "--ff-only")
}

func Fetch(ctx context.Context, repoPath string, r Runner) error {
	return r.Run(ctx, repoPath, "git", "fetch", "--prune")
}

func ResetHardToUpstream(ctx context.Context, repoPath string, r Runner) error {
	return r.Run(ctx, repoPath, "git", "reset", "--hard", "@{upstream}")
}

// EnsureBranch moves the configured dogfood checkout onto its target branch before update work runs.
func EnsureBranch(ctx context.Context, repoPath string, branch string, r Runner, force bool) error {
	branch = strings.TrimSpace(branch)
	if branch == "" {
		return nil
	}
	current, err := CurrentBranch(repoPath, r)
	if err != nil {
		return err
	}
	if current == branch {
		return nil
	}
	if force {
		return r.Run(ctx, repoPath, "git", "switch", "--force", branch)
	}
	return r.Run(ctx, repoPath, "git", "switch", branch)
}

func Head(repoPath string, r Runner) (string, error) {
	out, err := r.OutputRun(repoPath, "git", "rev-parse", "--short", "HEAD")
	if err != nil {
		return "", err
	}
	return strings.TrimSpace(out), nil
}

func CurrentBranch(repoPath string, r Runner) (string, error) {
	out, err := r.OutputRun(repoPath, "git", "branch", "--show-current")
	if err != nil {
		return "", err
	}
	return strings.TrimSpace(out), nil
}

func Branch(repoPath string, r Runner) string {
	out, err := CurrentBranch(repoPath, r)
	if err != nil {
		return ""
	}
	return out
}
