package resolve

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"

	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/patch"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/workspace"
)

type Operation struct {
	ChromiumPath string       `json:"chromium_path"`
	PatchRel     string       `json:"patch_rel"`
	Op           patch.FileOp `json:"op"`
	OldPath      string       `json:"old_path,omitempty"`
	RejectPath   string       `json:"reject_path,omitempty"`
	Message      string       `json:"message,omitempty"`
}

type State struct {
	Workspace  string      `json:"workspace"`
	RepoRoot   string      `json:"repo_root"`
	BaseCommit string      `json:"base_commit"`
	RepoRev    string      `json:"repo_rev,omitempty"`
	Mode       string      `json:"mode,omitempty"`
	Current    int         `json:"current"`
	Operations []Operation `json:"operations"`
	Resolved   []string    `json:"resolved,omitempty"`
	Skipped    []string    `json:"skipped,omitempty"`
}

func Path(workspacePath string) string {
	return filepath.Join(workspace.StateDir(workspacePath), "resolve.json")
}

func Exists(workspacePath string) bool {
	_, err := os.Stat(Path(workspacePath))
	return err == nil
}

func Load(workspacePath string) (*State, error) {
	data, err := os.ReadFile(Path(workspacePath))
	if err != nil {
		return nil, err
	}
	var state State
	if err := json.Unmarshal(data, &state); err != nil {
		return nil, err
	}
	return &state, nil
}

func Save(workspacePath string, state *State) error {
	if err := os.MkdirAll(workspace.StateDir(workspacePath), 0o755); err != nil {
		return err
	}
	body, err := json.MarshalIndent(state, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(Path(workspacePath), append(body, '\n'), 0o644)
}

func Delete(workspacePath string) error {
	if err := os.Remove(Path(workspacePath)); err != nil && !os.IsNotExist(err) {
		return err
	}
	return nil
}

func FindActive(reg *workspace.Registry, cwd string) (workspace.Entry, error) {
	if ws, err := workspace.Detect(reg, cwd); err == nil && Exists(ws.Path) {
		return ws, nil
	}
	var active []workspace.Entry
	for _, ws := range reg.Workspaces {
		if Exists(ws.Path) {
			active = append(active, ws)
		}
	}
	switch len(active) {
	case 0:
		return workspace.Entry{}, fmt.Errorf(`no active conflict resolution found; run "bdev apply" or "bdev sync --rebase" first`)
	case 1:
		return active[0], nil
	default:
		return workspace.Entry{}, fmt.Errorf("multiple workspaces have active conflicts; run from inside the target workspace")
	}
}

func (s *State) CurrentOperation() (Operation, error) {
	if s.Current < 0 || s.Current >= len(s.Operations) {
		return Operation{}, fmt.Errorf("no active conflict remaining")
	}
	return s.Operations[s.Current], nil
}
