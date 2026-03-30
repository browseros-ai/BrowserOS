package workspace

import (
	"fmt"
	"os"
	"path/filepath"
	"time"

	"gopkg.in/yaml.v3"
)

type State struct {
	Version        int       `yaml:"version" json:"version"`
	Workspace      string    `yaml:"workspace,omitempty" json:"workspace,omitempty"`
	BaseCommit     string    `yaml:"base_commit,omitempty" json:"base_commit,omitempty"`
	LastApplyRev   string    `yaml:"last_apply_rev,omitempty" json:"last_apply_rev,omitempty"`
	LastSyncRev    string    `yaml:"last_sync_rev,omitempty" json:"last_sync_rev,omitempty"`
	LastExtractRev string    `yaml:"last_extract_rev,omitempty" json:"last_extract_rev,omitempty"`
	PendingStash   string    `yaml:"pending_stash,omitempty" json:"pending_stash,omitempty"`
	LastApplyAt    time.Time `yaml:"last_apply_at,omitempty" json:"last_apply_at,omitempty"`
	LastSyncAt     time.Time `yaml:"last_sync_at,omitempty" json:"last_sync_at,omitempty"`
	LastExtractAt  time.Time `yaml:"last_extract_at,omitempty" json:"last_extract_at,omitempty"`
}

func StateDir(workspacePath string) string {
	return filepath.Join(workspacePath, ".bdev")
}

func StatePath(workspacePath string) string {
	return filepath.Join(StateDir(workspacePath), "state.yaml")
}

func LoadState(workspacePath string) (*State, error) {
	data, err := os.ReadFile(StatePath(workspacePath))
	if err != nil {
		if os.IsNotExist(err) {
			return &State{Version: 1, Workspace: workspacePath}, nil
		}
		return nil, err
	}
	var state State
	if err := yaml.Unmarshal(data, &state); err != nil {
		return nil, fmt.Errorf("parse state: %w", err)
	}
	if state.Version == 0 {
		state.Version = 1
	}
	if state.Workspace == "" {
		state.Workspace = workspacePath
	}
	return &state, nil
}

func SaveState(workspacePath string, state *State) error {
	if state.Version == 0 {
		state.Version = 1
	}
	if state.Workspace == "" {
		state.Workspace = workspacePath
	}
	if err := os.MkdirAll(StateDir(workspacePath), 0o755); err != nil {
		return err
	}
	body, err := yaml.Marshal(state)
	if err != nil {
		return err
	}
	header := "# bdev workspace state\n\n"
	return os.WriteFile(StatePath(workspacePath), append([]byte(header), body...), 0o644)
}
