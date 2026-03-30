package repo

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

type Info struct {
	Root       string `json:"root"`
	PatchesDir string `json:"patches_dir"`
	BaseCommit string `json:"base_commit"`
}

func Discover(start string) (string, error) {
	abs, err := filepath.Abs(start)
	if err != nil {
		return "", err
	}
	current := filepath.Clean(abs)
	for {
		if hasRepoMarkers(current) {
			return current, nil
		}
		next := filepath.Dir(current)
		if next == current {
			break
		}
		current = next
	}
	return "", fmt.Errorf("unable to find patches repo from %s", start)
}

func Load(root string) (*Info, error) {
	abs, err := filepath.Abs(root)
	if err != nil {
		return nil, err
	}
	clean := filepath.Clean(abs)
	if !hasRepoMarkers(clean) {
		return nil, fmt.Errorf("not a browseros patches repo: %s", clean)
	}
	base, err := os.ReadFile(filepath.Join(clean, "BASE_COMMIT"))
	if err != nil {
		return nil, err
	}
	return &Info{
		Root:       clean,
		PatchesDir: filepath.Join(clean, "chromium_patches"),
		BaseCommit: strings.TrimSpace(string(base)),
	}, nil
}

func hasRepoMarkers(root string) bool {
	if _, err := os.Stat(filepath.Join(root, "BASE_COMMIT")); err != nil {
		return false
	}
	info, err := os.Stat(filepath.Join(root, "chromium_patches"))
	return err == nil && info.IsDir()
}
