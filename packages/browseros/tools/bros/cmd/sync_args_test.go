package cmd

import (
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"testing"
)

func TestResolveRemoteAndFiles(t *testing.T) {
	t.Parallel()

	repo := initRemoteRepo(t)

	remote, files, err := resolveRemoteAndFiles(repo, []string{"origin", "content/foo.cc"}, "")
	if err != nil {
		t.Fatalf("resolveRemoteAndFiles: %v", err)
	}
	if remote != "origin" {
		t.Fatalf("expected origin, got %q", remote)
	}
	if !reflect.DeepEqual(files, []string{"content/foo.cc"}) {
		t.Fatalf("unexpected files: %#v", files)
	}
}

func TestResolveRemoteAndFilesUnknownExplicitRemote(t *testing.T) {
	t.Parallel()

	repo := initRemoteRepo(t)
	if _, _, err := resolveRemoteAndFiles(repo, nil, "missing"); err == nil {
		t.Fatalf("expected error for unknown explicit remote")
	}
}

func initRemoteRepo(t *testing.T) string {
	t.Helper()

	dir := filepath.Join(t.TempDir(), "patches")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}

	runGitCmd(t, dir, "init")
	runGitCmd(t, dir, "remote", "add", "origin", "https://example.com/org/repo.git")
	return dir
}

func runGitCmd(t *testing.T, dir string, args ...string) string {
	t.Helper()
	cmd := exec.Command("git", args...)
	cmd.Dir = dir
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("git %v failed: %v\n%s", args, err, string(out))
	}
	return string(out)
}
