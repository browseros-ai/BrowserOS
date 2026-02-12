package config

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

func ReadBaseCommit(patchesRepo string) (string, error) {
	path := filepath.Join(patchesRepo, "BASE_COMMIT")
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return "", fmt.Errorf("BASE_COMMIT not found in %s — create it with the Chromium commit hash", patchesRepo)
		}
		return "", fmt.Errorf("reading BASE_COMMIT: %w", err)
	}
	commit := strings.TrimSpace(string(data))
	if commit == "" {
		return "", fmt.Errorf("BASE_COMMIT is empty in %s", patchesRepo)
	}
	return commit, nil
}

func ReadChromiumVersion(patchesRepo string) (string, error) {
	path := filepath.Join(patchesRepo, "CHROMIUM_VERSION")
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return "", nil
		}
		return "", fmt.Errorf("reading CHROMIUM_VERSION: %w", err)
	}

	vars := make(map[string]string)
	for _, line := range strings.Split(string(data), "\n") {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		parts := strings.SplitN(line, "=", 2)
		if len(parts) == 2 {
			vars[strings.TrimSpace(parts[0])] = strings.TrimSpace(parts[1])
		}
	}

	major := vars["MAJOR"]
	minor := vars["MINOR"]
	build := vars["BUILD"]
	patch := vars["PATCH"]

	if major == "" {
		return "", nil
	}
	return fmt.Sprintf("%s.%s.%s.%s", major, minor, build, patch), nil
}
