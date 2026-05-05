package cmd

import (
	"bytes"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/patch/internal/app"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/patch/internal/workspace"
	"github.com/spf13/cobra"
)

func TestCommandProgressWritesHumanUpdatesToStderr(t *testing.T) {
	oldJSONOut := jsonOut
	t.Cleanup(func() {
		jsonOut = oldJSONOut
	})
	jsonOut = false

	var stderr bytes.Buffer
	cmd := &cobra.Command{}
	cmd.SetErr(&stderr)

	progress := commandProgress(cmd)
	if progress == nil {
		t.Fatalf("expected human progress reporter")
	}
	progress.Step("Applying 1 patch operation")

	if !strings.Contains(stderr.String(), "Applying 1 patch operation") {
		t.Fatalf("expected progress on stderr, got %q", stderr.String())
	}
}

func TestCommandProgressDisabledForJSON(t *testing.T) {
	oldJSONOut := jsonOut
	t.Cleanup(func() {
		jsonOut = oldJSONOut
	})
	jsonOut = true

	if progress := commandProgress(&cobra.Command{}); progress != nil {
		t.Fatalf("expected nil progress reporter in JSON mode")
	}
}

func TestResolveWorkspaceErrorUsesCurrentCommandExample(t *testing.T) {
	oldAppState := appState
	t.Cleanup(func() {
		appState = oldAppState
	})

	root := t.TempDir()
	registered := filepath.Join(root, "chromium-src")
	outside := filepath.Join(root, "outside")
	appState = &app.App{
		CWD: outside,
		Registry: &workspace.Registry{Version: 1, Workspaces: []workspace.Entry{
			{Name: "ch1", Path: registered},
		}},
	}

	rootCmd := &cobra.Command{Use: "browseros-patch"}
	diffCmd := &cobra.Command{Use: "diff"}
	rootCmd.AddCommand(diffCmd)

	_, err := resolveWorkspace(diffCmd, nil, "")
	if err == nil {
		t.Fatalf("expected error")
	}
	if !strings.Contains(err.Error(), `browseros-patch diff ch1`) {
		t.Fatalf("expected command-specific example, got:\n%s", err)
	}
}

func TestResolveWorkspaceNamedCheckoutIgnoresCWD(t *testing.T) {
	oldAppState := appState
	t.Cleanup(func() {
		appState = oldAppState
	})

	root := t.TempDir()
	registered := filepath.Join(root, "chromium-src")
	outside := filepath.Join(root, "outside")
	appState = &app.App{
		CWD: outside,
		Registry: &workspace.Registry{Version: 1, Workspaces: []workspace.Entry{
			{Name: "ch1", Path: registered},
		}},
	}

	rootCmd := &cobra.Command{Use: "browseros-patch"}
	diffCmd := &cobra.Command{Use: "diff"}
	rootCmd.AddCommand(diffCmd)

	ws, err := resolveWorkspace(diffCmd, []string{"ch1"}, "")
	if err != nil {
		t.Fatalf("resolve named checkout: %v", err)
	}
	if ws.Path != registered {
		t.Fatalf("resolved path = %q, want %q", ws.Path, registered)
	}
}

func TestListReadsOnlyRegistry(t *testing.T) {
	oldAppState := appState
	oldJSONOut := jsonOut
	t.Cleanup(func() {
		appState = oldAppState
		jsonOut = oldJSONOut
	})

	missingCheckout := filepath.Join(t.TempDir(), "missing-src")
	appState = &app.App{
		Registry: &workspace.Registry{Version: 1, Workspaces: []workspace.Entry{
			{Name: "ch1", Path: missingCheckout},
		}},
	}
	jsonOut = false

	listCmd, _, err := rootCmd.Find([]string{"list"})
	if err != nil {
		t.Fatalf("find list: %v", err)
	}

	var runErr error
	output := captureStdout(t, func() {
		runErr = listCmd.RunE(listCmd, nil)
	})
	if runErr != nil {
		t.Fatalf("list should not inspect checkout path: %v", runErr)
	}
	for _, want := range []string{"ch1", missingCheckout} {
		if !strings.Contains(output, want) {
			t.Fatalf("expected list output to contain %q, got:\n%s", want, output)
		}
	}
}

