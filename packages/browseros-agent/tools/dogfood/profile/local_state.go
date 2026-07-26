package profile

import (
	"encoding/json"
	"os"
	"path/filepath"
	"sort"
)

type BrowserProfile struct {
	Dir   string
	Name  string
	Email string
}

type localState struct {
	Profile struct {
		InfoCache map[string]struct {
			Name     string `json:"name"`
			UserName string `json:"user_name"`
		} `json:"info_cache"`
	} `json:"profile"`
}

func ReadProfiles(userDataDir string) ([]BrowserProfile, error) {
	data, err := os.ReadFile(filepath.Join(userDataDir, "Local State"))
	if err != nil {
		return []BrowserProfile{{Dir: "Default", Name: "Default"}}, nil
	}
	var state localState
	if err := json.Unmarshal(data, &state); err != nil {
		return []BrowserProfile{{Dir: "Default", Name: "Default"}}, nil
	}
	if len(state.Profile.InfoCache) == 0 {
		return []BrowserProfile{{Dir: "Default", Name: "Default"}}, nil
	}
	profiles := make([]BrowserProfile, 0, len(state.Profile.InfoCache))
	for dir, meta := range state.Profile.InfoCache {
		name := meta.Name
		if name == "" {
			name = dir
		}
		profiles = append(profiles, BrowserProfile{
			Dir:   dir,
			Name:  name,
			Email: meta.UserName,
		})
	}
	sort.Slice(profiles, func(i, j int) bool {
		return profiles[i].Dir < profiles[j].Dir
	})
	return profiles, nil
}
