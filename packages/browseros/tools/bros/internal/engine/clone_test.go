package engine

import (
	"path/filepath"
	"strings"
	"testing"
)

func TestCloneCleanAppliesPatchAndResetsLocalChanges(t *testing.T) {
	t.Parallel()

	ctx := setupPullFixture(t)
	result, err := Clone(ctx, CloneOpts{
		Clean: true,
	})
	if err != nil {
		t.Fatalf("Clone: %v", err)
	}

	if !contains(result.Applied, "foo.txt") {
		t.Fatalf("expected foo.txt to be applied, got %#v", result.Applied)
	}

	foo := mustRead(t, filepath.Join(ctx.ChromiumDir, "foo.txt"))
	if strings.TrimSpace(foo) != "repo-version" {
		t.Fatalf("expected foo.txt to match patch repo, got %q", foo)
	}

	orphan := mustRead(t, filepath.Join(ctx.ChromiumDir, "orphan.txt"))
	if strings.TrimSpace(orphan) != "orphan-base" {
		t.Fatalf("expected orphan.txt to be reset during clean clone, got %q", orphan)
	}
}

func TestCloneVerifyBaseRejectsMismatchedHead(t *testing.T) {
	t.Parallel()

	ctx := setupPullFixture(t)
	writeFile(t, filepath.Join(ctx.ChromiumDir, "post_base.txt"), "new commit\n")
	runGit(t, ctx.ChromiumDir, "add", "post_base.txt")
	runGit(t, ctx.ChromiumDir, "commit", "-m", "move head")

	_, err := Clone(ctx, CloneOpts{VerifyBase: true})
	if err == nil {
		t.Fatalf("expected verify-base failure when HEAD diverges from BASE_COMMIT")
	}
	if !strings.Contains(err.Error(), "does not match BASE_COMMIT") {
		t.Fatalf("unexpected error: %v", err)
	}
}
