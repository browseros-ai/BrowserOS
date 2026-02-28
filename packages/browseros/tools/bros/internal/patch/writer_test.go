package patch

import (
	"os"
	"path/filepath"
	"testing"
)

func TestWritePatchSetDeletedCleansSiblingArtifacts(t *testing.T) {
	t.Parallel()

	dir := t.TempDir()
	chromPath := "components/foo/bar.cc"
	basePath := filepath.Join(dir, chromPath)

	if err := os.MkdirAll(filepath.Dir(basePath), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}

	preExisting := []string{
		basePath,
		basePath + ".binary",
		basePath + ".rename",
	}
	for _, p := range preExisting {
		if err := os.WriteFile(p, []byte("old"), 0o644); err != nil {
			t.Fatalf("write %s: %v", p, err)
		}
	}

	ps := NewPatchSet("")
	ps.Patches[chromPath] = &FilePatch{Path: chromPath, Op: OpDeleted}
	if err := WritePatchSet(dir, ps, false); err != nil {
		t.Fatalf("WritePatchSet: %v", err)
	}

	if _, err := os.Stat(basePath + ".deleted"); err != nil {
		t.Fatalf("expected deleted marker: %v", err)
	}
	for _, stale := range preExisting {
		if _, err := os.Stat(stale); !os.IsNotExist(err) {
			t.Fatalf("expected stale artifact removed: %s", stale)
		}
	}
}

func TestWritePatchSetRenameKeepsPatchAndRenameMarker(t *testing.T) {
	t.Parallel()

	dir := t.TempDir()
	chromPath := "content/new_file.cc"
	basePath := filepath.Join(dir, chromPath)
	if err := os.MkdirAll(filepath.Dir(basePath), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}

	if err := os.WriteFile(basePath+".deleted", []byte("stale"), 0o644); err != nil {
		t.Fatalf("seed marker: %v", err)
	}
	if err := os.WriteFile(basePath+".binary", []byte("stale"), 0o644); err != nil {
		t.Fatalf("seed marker: %v", err)
	}

	ps := NewPatchSet("")
	ps.Patches[chromPath] = &FilePatch{
		Path:       chromPath,
		Op:         OpRenamed,
		OldPath:    "content/old_file.cc",
		Similarity: 95,
		Content:    []byte("diff --git a/content/old_file.cc b/content/new_file.cc\n"),
	}
	if err := WritePatchSet(dir, ps, false); err != nil {
		t.Fatalf("WritePatchSet: %v", err)
	}

	if _, err := os.Stat(basePath); err != nil {
		t.Fatalf("expected rename patch content file: %v", err)
	}
	if _, err := os.Stat(basePath + ".rename"); err != nil {
		t.Fatalf("expected rename marker file: %v", err)
	}
	for _, stale := range []string{basePath + ".deleted", basePath + ".binary"} {
		if _, err := os.Stat(stale); !os.IsNotExist(err) {
			t.Fatalf("expected stale artifact removed: %s", stale)
		}
	}
}
