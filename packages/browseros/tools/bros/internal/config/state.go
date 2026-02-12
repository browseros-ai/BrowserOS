package config

import (
	"fmt"
	"os"
	"path/filepath"
	"time"

	"gopkg.in/yaml.v3"
)

type State struct {
	LastPull *SyncEvent `yaml:"last_pull,omitempty"`
	LastPush *SyncEvent `yaml:"last_push,omitempty"`
}

type SyncEvent struct {
	PatchesRepoRev string    `yaml:"patches_repo_rev"`
	Timestamp      time.Time `yaml:"timestamp"`
	FileCount      int       `yaml:"file_count"`
}

func ReadState(brosDir string) (*State, error) {
	path := filepath.Join(brosDir, "state.yaml")
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return &State{}, nil
		}
		return nil, fmt.Errorf("reading state: %w", err)
	}
	var s State
	if err := yaml.Unmarshal(data, &s); err != nil {
		return nil, fmt.Errorf("parsing state.yaml: %w", err)
	}
	return &s, nil
}

func WriteState(brosDir string, s *State) error {
	data, err := yaml.Marshal(s)
	if err != nil {
		return fmt.Errorf("marshaling state: %w", err)
	}
	path := filepath.Join(brosDir, "state.yaml")
	if err := os.WriteFile(path, data, 0o644); err != nil {
		return fmt.Errorf("writing state.yaml: %w", err)
	}
	return nil
}
