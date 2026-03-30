package update

import (
	"encoding/json"
	"os"
	"path/filepath"
	"time"

	"browseros-cli/config"
)

type State struct {
	LastCheckedAt     time.Time `json:"last_checked_at"`
	LatestVersion     string    `json:"latest_version,omitempty"`
	LatestPublishedAt string    `json:"latest_published_at,omitempty"`
	AssetURL          string    `json:"asset_url,omitempty"`
	CheckError        string    `json:"check_error,omitempty"`
}

func StatePath() string {
	return filepath.Join(config.Dir(), "update-state.json")
}

func LoadState() (*State, error) {
	data, err := os.ReadFile(StatePath())
	if err != nil {
		if os.IsNotExist(err) {
			return &State{}, nil
		}
		return nil, err
	}

	var state State
	if err := json.Unmarshal(data, &state); err != nil {
		return nil, err
	}

	return &state, nil
}

func SaveState(state *State) error {
	if state == nil {
		state = &State{}
	}

	dir := config.Dir()
	if err := os.MkdirAll(dir, 0755); err != nil {
		return err
	}

	tmpFile, err := os.CreateTemp(dir, "update-state-*.json")
	if err != nil {
		return err
	}

	encoder := json.NewEncoder(tmpFile)
	encoder.SetIndent("", "  ")
	if err := encoder.Encode(state); err != nil {
		tmpFile.Close()
		os.Remove(tmpFile.Name())
		return err
	}
	if err := tmpFile.Close(); err != nil {
		os.Remove(tmpFile.Name())
		return err
	}
	if err := os.Rename(tmpFile.Name(), StatePath()); err != nil {
		os.Remove(tmpFile.Name())
		return err
	}

	return nil
}

func (s *State) IsStale(now time.Time, ttl time.Duration) bool {
	if s == nil || s.LastCheckedAt.IsZero() {
		return true
	}
	return now.Sub(s.LastCheckedAt) >= ttl
}
