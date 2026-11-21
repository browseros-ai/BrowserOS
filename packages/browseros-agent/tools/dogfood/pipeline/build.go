package pipeline

import "context"

func Build(ctx context.Context, agentRoot string, r Runner) error {
	if err := Setup(ctx, agentRoot, r); err != nil {
		return err
	}
	if err := r.Run(ctx, agentRoot, "bun", "--cwd", "apps/app", "--env-file=../../.env.development", "wxt", "build", "--mode", "development"); err != nil {
		return err
	}
	// Finish the cold Cargo build before the daemon starts its health deadline.
	return r.Run(ctx, agentRoot, BrowserOSServerCargoCommand("build")...)
}

// BrowserOSServerCargoCommand gives the build and launch the same toolchain and
// dotenv inputs so Cargo can reuse its build. Inherited values, including the
// dogfood state directory on launch, take precedence over the file.
func BrowserOSServerCargoCommand(action string, args ...string) []string {
	return append([]string{
		"env", "NODE_ENV=development", "bun", "--env-file=.env.development", "run", "--",
		"rustup", "run", "1.98.0", "cargo", action, "--locked", "-p", "browseros-server",
	}, args...)
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