func TestPublicHelpUsesCheckoutTerminology(t *testing.T) {
	help := rootCmd.Short + groupedHelp(rootCmd)
	for _, want := range []string{
		"Chromium checkouts",
		"Chromium Checkouts:",
	} {
		if !strings.Contains(help, want) {
			t.Fatalf("expected help to contain %q, got:\n%s", want, help)
		}
	}
	for _, forbidden := range []string{
		"Workspace-centric",
		"Workspace:",
		" workspace",
		" workspaces",
	} {
		if strings.Contains(help, forbidden) {
			t.Fatalf("expected help not to contain %q, got:\n%s", forbidden, help)
		}
	}
}

func TestCheckoutCommandUsageTerminology(t *testing.T) {
	for _, tc := range []struct {
		name string
		use  string
	}{
		{name: "diff", use: "diff [checkout]"},
		{name: "status", use: "status [checkout]"},
		{name: "apply", use: "apply [checkout] [-- files...]"},
		{name: "sync", use: "sync [checkout]"},
		{name: "extract", use: "extract [checkout] [--range <start> <end>] [-- files...]"},
	} {
		cmd, _, err := rootCmd.Find([]string{tc.name})
		if err != nil {
			t.Fatalf("find %s: %v", tc.name, err)
		}
		if cmd.Use != tc.use {
			t.Fatalf("%s use = %q, want %q", tc.name, cmd.Use, tc.use)
		}
		if strings.Contains(strings.ToLower(cmd.Short), "workspace") {
			t.Fatalf("%s short should use checkout terminology: %q", tc.name, cmd.Short)
		}
	}
}

func TestRootHelpExplainsPatchRepoAndCheckoutModel(t *testing.T) {
	for _, want := range []string{
		"patch repo",
		"chromium_patches/",
		"Chromium checkout",
		"ch1",
	} {
		if !strings.Contains(rootCmd.Long, want) {
			t.Fatalf("expected root long help to contain %q, got:\n%s", want, rootCmd.Long)
		}
	}

	for _, want := range []string{
		"browseros-patch add ch1 /path/to/chromium/src",
		"browseros-patch list",
		"browseros-patch diff ch1",
		"browseros-patch sync ch1",
		"browseros-patch extract ch1",
	} {
		if !strings.Contains(rootCmd.Example, want) {
			t.Fatalf("expected root examples to contain %q, got:\n%s", want, rootCmd.Example)
		}
	}
}

func TestCheckoutCommandExamplesUseNamedCheckout(t *testing.T) {
	for _, tc := range []struct {
		name    string
		example string
	}{
		{name: "diff", example: "browseros-patch diff ch1"},
		{name: "status", example: "browseros-patch status ch1"},
		{name: "apply", example: "browseros-patch apply ch1"},
		{name: "sync", example: "browseros-patch sync ch1"},
		{name: "extract", example: "browseros-patch extract ch1"},
	} {
		cmd, _, err := rootCmd.Find([]string{tc.name})
		if err != nil {
			t.Fatalf("find %s: %v", tc.name, err)
		}
		if !strings.Contains(cmd.Example, tc.example) {
			t.Fatalf("expected %s examples to contain %q, got:\n%s", tc.name, tc.example, cmd.Example)
		}
	}
}

func TestSrcFlagExplainsDirectCheckoutPath(t *testing.T) {
	for _, name := range []string{"diff", "status", "apply", "sync", "extract"} {
		cmd, _, err := rootCmd.Find([]string{name})
		if err != nil {
			t.Fatalf("find %s: %v", name, err)
		}
		flag := cmd.Flags().Lookup("src")
		if flag == nil {
			t.Fatalf("%s missing --src flag", name)
		}
		if !strings.Contains(flag.Usage, "without registry lookup") {
			t.Fatalf("%s --src usage should explain registry bypass, got %q", name, flag.Usage)
		}
	}
}

