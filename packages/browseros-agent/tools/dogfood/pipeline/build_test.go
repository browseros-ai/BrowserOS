package pipeline

import (
	"context"
	"testing"
)

func TestBuildRunsExpectedCommands(t *testing.T) {
	root := t.TempDir()
	r := &FakeRunner{}
	if err := Build(context.Background(), root, r); err != nil {
		t.Fatal(err)
	}
	want := []string{
		"./tools/dev/setup.sh",
		"bun --cwd apps/app --env-file=../../.env.development wxt build --mode development",
		"env NODE_ENV=development bun --env-file=.env.development run -- rustup run 1.98.0 cargo build --locked -p browseros-server",
	}
	if len(r.Commands) != len(want) {
		t.Fatalf("commands got %#v want %#v", r.Commands, want)
	}
	for i := range want {
		if r.Commands[i] != want[i] {
			t.Fatalf("command %d got %q want %q", i, r.Commands[i], want[i])
		}
	}
}

func TestSetupRunsDevSetupOnly(t *testing.T) {
	root := t.TempDir()
	r := &FakeRunner{}
	if err := Setup(context.Background(), root, r); err != nil {
		t.Fatal(err)
	}
	if len(r.Commands) != 1 || r.Commands[0] != "./tools/dev/setup.sh" {
		t.Fatalf("commands got %#v", r.Commands)
	}
}
