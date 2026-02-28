package engine

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"bros/internal/config"
)

func TestPullKeepsLocalOnlyFilesByDefault(t *testing.T) {
	t.Parallel()

	ctx := setupPullFixture(t)
	result, err := Pull(ctx, PullOpts{KeepLocalOnly: true})
	if err != nil {
		t.Fatalf("Pull: %v", err)
	}

	if !contains(result.Applied, "foo.txt") {
		t.Fatalf("expected foo.txt to be applied, got %#v", result.Applied)
	}
	if !contains(result.LocalOnly, "orphan.txt") {
		t.Fatalf("expected orphan.txt in local-only list, got %#v", result.LocalOnly)
	}
	if contains(result.Reverted, "orphan.txt") {
		t.Fatalf("orphan.txt should not be reverted when KeepLocalOnly=true")
	}

	fooContent := mustRead(t, filepath.Join(ctx.ChromiumDir, "foo.txt"))
	if strings.TrimSpace(fooContent) != "repo-version" {
		t.Fatalf("unexpected foo.txt content: %q", fooContent)
	}

	orphanContent := mustRead(t, filepath.Join(ctx.ChromiumDir, "orphan.txt"))
	if strings.TrimSpace(orphanContent) != "local-only-change" {
		t.Fatalf("orphan.txt should be preserved, got %q", orphanContent)
	}
}

func TestPullRevertsLocalOnlyWhenDisabled(t *testing.T) {
	t.Parallel()

	ctx := setupPullFixture(t)
	result, err := Pull(ctx, PullOpts{KeepLocalOnly: false})
	if err != nil {
		t.Fatalf("Pull: %v", err)
	}

	if !contains(result.Reverted, "orphan.txt") {
		t.Fatalf("expected orphan.txt to be reverted, got %#v", result.Reverted)
	}
	if contains(result.LocalOnly, "orphan.txt") {
		t.Fatalf("orphan.txt should not be local-only when KeepLocalOnly=false")
	}

	orphanContent := mustRead(t, filepath.Join(ctx.ChromiumDir, "orphan.txt"))
	if strings.TrimSpace(orphanContent) != "orphan-base" {
		t.Fatalf("orphan.txt should be reset to base, got %q", orphanContent)
	}
}

func setupPullFixture(t *testing.T) *config.Context {
	t.Helper()

	root := t.TempDir()
	chromiumDir := filepath.Join(root, "chromium")
	patchesRepo := filepath.Join(root, "patches")

	if err := os.MkdirAll(chromiumDir, 0o755); err != nil {
		t.Fatalf("mkdir chromium: %v", err)
	}
	if err := os.MkdirAll(patchesRepo, 0o755); err != nil {
		t.Fatalf("mkdir patches: %v", err)
	}

	initRepo(t, chromiumDir)
	writeFile(t, filepath.Join(chromiumDir, "foo.txt"), "base\n")
	writeFile(t, filepath.Join(chromiumDir, "orphan.txt"), "orphan-base\n")
	runGit(t, chromiumDir, "add", "foo.txt", "orphan.txt")
	runGit(t, chromiumDir, "commit", "-m", "base")
	baseCommit := strings.TrimSpace(runGit(t, chromiumDir, "rev-parse", "HEAD"))

	writeFile(t, filepath.Join(chromiumDir, "foo.txt"), "repo-version\n")
	patchDiff := runGit(t, chromiumDir, "diff", "--full-index", baseCommit, "--", "foo.txt")
	if strings.TrimSpace(patchDiff) == "" {
		t.Fatalf("expected patch diff for foo.txt")
	}
	runGit(t, chromiumDir, "checkout", baseCommit, "--", "foo.txt")

	writeFile(t, filepath.Join(chromiumDir, "orphan.txt"), "local-only-change\n")

	initRepo(t, patchesRepo)
	writeFile(t, filepath.Join(patchesRepo, "BASE_COMMIT"), baseCommit+"\n")
	writeFile(t, filepath.Join(patchesRepo, "chromium_patches", "foo.txt"), patchDiff)
	runGit(t, patchesRepo, "add", ".")
	runGit(t, patchesRepo, "commit", "-m", "seed patch repo")

	return &config.Context{
		Config:      &config.Config{Name: "test-checkout", PatchesRepo: patchesRepo},
		State:       &config.State{},
		ChromiumDir: chromiumDir,
		BrosDir:     filepath.Join(chromiumDir, ".bros"),
		PatchesRepo: patchesRepo,
		PatchesDir:  filepath.Join(patchesRepo, "chromium_patches"),
		BaseCommit:  baseCommit,
	}
}

func initRepo(t *testing.T, dir string) {
	t.Helper()
	runGit(t, dir, "init")
	runGit(t, dir, "config", "user.email", "bros-test@example.com")
	runGit(t, dir, "config", "user.name", "bros test")
}

func runGit(t *testing.T, dir string, args ...string) string {
	t.Helper()
	cmd := exec.Command("git", args...)
	cmd.Dir = dir
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("git %v failed: %v\n%s", args, err, string(out))
	}
	return string(out)
}

func contains(items []string, target string) bool {
	for _, item := range items {
		if item == target {
			return true
		}
	}
	return false
}

func writeFile(t *testing.T, path, content string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatalf("mkdir %s: %v", path, err)
	}
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatalf("write %s: %v", path, err)
	}
}

func mustRead(t *testing.T, path string) string {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read %s: %v", path, err)
	}
	return string(data)
}
