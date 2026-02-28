package e2e

import (
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

type scenario struct {
	root       string
	baseCommit string

	patchesRemote string
	patchesRepo   string
	chromiumA     string
	chromiumB     string

	trackedPath string
	newPath     string
}

type statusJSON struct {
	Ahead  int
	Behind int
	Synced int
}

func TestBrosOperationsE2E(t *testing.T) {
	env := setupScenario(t)

	runBros(t, env.chromiumA, "init", "--patches-repo", env.patchesRepo, "--name", "checkout-a")

	statusBefore := readStatus(t, env.chromiumA)
	if statusBefore.Behind == 0 {
		t.Fatalf("expected checkout-a to be behind before pull, got %#v", statusBefore)
	}

	pullPreview := runBros(t, env.chromiumA, "diff", "--direction", "pull")
	assertContains(t, pullPreview, env.trackedPath)

	runBros(t, env.chromiumA, "pull", "--no-sync")
	assertFileContains(t, filepath.Join(env.chromiumA, env.trackedPath), "patch-v1")

	statusAfterPull := readStatus(t, env.chromiumA)
	if statusAfterPull.Behind != 0 || statusAfterPull.Synced == 0 {
		t.Fatalf("unexpected status after pull: %#v", statusAfterPull)
	}

	writeFile(t, filepath.Join(env.chromiumA, "base", ".keep"), "my-local-work\n")
	pullAgain := runBros(t, env.chromiumA, "pull", "--no-sync", "base/.keep")
	assertContains(t, pullAgain, "local-only, kept")
	assertFileContains(t, filepath.Join(env.chromiumA, "base", ".keep"), "my-local-work")
	runGit(t, env.chromiumA, "checkout", env.baseCommit, "--", "base/.keep")

	writeFile(t, filepath.Join(env.chromiumA, env.trackedPath), "patch-v2\n")
	writeFile(t, filepath.Join(env.chromiumA, env.newPath), "brand-new\n")
	pushPreview := runBros(t, env.chromiumA, "diff", "--direction", "push")
	assertContains(t, pushPreview, env.trackedPath)

	runBros(t, env.chromiumA, "push", "--no-sync", env.trackedPath, env.newPath)
	assertFileContains(t, filepath.Join(env.patchesRepo, "chromium_patches", env.newPath), "diff --git")

	// Keep the patches repo clean before remote-aware publish flow.
	commitRepo(t, env.patchesRepo, "chore: e2e checkpoint after push --no-sync")

	writeFile(t, filepath.Join(env.chromiumA, env.trackedPath), "patch-v3\n")
	publish := runBros(t, env.chromiumA, "push", "origin", "-m", "e2e: publish patch-v3", env.trackedPath)
	assertContains(t, publish, "remote publish complete")

	mirror := filepath.Join(env.root, "mirror")
	runGit(t, env.root, "clone", env.patchesRemote, mirror)
	assertFileContains(t, filepath.Join(mirror, "chromium_patches", env.trackedPath), "patch-v3")

	collab := filepath.Join(env.root, "collab")
	runGit(t, env.root, "clone", env.patchesRemote, collab)
	configRepo(t, collab)
	diffV4 := buildDiffFromBase(t, env.chromiumA, env.baseCommit, env.trackedPath, "patch-v4\n")
	writeFile(t, filepath.Join(collab, "chromium_patches", env.trackedPath), diffV4)
	commitRepo(t, collab, "feat: remote patch-v4 update")
	branch := strings.TrimSpace(runGit(t, collab, "symbolic-ref", "--short", "HEAD"))
	runGit(t, collab, "push", "origin", "HEAD:"+branch)

	runBros(t, env.chromiumA, "pull", "origin")
	assertFileContains(t, filepath.Join(env.chromiumA, env.trackedPath), "patch-v4")

	runBros(t, env.chromiumB, "clone", "--patches-repo", env.patchesRepo, "--verify-base", "--clean", "--name", "checkout-b")
	assertFileContains(t, filepath.Join(env.chromiumB, env.trackedPath), "patch-v4")
	statusB := readStatus(t, env.chromiumB)
	if statusB.Ahead != 0 || statusB.Synced == 0 {
		t.Fatalf("expected checkout-b to have clean/synced clone state, got %#v", statusB)
	}
}

func setupScenario(t *testing.T) *scenario {
	t.Helper()

	root := t.TempDir()
	patchesRemote := filepath.Join(root, "patches-remote.git")
	chromiumA := filepath.Join(root, "chromium-a")
	chromiumB := filepath.Join(root, "chromium-b")
	patchesRepo := filepath.Join(root, "patches")
	trackedPath := filepath.ToSlash(filepath.Join("chrome", "app", "test.txt"))
	newPath := filepath.ToSlash(filepath.Join("chrome", "browser", "new_file.txt"))

	runGit(t, root, "init", "--bare", patchesRemote)
	setupChromiumRepo(t, chromiumA)

	writeFile(t, filepath.Join(chromiumA, trackedPath), "base\n")
	runGit(t, chromiumA, "add", "-A")
	runGit(t, chromiumA, "commit", "-m", "base")
	baseCommit := strings.TrimSpace(runGit(t, chromiumA, "rev-parse", "HEAD"))

	diffV1 := buildDiffFromBase(t, chromiumA, baseCommit, trackedPath, "patch-v1\n")

	runGit(t, root, "clone", patchesRemote, patchesRepo)
	configRepo(t, patchesRepo)
	writeFile(t, filepath.Join(patchesRepo, "BASE_COMMIT"), baseCommit+"\n")
	writeFile(t, filepath.Join(patchesRepo, "CHROMIUM_VERSION"), "MAJOR=145\nMINOR=0\nBUILD=7632\nPATCH=45\n")
	writeFile(t, filepath.Join(patchesRepo, "chromium_patches", trackedPath), diffV1)
	commitRepo(t, patchesRepo, "seed patches")
	branch := strings.TrimSpace(runGit(t, patchesRepo, "symbolic-ref", "--short", "HEAD"))
	runGit(t, patchesRepo, "push", "-u", "origin", "HEAD:"+branch)

	runGit(t, root, "clone", chromiumA, chromiumB)
	configRepo(t, chromiumB)

	return &scenario{
		root:          root,
		baseCommit:    baseCommit,
		patchesRemote: patchesRemote,
		patchesRepo:   patchesRepo,
		chromiumA:     chromiumA,
		chromiumB:     chromiumB,
		trackedPath:   trackedPath,
		newPath:       newPath,
	}
}

func setupChromiumRepo(t *testing.T, dir string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Join(dir, "chrome"), 0o755); err != nil {
		t.Fatalf("mkdir chrome: %v", err)
	}
	if err := os.MkdirAll(filepath.Join(dir, "base"), 0o755); err != nil {
		t.Fatalf("mkdir base: %v", err)
	}
	writeFile(t, filepath.Join(dir, "base", ".keep"), "marker\n")
	runGit(t, dir, "init")
	configRepo(t, dir)
}

