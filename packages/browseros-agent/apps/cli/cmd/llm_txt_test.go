package cmd

import (
	"bytes"
	"strings"
	"testing"
)

func TestLLMTxtGuideCoversTheContract(t *testing.T) {
	if strings.TrimSpace(llmTxtGuide) == "" {
		t.Fatal("llmTxtGuide is empty")
	}
	for _, want := range []string{
		"browseros-cli",          // the tool name
		"-p",                     // explicit page contract
		"open --json",            // how to capture a page id
		"snapshot",               // the observe step
		"UNTRUSTED_PAGE_CONTENT", // trust boundary
		"find text",              // no-ref locator
		"batch",                  // one-session flow
	} {
		if !strings.Contains(llmTxtGuide, want) {
			t.Fatalf("llmTxtGuide missing %q", want)
		}
	}
}

func TestLLMTxtFlagRegistered(t *testing.T) {
	if rootCmd.Flags().Lookup("llm-txt") == nil {
		t.Fatal("--llm-txt flag is not registered on root")
	}
}

func TestRunRootPrintsGuideWhenFlagSet(t *testing.T) {
	defer func() { showLLMTxt = false }()
	defer rootCmd.SetOut(nil)

	showLLMTxt = true
	var buf bytes.Buffer
	rootCmd.SetOut(&buf)

	runRoot(rootCmd, nil)

	if got := buf.String(); !strings.Contains(got, "browseros-cli — agent guide") {
		t.Fatalf("runRoot did not print the guide; got:\n%s", got)
	}
}

func TestLLMTxtSkipsAutomaticUpdates(t *testing.T) {
	if !shouldSkipAutomaticUpdates([]string{"--llm-txt"}) {
		t.Fatal("shouldSkipAutomaticUpdates([--llm-txt]) = false, want true")
	}
}
