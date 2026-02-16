package dev

import (
	"bytes"
	"errors"
	"fmt"
	"os/exec"
	"sort"
	"strings"
)

type gitResult struct {
	Stdout   string
	Stderr   string
	ExitCode int
}

func runGit(cwd string, args ...string) (gitResult, error) {
	cmd := exec.Command("git", args...)
	cmd.Dir = cwd

	var stdout bytes.Buffer
	var stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr

	err := cmd.Run()
	result := gitResult{
		Stdout: strings.TrimRight(stdout.String(), "\n"),
		Stderr: strings.TrimRight(stderr.String(), "\n"),
	}

	if err == nil {
		result.ExitCode = 0
		return result, nil
	}

	var exitErr *exec.ExitError
	if errors.As(err, &exitErr) {
		result.ExitCode = exitErr.ExitCode()
		return result, nil
	}

	return result, fmt.Errorf("running git %s: %w", strings.Join(args, " "), err)
}

// commitChangedFiles returns changed paths in a commit (new path for rename/copy).
func commitChangedFiles(cwd string, commitRef string) ([]string, error) {
	result, err := runGit(cwd, "diff-tree", "--no-commit-id", "--name-status", "-r", commitRef)
	if err != nil {
		return nil, err
	}
	if result.ExitCode != 0 || strings.TrimSpace(result.Stdout) == "" {
		return nil, nil
	}

	files := make([]string, 0)
	seen := map[string]struct{}{}
	for _, line := range strings.Split(result.Stdout, "\n") {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		parts := strings.Split(line, "\t")
		if len(parts) < 2 {
			continue
		}
		filePath := strings.TrimSpace(parts[len(parts)-1])
		if filePath == "" {
			continue
		}
		if _, ok := seen[filePath]; ok {
			continue
		}
		seen[filePath] = struct{}{}
		files = append(files, filePath)
	}

	sort.Strings(files)
	return files, nil
}
