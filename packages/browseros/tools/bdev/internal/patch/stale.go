package patch

import (
	"os"
	"path/filepath"
	"strings"
)

// RemoveStale walks chromium_patches/ and removes patches for files
// that are NOT in the given PatchSet.
func RemoveStale(patchesDir string, current *PatchSet, dryRun bool) ([]string, error) {
	var stale []string

	err := filepath.Walk(patchesDir, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return nil
		}
		if info.IsDir() {
			return nil
		}

		rel, err := filepath.Rel(patchesDir, path)
		if err != nil {
			return nil
		}

		// Normalize: strip marker suffixes to get the chromium path
		chromPath := rel
		chromPath = strings.TrimSuffix(chromPath, ".deleted")
		chromPath = strings.TrimSuffix(chromPath, ".binary")
		chromPath = strings.TrimSuffix(chromPath, ".rename")

		if _, exists := current.Patches[chromPath]; !exists {
			stale = append(stale, rel)
			if !dryRun {
				_ = os.Remove(path)
			}
		}

		return nil
	})

	if err != nil {
		return stale, err
	}

	// Clean up empty directories
	if !dryRun {
		cleanEmptyDirs(patchesDir)
	}

	return stale, nil
}

func cleanEmptyDirs(root string) {
	// Walk bottom-up by collecting dirs first then removing empty ones
	var dirs []string
	_ = filepath.Walk(root, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return nil
		}
		if info.IsDir() && path != root {
			dirs = append(dirs, path)
		}
		return nil
	})

	// Reverse order (deepest first)
	for i := len(dirs) - 1; i >= 0; i-- {
		entries, err := os.ReadDir(dirs[i])
		if err == nil && len(entries) == 0 {
			_ = os.Remove(dirs[i])
		}
	}
}
