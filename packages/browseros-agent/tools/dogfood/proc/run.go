package proc

import (
	"context"
	"os/exec"
	"sync"
)

func RunBlocking(ctx context.Context, dir string, t Tag, args ...string) error {
	return runBlocking(ctx, dir, nil, t, args...)
}

func RunBlockingWithEnv(ctx context.Context, dir string, env []string, t Tag, args ...string) error {
	return runBlocking(ctx, dir, env, t, args...)
}

func runBlocking(ctx context.Context, dir string, env []string, t Tag, args ...string) error {
	cmd := exec.CommandContext(ctx, args[0], args[1:]...)
	cmd.Dir = dir
	if env != nil {
		cmd.Env = env
	}

	stdout, _ := cmd.StdoutPipe()
	stderr, _ := cmd.StderrPipe()

	if err := cmd.Start(); err != nil {
		return err
	}

	var wg sync.WaitGroup
	wg.Add(2)
	go func() { defer wg.Done(); StreamLines(stdout, t) }()
	go func() { defer wg.Done(); StreamLines(stderr, t) }()
	wg.Wait()

	return cmd.Wait()
}
