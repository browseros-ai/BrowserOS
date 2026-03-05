package git

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

func TestRemoteSyncAndStatusHelpers(t *testing.T) {
	t.Parallel()

	root := t.TempDir()
	remote := filepath.Join(root, "remote.git")
	repoA := filepath.Join(root, "repo-a")
	repoB := filepath.Join(root, "repo-b")

	runGit(t, root, "init", "--bare", remote)
	runGit(t, root, "clone", remote, repoA)
	configRepo(t, repoA)
	writeFile(t, filepath.Join(repoA, "README.md"), "one\n")
	runGit(t, repoA, "add", "README.md")
	runGit(t, repoA, "commit", "-m", "init")

	branch := strings.TrimSpace(runGit(t, repoA, "symbolic-ref", "--short", "HEAD"))
	runGit(t, repoA, "push", "-u", "origin", branch)
	runGit(t, root, "clone", remote, repoB)
	configRepo(t, repoB)

	remotes, err := ListRemotes(repoB)
	if err != nil {
		t.Fatalf("ListRemotes: %v", err)
	}
	if len(remotes) != 1 || remotes[0] != "origin" {
		t.Fatalf("unexpected remotes: %#v", remotes)
	}

	writeFile(t, filepath.Join(repoA, "README.md"), "two\n")
	runGit(t, repoA, "commit", "-am", "update")
	runGit(t, repoA, "push", "origin", branch)
	targetRev := strings.TrimSpace(runGit(t, repoA, "rev-parse", "HEAD"))

	if err := Fetch(repoB, "origin"); err != nil {
		t.Fatalf("Fetch: %v", err)
	}
	curBranch, detached, err := CurrentBranch(repoB)
	if err != nil {
		t.Fatalf("CurrentBranch: %v", err)
	}
	if detached {
		t.Fatalf("expected checked-out branch in clone")
	}
	if err := Pull(repoB, "origin", curBranch, true); err != nil {
		t.Fatalf("Pull: %v", err)
	}

	currentRev, err := HeadRev(repoB)
	if err != nil {
		t.Fatalf("HeadRev: %v", err)
	}
	if currentRev != targetRev {
		t.Fatalf("repo-b did not fast-forward to latest rev: got %s want %s", currentRev, targetRev)
	}

	writeFile(t, filepath.Join(repoB, "scratch.txt"), "dirty\n")
	dirty, err := IsDirty(repoB)
	if err != nil {
		t.Fatalf("IsDirty: %v", err)
	}
	if !dirty {
		t.Fatalf("expected repo to be dirty")
	}
}

func configRepo(t *testing.T, dir string) {
	t.Helper()
	runGit(t, dir, "config", "user.email", "bdev-test@example.com")
	runGit(t, dir, "config", "user.name", "bdev test")
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

func writeFile(t *testing.T, path, content string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}
}
