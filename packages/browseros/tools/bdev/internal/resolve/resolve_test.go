package resolve

import (
	"testing"

	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/patch"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/workspace"
)

func TestStateRoundTrip(t *testing.T) {
	workspacePath := t.TempDir()
	state := &State{
		Workspace:  workspacePath,
		RepoRoot:   "/tmp/repo",
		BaseCommit: "abc123",
		Current:    1,
		Operations: []Operation{{
			ChromiumPath: "chrome/browser/foo.cc",
			PatchRel:     "chrome/browser/foo.cc",
			Op:           patch.OpModify,
		}},
	}
	if err := Save(workspacePath, state); err != nil {
		t.Fatalf("Save: %v", err)
	}
	loaded, err := Load(workspacePath)
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	if loaded.BaseCommit != state.BaseCommit || loaded.Current != state.Current {
		t.Fatalf("state mismatch: %#v", loaded)
	}
	if err := Delete(workspacePath); err != nil {
		t.Fatalf("Delete: %v", err)
	}
	if Exists(workspacePath) {
		t.Fatalf("expected resolve state to be deleted")
	}
}

func TestFindActivePrefersCurrentWorkspace(t *testing.T) {
	root := t.TempDir()
	workspaceA := workspace.Entry{Name: "a", Path: root + "/a"}
	workspaceB := workspace.Entry{Name: "b", Path: root + "/b"}
	for _, ws := range []workspace.Entry{workspaceA, workspaceB} {
		if err := Save(ws.Path, &State{Workspace: ws.Path}); err != nil {
			t.Fatalf("save %s: %v", ws.Name, err)
		}
	}

	reg := &workspace.Registry{Workspaces: []workspace.Entry{workspaceA, workspaceB}}
	active, err := FindActive(reg, workspaceB.Path)
	if err != nil {
		t.Fatalf("FindActive: %v", err)
	}
	if active.Name != workspaceB.Name {
		t.Fatalf("expected %q, got %q", workspaceB.Name, active.Name)
	}
}
