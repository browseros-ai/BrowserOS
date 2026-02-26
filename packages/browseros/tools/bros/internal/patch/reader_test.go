package patch

import (
	"os"
	"path/filepath"
	"testing"
)

func TestReadPatchSet_MergesRenameMarkerWithPatchContent(t *testing.T) {
	dir := t.TempDir()

	patchPath := filepath.Join(dir, "chrome", "browser", "new_file.cc")
	if err := os.MkdirAll(filepath.Dir(patchPath), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}

	patchContent := "" +
		"diff --git a/chrome/browser/new_file.cc b/chrome/browser/new_file.cc\n" +
		"index 1111111..2222222 100644\n" +
		"--- a/chrome/browser/new_file.cc\n" +
		"+++ b/chrome/browser/new_file.cc\n" +
		"@@ -1 +1 @@\n" +
		"-old\n" +
		"+new\n"
	if err := os.WriteFile(patchPath, []byte(patchContent), 0o644); err != nil {
		t.Fatalf("write patch: %v", err)
	}

	renameMarkerPath := patchPath + ".rename"
	renameContent := "rename_from: chrome/browser/old_file.cc\nsimilarity: 89\n"
	if err := os.WriteFile(renameMarkerPath, []byte(renameContent), 0o644); err != nil {
		t.Fatalf("write marker: %v", err)
	}

	ps, err := ReadPatchSet(dir)
	if err != nil {
		t.Fatalf("ReadPatchSet: %v", err)
	}

	fp, ok := ps.Patches["chrome/browser/new_file.cc"]
	if !ok {
		t.Fatalf("missing merged patch entry")
	}
	if fp.Op != OpRenamed {
		t.Fatalf("expected op %v, got %v", OpRenamed, fp.Op)
	}
	if fp.OldPath != "chrome/browser/old_file.cc" {
		t.Fatalf("unexpected old path: %q", fp.OldPath)
	}
	if fp.Similarity != 89 {
		t.Fatalf("unexpected similarity: %d", fp.Similarity)
	}
	if len(fp.Content) == 0 {
		t.Fatalf("expected patch content to be preserved")
	}
}

func TestReadPatchSet_DeletedMarkerWins(t *testing.T) {
	dir := t.TempDir()

	patchPath := filepath.Join(dir, "chrome", "browser", "dead_file.cc")
	if err := os.MkdirAll(filepath.Dir(patchPath), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}

	if err := os.WriteFile(patchPath, []byte("diff --git a/x b/x\n"), 0o644); err != nil {
		t.Fatalf("write patch: %v", err)
	}
	if err := os.WriteFile(patchPath+".deleted", []byte("deleted: chrome/browser/dead_file.cc\n"), 0o644); err != nil {
		t.Fatalf("write marker: %v", err)
	}

	ps, err := ReadPatchSet(dir)
	if err != nil {
		t.Fatalf("ReadPatchSet: %v", err)
	}

	fp, ok := ps.Patches["chrome/browser/dead_file.cc"]
	if !ok {
		t.Fatalf("missing deleted patch entry")
	}
	if fp.Op != OpDeleted {
		t.Fatalf("expected op %v, got %v", OpDeleted, fp.Op)
	}
	if fp.Content != nil {
		t.Fatalf("expected nil content for deleted entry")
	}
}