func TestLLMTxtGuideContent(t *testing.T) {
	text := llmTxtGuide()
	for _, want := range []string{
		"patch repo",
		"chromium_patches/",
		"Chromium checkout",
		"checkout name",
		"--src",
		"browseros-patch diff ch1",
		"browseros-patch list",
		"browseros-patch status ch1",
		"browseros-patch sync ch1",
		"browseros-patch apply ch1",
		"browseros-patch extract ch1",
		"list reads only registered Chromium checkouts",
		"does not inspect sync state",
	} {
		if !strings.Contains(text, want) {
			t.Fatalf("expected llm txt to contain %q, got:\n%s", want, text)
		}
	}
	if strings.Contains(text, "\x1b[") {
		t.Fatalf("llm txt should be uncolored, got:\n%s", text)
	}
}

func TestRootLLMTxtPrintsWithoutLoadingApp(t *testing.T) {
	oldAppState := appState
	oldLLMTxt := llmTxt
	t.Cleanup(func() {
		appState = oldAppState
		llmTxt = oldLLMTxt
		rootCmd.SetArgs(nil)
		rootCmd.SetOut(nil)
		rootCmd.SetErr(nil)
	})

	appState = nil
	llmTxt = false
	var stdout bytes.Buffer
	rootCmd.SetArgs([]string{"--llm-txt"})
	rootCmd.SetOut(&stdout)
	rootCmd.SetErr(io.Discard)

	if err := rootCmd.Execute(); err != nil {
		t.Fatalf("execute --llm-txt: %v", err)
	}
	if appState != nil {
		t.Fatalf("--llm-txt should not load app state")
	}
	if !strings.Contains(stdout.String(), "browseros-patch diff ch1") {
		t.Fatalf("expected llm txt output, got:\n%s", stdout.String())
	}
}

func TestLLMTxtRejectedWithSubcommand(t *testing.T) {
	oldAppState := appState
	oldLLMTxt := llmTxt
	t.Cleanup(func() {
		appState = oldAppState
		llmTxt = oldLLMTxt
		rootCmd.SetArgs(nil)
		rootCmd.SetOut(nil)
		rootCmd.SetErr(nil)
	})

	appState = nil
	llmTxt = false
	rootCmd.SetArgs([]string{"diff", "--llm-txt"})
	rootCmd.SetOut(io.Discard)
	rootCmd.SetErr(io.Discard)

	err := rootCmd.Execute()
	if err == nil {
		t.Fatalf("expected --llm-txt subcommand error")
	}
	if !strings.Contains(err.Error(), "unknown flag: --llm-txt") {
		t.Fatalf("unexpected error: %v", err)
	}
	if appState != nil {
		t.Fatalf("--llm-txt subcommand error should not load app state")
	}
}

func TestLLMTxtNotShownInSubcommandHelp(t *testing.T) {
	diffCmd, _, err := rootCmd.Find([]string{"diff"})
	if err != nil {
		t.Fatalf("find diff: %v", err)
	}

	var help bytes.Buffer
	diffCmd.SetOut(&help)
	t.Cleanup(func() {
		diffCmd.SetOut(nil)
	})

	if err := diffCmd.Help(); err != nil {
		t.Fatalf("diff help: %v", err)
	}
	if strings.Contains(help.String(), "--llm-txt") {
		t.Fatalf("subcommand help should not include root-only --llm-txt, got:\n%s", help.String())
	}
}

func captureStdout(t *testing.T, fn func()) string {
	t.Helper()

	oldStdout := os.Stdout
	reader, writer, err := os.Pipe()
	if err != nil {
		t.Fatalf("pipe stdout: %v", err)
	}
	os.Stdout = writer
	defer func() {
		os.Stdout = oldStdout
	}()

	fn()
	os.Stdout = oldStdout

	if err := writer.Close(); err != nil {
		t.Fatalf("close stdout writer: %v", err)
	}
	output, err := io.ReadAll(reader)
	if err != nil {
		t.Fatalf("read stdout: %v", err)
	}
	return string(output)
}
