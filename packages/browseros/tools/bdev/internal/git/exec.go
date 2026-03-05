package git

import (
	"bytes"
	"context"
	"fmt"
	"os/exec"
	"strings"
	"time"
)

const defaultTimeout = 120 * time.Second

func Run(dir string, args ...string) (string, error) {
	out, err := RunBytes(dir, args...)
	return strings.TrimRight(string(out), "\n"), err
}

func RunBytes(dir string, args ...string) ([]byte, error) {
	return RunWithTimeout(dir, defaultTimeout, args...)
}

func RunWithTimeout(dir string, timeout time.Duration, args ...string) ([]byte, error) {
	ctx, cancel := context.WithTimeout(context.Background(), timeout)
	defer cancel()

	cmd := exec.CommandContext(ctx, "git", args...)
	cmd.Dir = dir

	var stdout, stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr

	if err := cmd.Run(); err != nil {
		if ctx.Err() == context.DeadlineExceeded {
			return nil, fmt.Errorf("git %s: timed out after %s", args[0], timeout)
		}
		return nil, fmt.Errorf("git %s: %w\n%s", args[0], err, stderr.String())
	}
	return stdout.Bytes(), nil
}
