package engine

import (
	"path/filepath"
	"strings"
	"testing"
)

func TestPushIncludesUntrackedFiles(t *testing.T) {
	t.Parallel()

	ctx := setupPullFixture(t)
	if err := resetPathToBase(ctx, "orphan.txt"); err != nil {
		t.Fatalf("reset orphan.txt: %v", err)
	}

	writeFile(t, filepath.Join(ctx.ChromiumDir, "foo.txt"), "repo-version-v2\n")
	writeFile(t, filepath.Join(ctx.ChromiumDir, "new", "file.txt"), "brand-new\n")

	dryRun, err := Push(ctx, PushOpts{DryRun: true})
	if err != nil {
		t.Fatalf("Push dry-run: %v", err)
	}
	if !contains(dryRun.Modified, "foo.txt") {
		t.Fatalf("expected foo.txt in modified set, got %#v", dryRun.Modified)
	}
	if !contains(dryRun.Added, "new/file.txt") {
		t.Fatalf("expected new/file.txt in added set, got %#v", dryRun.Added)
	}

	result, err := Push(ctx, PushOpts{})
	if err != nil {
		t.Fatalf("Push: %v", err)
	}
	if !contains(result.Added, "new/file.txt") {
		t.Fatalf("expected new/file.txt in added result, got %#v", result.Added)
	}

	patchContent := mustRead(t, filepath.Join(ctx.PatchesDir, "new", "file.txt"))
	if !strings.Contains(patchContent, "diff --git a/new/file.txt b/new/file.txt") {
		t.Fatalf("unexpected patch content for untracked file:\n%s", patchContent)
	}
}
