package patch

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

func TestParseDiffOutputDetectsRenameAndDeleteSignatures(t *testing.T) {
	renameDiff := `diff --git a/chrome/old.cc b/chrome/new.cc
similarity index 100%
rename from chrome/old.cc
rename to chrome/new.cc
`
	deleteDiff := `diff --git a/chrome/dead.cc b/chrome/dead.cc
deleted file mode 100644
index 123..000 100644
--- a/chrome/dead.cc
+++ /dev/null
@@ -1 +0,0 @@
-gone
`
	renameSet, err := ParseDiffOutput(renameDiff)
	if err != nil {
		t.Fatalf("ParseDiffOutput rename: %v", err)
	}
	deleteSet, err := ParseDiffOutput(deleteDiff)
	if err != nil {
		t.Fatalf("ParseDiffOutput delete: %v", err)
	}
	renamePatch := renameSet["chrome/new.cc"]
	if !renamePatch.IsPureRename() {
		t.Fatalf("expected pure rename patch")
	}
	if deletePatch := deleteSet["chrome/dead.cc"]; signature(deletePatch) != "delete:chrome/dead.cc" {
		t.Fatalf("unexpected delete signature: %s", signature(deletePatch))
	}
}

func TestWriteRepoPatchSetWritesMarkersAndReloads(t *testing.T) {
	patchesDir := t.TempDir()
	set := PatchSet{
		"chrome/dead.cc": {
			Path: "chrome/dead.cc",
			Op:   OpDelete,
		},
		"chrome/new.cc": {
			Path:       "chrome/new.cc",
			Op:         OpRename,
			OldPath:    "chrome/old.cc",
			Similarity: 100,
			Content: []byte(`diff --git a/chrome/old.cc b/chrome/new.cc
similarity index 100%
rename from chrome/old.cc
rename to chrome/new.cc
`),
		},
	}
	if _, _, err := WriteRepoPatchSet(patchesDir, set, nil); err != nil {
		t.Fatalf("WriteRepoPatchSet: %v", err)
	}
	if _, err := filepath.Abs(filepath.Join(patchesDir, "chrome", "dead.cc.deleted")); err != nil {
		t.Fatalf("abs: %v", err)
	}
	loaded, err := LoadRepoPatchSet(patchesDir, nil)
	if err != nil {
		t.Fatalf("LoadRepoPatchSet: %v", err)
	}
	if loaded["chrome/dead.cc"].Op != OpDelete {
		t.Fatalf("expected delete marker to round-trip")
	}
	if !loaded["chrome/new.cc"].IsPureRename() {
		t.Fatalf("expected rename marker to round-trip")
	}
}

func TestPathMatchesSkipsInternalState(t *testing.T) {
	if PathMatches(".bdev/state.yaml", nil) {
		t.Fatalf("expected internal state path to be ignored")
	}
}

func TestBuildRangePatchSetUsesLatestBaseScopedPatch(t *testing.T) {
	ctx := context.Background()
	repoDir := t.TempDir()
	runGit(t, repoDir, "init")
	runGit(t, repoDir, "config", "user.name", "Test User")
	runGit(t, repoDir, "config", "user.email", "test@example.com")

	writeRepoFile(t, filepath.Join(repoDir, "chrome", "foo.txt"), "one\n")
	runGit(t, repoDir, "add", "chrome/foo.txt")
	runGit(t, repoDir, "commit", "-m", "base")
	base := gitOutput(t, repoDir, "rev-parse", "HEAD")

	writeRepoFile(t, filepath.Join(repoDir, "chrome", "foo.txt"), "two\n")
	runGit(t, repoDir, "commit", "-am", "step one")
	writeRepoFile(t, filepath.Join(repoDir, "chrome", "foo.txt"), "three\n")
	runGit(t, repoDir, "commit", "-am", "step two")
	end := gitOutput(t, repoDir, "rev-parse", "HEAD")

	set, err := BuildRangePatchSet(ctx, repoDir, base, end, base, false, nil)
	if err != nil {
		t.Fatalf("BuildRangePatchSet: %v", err)
	}
	content := string(set["chrome/foo.txt"].Content)
	if !strings.Contains(content, "+three") {
		t.Fatalf("expected final patch content, got %q", content)
	}
	if strings.Contains(content, "+two") {
		t.Fatalf("expected latest base-scoped patch, got %q", content)
	}
}

func TestSignatureForBinaryPatchWithContent(t *testing.T) {
	// Create a binary patch with content
	p := FilePatch{
		Path:     "chrome/binary.bin",
		Op:       OpBinary,
		IsBinary: true,
		Content:  []byte("GIT binary patch\nliteral 5\nabcde\n"),
	}
	
	sig := signature(p)
	
	// The signature should handle binary patches with content
	// With our fix, it should always use "binary:" prefix for OpBinary
	if !strings.HasPrefix(sig, "binary:") {
		t.Errorf("Expected signature for binary patch with content to start with 'binary:', got %q", sig)
	}
	// Check the path is included
	expectedPath := "binary:chrome/binary.bin"
	if sig != expectedPath {
		t.Errorf("Expected signature %q, got %q", expectedPath, sig)
	}
}

func TestSignatureForBinaryPatchWithoutContent(t *testing.T) {
	// Create a binary patch without content
	p := FilePatch{
		Path:     "chrome/empty.bin",
		Op:       OpBinary,
		IsBinary: true,
		Content:  []byte{},
	}
	
	sig := signature(p)
	
	// Should still use "binary:" prefix
	expectedPath := "binary:chrome/empty.bin"
	if sig != expectedPath {
		t.Errorf("Expected signature %q, got %q", expectedPath, sig)
	}
}

func runGit(t *testing.T, dir string, args ...string) {
	t.Helper()
	cmd := exec.Command("git", args...)
	cmd.Dir = dir
	output, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("git %s: %v\n%s", strings.Join(args, " "), err, string(output))
	}
}

func gitOutput(t *testing.T, dir string, args ...string) string {
	t.Helper()
	cmd := exec.Command("git", args...)
	cmd.Dir = dir
	output, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("git %s: %v\n%s", strings.Join(args, " "), err, string(output))
	}
	return strings.TrimSpace(string(output))
}

func writeRepoFile(t *testing.T, path string, body string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(path, []byte(body), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}
}
