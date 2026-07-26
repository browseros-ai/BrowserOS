package cmd

import (
	"context"
	"errors"
	"strings"
	"testing"

	"browseros-dogfood/config"
)

func TestUpdateConfiguredRepoSwitchesToConfiguredBranchBeforePull(t *testing.T) {
	r := &recordingRunner{output: map[string]string{
		"git status --porcelain":    "",
		"git branch --show-current": "feature\n",
	}}
	cfg := config.Config{RepoPath: "/repo", Branch: "dogfood"}

	if err := updateConfiguredRepo(context.Background(), cfg, r, repoUpdateOptions{}); err != nil {
		t.Fatal(err)
	}

	want := []string{
		"git status --porcelain",
		"git fetch --prune",
		"git branch --show-current",
		"git switch dogfood",
		"git pull --ff-only",
	}
	if got := strings.Join(r.commands, "\n"); got != strings.Join(want, "\n") {
		t.Fatalf("commands got:\n%s\nwant:\n%s", got, strings.Join(want, "\n"))
	}
}

func TestUpdateConfiguredRepoRejectsDirtyCheckoutBeforeSwitch(t *testing.T) {
	r := &recordingRunner{output: map[string]string{
		"git status --porcelain": " M file.go\n",
	}}
	cfg := config.Config{RepoPath: "/repo", Branch: "dogfood"}

	err := updateConfiguredRepo(context.Background(), cfg, r, repoUpdateOptions{})

	if err == nil || !strings.Contains(err.Error(), "checkout has uncommitted changes") {
		t.Fatalf("error got %v", err)
	}
	if len(r.commands) != 1 || r.commands[0] != "git status --porcelain" {
		t.Fatalf("commands got %#v", r.commands)
	}
}

func TestUpdateConfiguredRepoFetchesSwitchesAndResetsWhenRequested(t *testing.T) {
	r := &recordingRunner{output: map[string]string{
		"git branch --show-current": "feature\n",
	}}
	cfg := config.Config{RepoPath: "/repo", Branch: "dogfood"}

	if err := updateConfiguredRepo(context.Background(), cfg, r, repoUpdateOptions{Force: true, ResetToUpstream: true}); err != nil {
		t.Fatal(err)
	}

	want := []string{
		"git fetch --prune",
		"git branch --show-current",
		"git switch --force dogfood",
		"git reset --hard @{upstream}",
	}
	if got := strings.Join(r.commands, "\n"); got != strings.Join(want, "\n") {
		t.Fatalf("commands got:\n%s\nwant:\n%s", got, strings.Join(want, "\n"))
	}
}

type recordingRunner struct {
	commands      []string
	output        map[string]string
	err           error
	commandErrors map[string]error
}

func (r *recordingRunner) Run(ctx context.Context, dir string, args ...string) error {
	cmd := strings.Join(args, " ")
	r.commands = append(r.commands, cmd)
	if err := r.commandErrors[cmd]; err != nil {
		return err
	}
	return r.err
}

func (r *recordingRunner) OutputRun(dir string, args ...string) (string, error) {
	cmd := strings.Join(args, " ")
	r.commands = append(r.commands, cmd)
	if r.err != nil {
		return "", r.err
	}
	if r.output == nil {
		return "", errors.New("missing output")
	}
	return r.output[cmd], nil
}
