package git

import (
	"fmt"
	"strings"
)

func StatusPorcelain(dir string, pathspec ...string) ([]string, error) {
	args := []string{"status", "--porcelain"}
	if len(pathspec) > 0 {
		args = append(args, "--")
		args = append(args, pathspec...)
	}

	out, err := Run(dir, args...)
	if err != nil {
		return nil, fmt.Errorf("status --porcelain: %w", err)
	}

	if strings.TrimSpace(out) == "" {
		return nil, nil
	}

	var lines []string
	for _, line := range strings.Split(out, "\n") {
		line = strings.TrimSpace(line)
		if line != "" {
			lines = append(lines, line)
		}
	}
	return lines, nil
}

func IsDirty(dir string, pathspec ...string) (bool, error) {
	lines, err := StatusPorcelain(dir, pathspec...)
	if err != nil {
		return false, err
	}
	return len(lines) > 0, nil
}

func Add(dir string, pathspec ...string) error {
	args := []string{"add", "-A"}
	if len(pathspec) > 0 {
		args = append(args, "--")
		args = append(args, pathspec...)
	}
	_, err := Run(dir, args...)
	if err != nil {
		return fmt.Errorf("add: %w", err)
	}
	return nil
}

func Commit(dir, message string) error {
	_, err := Run(dir, "commit", "-m", message)
	if err != nil {
		return fmt.Errorf("commit: %w", err)
	}
	return nil
}
