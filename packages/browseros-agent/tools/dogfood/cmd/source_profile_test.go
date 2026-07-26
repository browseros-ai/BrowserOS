package cmd

import (
	"bufio"
	"bytes"
	"strings"
	"testing"
)

func TestConfirmSourceProfileImportReturnsWithoutPromptWhenUnlocked(t *testing.T) {
	var out bytes.Buffer
	err := confirmSourceProfileImportWithChecker(
		&out,
		bufio.NewReader(strings.NewReader("")),
		func() (bool, error) { return false, nil },
	)
	if err != nil {
		t.Fatal(err)
	}
	if out.String() != "" {
		t.Fatalf("unexpected prompt: %q", out.String())
	}
}

func TestConfirmSourceProfileImportRetriesAfterUserQuitsBrowserOS(t *testing.T) {
	var out bytes.Buffer
	checks := []bool{true, false}
	err := confirmSourceProfileImportWithChecker(
		&out,
		bufio.NewReader(strings.NewReader("\n")),
		func() (bool, error) {
			next := checks[0]
			checks = checks[1:]
			return next, nil
		},
	)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out.String(), "Quit BrowserOS") {
		t.Fatalf("missing quit prompt: %q", out.String())
	}
}

func TestConfirmSourceProfileImportCanContinuePastStaleLock(t *testing.T) {
	var out bytes.Buffer
	err := confirmSourceProfileImportWithChecker(
		&out,
		bufio.NewReader(strings.NewReader("continue\n")),
		func() (bool, error) { return true, nil },
	)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out.String(), "continue") {
		t.Fatalf("missing continue escape hatch: %q", out.String())
	}
}

func TestConfirmSourceProfileImportCanContinueWithoutTrailingNewline(t *testing.T) {
	var out bytes.Buffer
	err := confirmSourceProfileImportWithChecker(
		&out,
		bufio.NewReader(strings.NewReader("continue")),
		func() (bool, error) { return true, nil },
	)
	if err != nil {
		t.Fatal(err)
	}
}
