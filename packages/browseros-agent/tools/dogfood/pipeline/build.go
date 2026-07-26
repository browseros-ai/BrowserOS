package pipeline

import "context"

func Build(ctx context.Context, agentRoot string, r Runner) error {
	if err := Setup(ctx, agentRoot, r); err != nil {
		return err
	}
	return r.Run(ctx, agentRoot, "bun", "--cwd", "apps/app", "--env-file=../../.env.development", "wxt", "build", "--mode", "development")
}

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
