package build

import (
	"bytes"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"strings"
)

type cmdResult struct {
	Stdout   string
	Stderr   string
	ExitCode int
}

func runCmdCapture(dir string, name string, args ...string) (cmdResult, error) {
	cmd := exec.Command(name, args...)
	if strings.TrimSpace(dir) != "" {
		cmd.Dir = dir
	}

	var stdout bytes.Buffer
	var stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr

	err := cmd.Run()
	result := cmdResult{
		Stdout: strings.TrimSpace(stdout.String()),
		Stderr: strings.TrimSpace(stderr.String()),
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

	return result, fmt.Errorf("running %s %s: %w", name, strings.Join(args, " "), err)
}

func runCmdWithEnv(dir string, extraEnv map[string]string, name string, args ...string) error {
	cmd := exec.Command(name, args...)
	if strings.TrimSpace(dir) != "" {
		cmd.Dir = dir
	}
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	cmd.Stdin = os.Stdin
	cmd.Env = os.Environ()
	for key, value := range extraEnv {
		cmd.Env = append(cmd.Env, fmt.Sprintf("%s=%s", key, value))
	}
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("%s %s: %w", name, strings.Join(args, " "), err)
	}
	return nil
}
