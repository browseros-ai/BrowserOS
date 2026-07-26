package pipeline

import (
	"context"
	"testing"
)

func TestDirtyStatus(t *testing.T) {
	r := &FakeRunner{Output: map[string]string{
		"git status --porcelain": " M file.go\n",
	}}
	dirty, err := Dirty("/repo", r)
	if err != nil {
		t.Fatal(err)
	}
	if !dirty {
		t.Fatal("expected dirty")
	}
}

func TestPullRunsFastForwardOnly(t *testing.T) {
	r := &FakeRunner{}
	if err := Pull(context.Background(), "/repo", r); err != nil {
		t.Fatal(err)
	}
	if got := r.Commands[0]; got != "git pull --ff-only" {
		t.Fatalf("got %q", got)
	}
}

func TestFetchRunsPrune(t *testing.T) {
	r := &FakeRunner{}
	if err := Fetch(context.Background(), "/repo", r); err != nil {
		t.Fatal(err)
	}
	if got := r.Commands[0]; got != "git fetch --prune" {
		t.Fatalf("got %q", got)
	}
}

func TestResetHardToUpstream(t *testing.T) {
	r := &FakeRunner{}
	if err := ResetHardToUpstream(context.Background(), "/repo", r); err != nil {
		t.Fatal(err)
	}
	if got := r.Commands[0]; got != "git reset --hard @{upstream}" {
		t.Fatalf("got %q", got)
	}
}

func TestEnsureBranchSwitchesWhenCurrentBranchDiffers(t *testing.T) {
	r := &FakeRunner{Output: map[string]string{
		"git branch --show-current": "feature\n",
	}}

	if err := EnsureBranch(context.Background(), "/repo", "dogfood", r, false); err != nil {
		t.Fatal(err)
	}

	want := []string{"git branch --show-current", "git switch dogfood"}
	for i := range want {
		if r.Commands[i] != want[i] {
			t.Fatalf("command %d got %q want %q", i, r.Commands[i], want[i])
		}
	}
}

func TestEnsureBranchSkipsSwitchWhenAlreadyOnBranch(t *testing.T) {
	r := &FakeRunner{Output: map[string]string{
		"git branch --show-current": "dogfood\n",
	}}

	if err := EnsureBranch(context.Background(), "/repo", "dogfood", r, false); err != nil {
		t.Fatal(err)
	}

	if len(r.Commands) != 1 || r.Commands[0] != "git branch --show-current" {
		t.Fatalf("commands got %#v", r.Commands)
	}
}

func TestEnsureBranchForceSwitchesWhenRequested(t *testing.T) {
	r := &FakeRunner{Output: map[string]string{
		"git branch --show-current": "feature\n",
	}}

	if err := EnsureBranch(context.Background(), "/repo", "dogfood", r, true); err != nil {
		t.Fatal(err)
	}

	want := []string{"git branch --show-current", "git switch --force dogfood"}
	for i := range want {
		if r.Commands[i] != want[i] {
			t.Fatalf("command %d got %q want %q", i, r.Commands[i], want[i])
		}
	}
}

type FakeRunner struct {
	Commands []string
	Output   map[string]string
}

func (f *FakeRunner) Run(ctx context.Context, dir string, args ...string) error {
	f.Commands = append(f.Commands, join(args))
	return nil
}

func (f *FakeRunner) OutputRun(dir string, args ...string) (string, error) {
	cmd := join(args)
	f.Commands = append(f.Commands, cmd)
	return f.Output[cmd], nil
}

func join(args []string) string {
	out := ""
	for i, arg := range args {
		if i > 0 {
			out += " "
		}
		out += arg
	}
	return out
}