func buildDiffFromBase(t *testing.T, repo, base, relPath, content string) string {
	t.Helper()
	abs := filepath.Join(repo, relPath)
	original := mustRead(t, abs)
	writeFile(t, abs, content)
	diff := runGit(t, repo, "diff", "--full-index", base, "--", relPath)
	writeFile(t, abs, original)
	if strings.TrimSpace(diff) == "" {
		t.Fatalf("expected non-empty diff for %s", relPath)
	}
	return diff
}

func readStatus(t *testing.T, chromiumDir string) statusJSON {
	t.Helper()
	raw := runBros(t, chromiumDir, "status", "--json")
	var s statusJSON
	if err := json.Unmarshal([]byte(raw), &s); err != nil {
		t.Fatalf("failed to parse status json: %v\nraw=%s", err, raw)
	}
	return s
}

func runBros(t *testing.T, dir string, args ...string) string {
	t.Helper()
	cmd := exec.Command(brosBinary, args...)
	cmd.Dir = dir
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("bros %v failed: %v\n%s", args, err, string(out))
	}
	return string(out)
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

func commitRepo(t *testing.T, dir, message string) {
	t.Helper()
	runGit(t, dir, "add", "-A")
	runGit(t, dir, "commit", "-m", message)
}

func configRepo(t *testing.T, dir string) {
	t.Helper()
	runGit(t, dir, "config", "user.email", "bros-e2e@example.com")
	runGit(t, dir, "config", "user.name", "bros e2e")
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

func assertContains(t *testing.T, output, want string) {
	t.Helper()
	if !strings.Contains(output, want) {
		t.Fatalf("expected output to contain %q\noutput:\n%s", want, output)
	}
}

func assertFileContains(t *testing.T, path, want string) {
	t.Helper()
	content := mustRead(t, path)
	if !strings.Contains(content, want) {
		t.Fatalf("expected %s to contain %q\ncontent:\n%s", path, want, content)
	}
}
