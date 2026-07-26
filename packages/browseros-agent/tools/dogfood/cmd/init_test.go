package cmd

import (
	"bufio"
	"bytes"
	"path/filepath"
	"strings"
	"testing"

	"browseros-dogfood/config"
	"browseros-dogfood/profile"
)

func TestPrintInitNextStepsShowsInlineAndBackgroundStart(t *testing.T) {
	var out bytes.Buffer
	printInitNextSteps(&out, "/tmp/config.yaml", config.TargetClaw)

	got := out.String()
	for _, want := range []string{
		"Config written: /tmp/config.yaml",
		"Start dogfood: BrowserClaw",
		"Inline:     browseros-dogfood --claw start",
		"Background: browseros-dogfood --claw start-background",
	} {
		if !strings.Contains(got, want) {
			t.Fatalf("missing %q in\n%s", want, got)
		}
	}
}

func TestLoadInitConfigPreservesExistingOtherTarget(t *testing.T) {
	home := t.TempDir()
	path := filepath.Join(t.TempDir(), "config.yaml")
	cfg := config.Defaults(home)
	if err := cfg.ApplyTarget(config.TargetBrowserOS); err != nil {
		t.Fatal(err)
	}
	cfg.DevUserDataDir = "/custom-browseros-profile"
	cfg.BrowserOSDir = "/custom-browseros-state"
	if err := config.Save(path, cfg); err != nil {
		t.Fatal(err)
	}

	got, err := loadInitConfig(home, path, config.TargetClaw)
	if err != nil {
		t.Fatal(err)
	}

	if got.Target != config.TargetClaw {
		t.Fatalf("target got %q want claw", got.Target)
	}
	browseros := got.Targets[string(config.TargetBrowserOS)]
	if browseros.DevUserDataDir != "/custom-browseros-profile" || browseros.BrowserOSDir != "/custom-browseros-state" {
		t.Fatalf("browseros target was not preserved: %+v", browseros)
	}
	if got.DevUserDataDir != filepath.Join(home, ".config/browseros-dogfood/claw/profile") {
		t.Fatalf("active target profile got %q", got.DevUserDataDir)
	}
}

func TestPrintRepoPathHelpExplainsOnlyRepoPath(t *testing.T) {
	var out bytes.Buffer
	printRepoPathHelp(&out)

	got := stripANSI(out.String())
	for _, want := range []string{
		"Repo path is the root BrowserOS repo clone for alpha dogfood.",
		"Use a separate clone from your everyday dev checkout if you can.",
		"Example: /Users/you/code/browseros-alpha",
		"not packages/browseros-agent",
	} {
		if !strings.Contains(got, want) {
			t.Fatalf("missing %q in\n%s", want, got)
		}
	}
	if strings.Contains(got, "BrowserOS binary") {
		t.Fatalf("repo path help should not explain BrowserOS binary:\n%s", got)
	}
}

func TestPrintSourceProfileHelpExplainsProfileChoice(t *testing.T) {
	var out bytes.Buffer
	printSourceProfileHelp(&out)

	got := stripANSI(out.String())
	for _, want := range []string{
		"Choose the installed BrowserOS profile you normally use.",
		"Dogfood copies it into a separate dev profile.",
	} {
		if !strings.Contains(got, want) {
			t.Fatalf("missing %q in\n%s", want, got)
		}
	}
}

func TestPromptWritesPromptToOutputWriter(t *testing.T) {
	var out bytes.Buffer
	reader := bufio.NewReader(strings.NewReader("\n"))

	got := prompt(&out, reader, "Repo path", "/tmp/browseros-alpha")

	if got != "/tmp/browseros-alpha" {
		t.Fatalf("prompt returned %q", got)
	}
	if want := "Repo path [/tmp/browseros-alpha]: "; !strings.Contains(stripANSI(out.String()), want) {
		t.Fatalf("missing prompt %q in\n%s", want, out.String())
	}
}

func TestChooseProfileWritesChoicesToOutputWriter(t *testing.T) {
	var out bytes.Buffer
	reader := bufio.NewReader(strings.NewReader("\n"))

	got := chooseProfile(&out, reader, []profile.BrowserProfile{{
		Name:  "Main",
		Dir:   "Default",
		Email: "you@example.com",
	}})

	if got != "Default" {
		t.Fatalf("chooseProfile returned %q", got)
	}
	for _, want := range []string{
		"Found 1 BrowserOS profiles:",
		"1. Main (Default) you@example.com",
		"Select source profile [1]: ",
	} {
		if !strings.Contains(stripANSI(out.String()), want) {
			t.Fatalf("missing %q in\n%s", want, out.String())
		}
	}
}
