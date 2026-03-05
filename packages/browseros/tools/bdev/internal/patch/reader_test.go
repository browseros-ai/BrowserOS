package patch

import (
	"os"
	"path/filepath"
	"testing"
)

func TestReadPatchSetMarkerPrecedence(t *testing.T) {
	t.Parallel()

	dir := t.TempDir()
	patchPath := filepath.Join(dir, "chrome", "browser", "foo.cc")
	if err := os.MkdirAll(filepath.Dir(patchPath), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}

	if err := os.WriteFile(patchPath, []byte("diff --git a/chrome/browser/foo.cc b/chrome/browser/foo.cc\n"), 0o644); err != nil {
		t.Fatalf("write patch: %v", err)
	}
	if err := os.WriteFile(patchPath+".deleted", []byte("deleted: chrome/browser/foo.cc\n"), 0o644); err != nil {
		t.Fatalf("write marker: %v", err)
	}

	ps, err := ReadPatchSet(dir)
	if err != nil {
		t.Fatalf("ReadPatchSet: %v", err)
	}

	fp, ok := ps.Patches["chrome/browser/foo.cc"]
	if !ok {
		t.Fatalf("missing patch entry")
	}
	if fp.Op != OpDeleted {
		t.Fatalf("expected OpDeleted, got %v", fp.Op)
	}
	if len(fp.Content) != 0 {
		t.Fatalf("expected empty content for deleted marker")
	}
}

func TestReadPatchSetRenameMergesContent(t *testing.T) {
	t.Parallel()

	dir := t.TempDir()
	patchPath := filepath.Join(dir, "chrome", "browser", "new_name.cc")
	if err := os.MkdirAll(filepath.Dir(patchPath), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}

	diff := "diff --git a/chrome/browser/old_name.cc b/chrome/browser/new_name.cc\nrename from chrome/browser/old_name.cc\nrename to chrome/browser/new_name.cc\n"
	if err := os.WriteFile(patchPath, []byte(diff), 0o644); err != nil {
		t.Fatalf("write patch: %v", err)
	}
	if err := os.WriteFile(patchPath+".rename", []byte("rename_from: chrome/browser/old_name.cc\nsimilarity: 92\n"), 0o644); err != nil {
		t.Fatalf("write rename marker: %v", err)
	}

	ps, err := ReadPatchSet(dir)
	if err != nil {
		t.Fatalf("ReadPatchSet: %v", err)
	}

	fp, ok := ps.Patches["chrome/browser/new_name.cc"]
	if !ok {
		t.Fatalf("missing rename patch entry")
	}
	if fp.Op != OpRenamed {
		t.Fatalf("expected OpRenamed, got %v", fp.Op)
	}
	if fp.OldPath != "chrome/browser/old_name.cc" {
		t.Fatalf("unexpected old path: %q", fp.OldPath)
	}
	if len(fp.Content) == 0 {
		t.Fatalf("expected rename patch to keep diff content")
	}
}
