package engine

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"bros/internal/config"
	"bros/internal/git"
)

func TestCollectChangedChromiumPaths(t *testing.T) {
	changes := []git.NameStatusEntry{
		{RawStatus: "A", Path: "chromium_patches/chrome/browser/new_file.cc"},
		{RawStatus: "M", Path: "chromium_patches/chrome/browser/toggle.cc.deleted"},
		{
			RawStatus: "R100",
			OldPath:   "chromium_patches/chrome/browser/old_name.cc",
			Path:      "chromium_patches/chrome/browser/new_name.cc",
		},
		{RawStatus: "M", Path: "docs/readme.md"},
	}

	got := collectChangedChromiumPaths(changes)
	want := []string{
		"chrome/browser/new_file.cc",
		"chrome/browser/new_name.cc",
		"chrome/browser/old_name.cc",
		"chrome/browser/toggle.cc",
	}

	if !reflect.DeepEqual(got, want) {
		t.Fatalf("unexpected changed paths\nwant: %#v\ngot:  %#v", want, got)
	}
}

func TestRepoPatchPathToChromiumPath(t *testing.T) {
	tests := []struct {
		input string
		want  string
		ok    bool
	}{
		{"chromium_patches/chrome/a.cc", "chrome/a.cc", true},
		{"chromium_patches/chrome/a.cc.deleted", "chrome/a.cc", true},
		{"chromium_patches/chrome/a.cc.binary", "chrome/a.cc", true},
		{"chromium_patches/chrome/a.cc.rename", "chrome/a.cc", true},
		{"docs/readme.md", "", false},
	}

	for _, tt := range tests {
		got, ok := repoPatchPathToChromiumPath(tt.input)
		if got != tt.want || ok != tt.ok {
			t.Fatalf("repoPatchPathToChromiumPath(%q) => (%q,%v), want (%q,%v)", tt.input, got, ok, tt.want, tt.ok)
		}
	}
}

func TestPull_ResetsRemovedPatchFromIncrementalRange(t *testing.T) {
	chromiumDir := t.TempDir()
	initGitRepo(t, chromiumDir)

	target := filepath.Join(chromiumDir, "chrome", "browser", "a.cc")
	writeFile(t, target, "base\n")
	gitRun(t, chromiumDir, "add", ".")
	gitRun(t, chromiumDir, "commit", "-m", "base")
	baseRev := strings.TrimSpace(gitRun(t, chromiumDir, "rev-parse", "HEAD"))

	// Simulate previously-applied patch in working tree.
	writeFile(t, target, "patched\n")

	patchesRepo := t.TempDir()
	initGitRepo(t, patchesRepo)
	patchesDir := filepath.Join(patchesRepo, "chromium_patches", "chrome", "browser")
	if err := os.MkdirAll(patchesDir, 0o755); err != nil {
		t.Fatalf("mkdir patches dir: %v", err)
	}

	patchPath := filepath.Join(patchesDir, "a.cc")
	writeFile(t, patchPath, "diff --git a/chrome/browser/a.cc b/chrome/browser/a.cc\n")
	gitRun(t, patchesRepo, "add", ".")
	gitRun(t, patchesRepo, "commit", "-m", "add patch")
	revBeforeDelete := strings.TrimSpace(gitRun(t, patchesRepo, "rev-parse", "HEAD"))

	if err := os.Remove(patchPath); err != nil {
		t.Fatalf("remove patch file: %v", err)
	}
	gitRun(t, patchesRepo, "add", "-A")
	gitRun(t, patchesRepo, "commit", "-m", "delete patch")

	ctx := &config.Context{
		ChromiumDir: chromiumDir,
		PatchesRepo: patchesRepo,
		PatchesDir:  filepath.Join(patchesRepo, "chromium_patches"),
		BaseCommit:  baseRev,
		State: &config.State{
			LastPull: &config.SyncEvent{
				PatchesRepoRev: revBeforeDelete,
			},
		},
	}

	result, err := Pull(ctx, PullOpts{})
	if err != nil {
		t.Fatalf("Pull failed: %v", err)
	}

	got := readFile(t, target)
	if got != "base\n" {
		t.Fatalf("expected file reset to base, got %q", got)
	}
	if !contains(result.Reset, "chrome/browser/a.cc") {
		t.Fatalf("expected reset result for deleted patch, got %#v", result.Reset)
	}
}

func initGitRepo(t *testing.T, dir string) {
	t.Helper()
	gitRun(t, dir, "init")
	gitRun(t, dir, "config", "user.email", "test@example.com")
	gitRun(t, dir, "config", "user.name", "Test User")
}

func gitRun(t *testing.T, dir string, args ...string) string {
	t.Helper()
	cmd := exec.Command("git", args...)
	cmd.Dir = dir
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("git %v failed: %v\n%s", args, err, out)
	}
	return string(out)
}

func writeFile(t *testing.T, path, content string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatalf("mkdir parent: %v", err)
	}
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatalf("write file %s: %v", path, err)
	}
}

func readFile(t *testing.T, path string) string {
	t.Helper()
	b, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read file %s: %v", path, err)
	}
	return string(b)
}

func contains(values []string, target string) bool {
	for _, v := range values {
		if v == target {
			return true
		}
	}
	return false
}

func TestPullChangedPaths_EmptyRangeNoops(t *testing.T) {
	ctx := &config.Context{
		ChromiumDir: t.TempDir(),
		PatchesRepo: t.TempDir(),
		PatchesDir:  t.TempDir(),
		BaseCommit:  "deadbeef",
		State:       &config.State{},
	}

	result, err := pullChangedPaths(ctx, nil, nil, true)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(result.Applied)+len(result.Reset)+len(result.Deleted)+len(result.Conflicts) != 0 {
		t.Fatalf("expected no-op result, got %+v", fmt.Sprintf("%+v", result))
	}
}
