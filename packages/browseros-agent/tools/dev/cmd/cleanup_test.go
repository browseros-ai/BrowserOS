package cmd

import (
	"bufio"
	"bytes"
	"os"
	"strings"
	"testing"
)

func TestConfirmYesNoDefaultsNoAndExplainsAction(t *testing.T) {
	var out bytes.Buffer
	prompt := resetPrompt{
		Title:  "Stop VM?",
		Body:   "This shuts down browseros-vm. Data stays on disk.",
		Action: "Stop browseros-vm",
	}

	ok, err := confirmYesNo(&out, bufio.NewReader(strings.NewReader("\n")), prompt)
	if err != nil {
		t.Fatal(err)
	}

	if ok {
		t.Fatal("expected empty answer to default to no")
	}
	text := out.String()
	for _, want := range []string{
		"Stop VM?",
		"This shuts down browseros-vm. Data stays on disk.",
		"Stop browseros-vm",
		"[y/N]",
	} {
		if !strings.Contains(text, want) {
			t.Fatalf("missing %q in prompt:\n%s", want, text)
		}
	}
}

func TestConfirmTypedRequiresExactToken(t *testing.T) {
	var out bytes.Buffer
	ok, err := confirmTyped(
		&out,
		bufio.NewReader(strings.NewReader("delete\nDELETE\n")),
		"Delete dev profile?",
		"This removes ~/.browseros-dev.",
		"DELETE",
	)
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("expected exact token to confirm")
	}

	text := out.String()
	if !strings.Contains(text, "Type DELETE to continue") {
		t.Fatalf("missing typed confirmation instruction:\n%s", text)
	}
	if !strings.Contains(text, "Confirmation did not match") {
		t.Fatalf("missing retry warning:\n%s", text)
	}
}

func TestResetOverviewTellsUserToUseSmallestReset(t *testing.T) {
	var out bytes.Buffer
	printResetOverview(&out, resetTarget{
		Title:           "BrowserOS dev reset",
		BrowserOSDir:    "/Users/me/.browseros-dev",
		DeleteRootLabel: "Delete dev profile:",
	})

	text := out.String()
	for _, want := range []string{
		"BrowserOS dev reset",
		"Pick the smallest reset",
		"/Users/me/.browseros-dev",
		"Stop VM",
		"Delete VM",
		"Delete dev profile",
	} {
		if !strings.Contains(text, want) {
			t.Fatalf("missing %q in overview:\n%s", want, text)
		}
	}
}

func TestParseLimaListOutputAcceptsSingleObject(t *testing.T) {
	entries, err := parseLimaListOutput([]byte(`{"name":"browseros-vm","status":"Running"}`))
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != 1 || entries[0].Name != "browseros-vm" || entries[0].Status != "Running" {
		t.Fatalf("unexpected entries: %#v", entries)
	}
}

func TestParseLimaListOutputAcceptsJSONLines(t *testing.T) {
	entries, err := parseLimaListOutput([]byte("{\"name\":\"one\",\"status\":\"Stopped\"}\n{\"name\":\"browseros-vm\",\"status\":\"Running\"}\n"))
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != 2 || entries[1].Name != "browseros-vm" || entries[1].Status != "Running" {
		t.Fatalf("unexpected entries: %#v", entries)
	}
}

func TestValidateDevProfileRootRejectsUnsafePaths(t *testing.T) {
	home, err := os.UserHomeDir()
	if err != nil {
		t.Fatal(err)
	}
	for _, path := range []string{"/", home, "/etc"} {
		if err := validateDevProfileRootForDeletion(path); err == nil {
			t.Fatalf("expected %s to be rejected", path)
		}
	}
}

func TestLimactlShellArgsUseGuestWorkdir(t *testing.T) {
	args := limactlShellArgs("sh", "-lc", "true")
	want := []string{"shell", "--workdir", "/", "browseros-vm", "--", "sh", "-lc", "true"}
	if strings.Join(args, "\x00") != strings.Join(want, "\x00") {
		t.Fatalf("expected %#v, got %#v", want, args)
	}
}

func TestParsePodmanMachineList(t *testing.T) {
	machines, err := parsePodmanMachineList([]byte(`[{"Name":"podman-machine-default","Running":true}]`))
	if err != nil {
		t.Fatal(err)
	}
	if len(machines) != 1 || machines[0].Name != "podman-machine-default" || !machines[0].Running {
		t.Fatalf("unexpected machines: %#v", machines)
	}
}
