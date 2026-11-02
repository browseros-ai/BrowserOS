package pipeline

import "context"

func Setup(ctx context.Context, agentRoot string, r Runner) error {
	return r.Run(ctx, agentRoot, "./tools/dev/setup.sh")
}

type ExecRunner struct{}

func (ExecRunner) Run(ctx context.Context, dir string, args ...string) error {
	return runCommand(ctx, dir, args...)
}

func (ExecRunner) OutputRun(dir string, args ...string) (string, error) {
	return outputCommand(dir, args...)
}
