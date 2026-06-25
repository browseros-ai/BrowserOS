package cmd

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"browseros-cli/config"
)

func TestSetVersionUpdatesRootCommand(t *testing.T) {
	originalVersion := version
	originalRootVersion := rootCmd.Version
	t.Cleanup(func() {
		version = originalVersion
		rootCmd.Version = originalRootVersion
	})

	SetVersion("1.2.3")

	if version != "1.2.3" {
		t.Fatalf("version = %q, want %q", version, "1.2.3")
	}
	if rootCmd.Version != "1.2.3" {
		t.Fatalf("rootCmd.Version = %q, want %q", rootCmd.Version, "1.2.3")
	}
}

func TestCommandName(t *testing.T) {
	tests := []struct {
		name string
		args []string
		want string
	}{
		{"empty args", nil, "unknown"},
		{"known command", []string{"health"}, "browseros-cli health"},
		{"diff command", []string{"diff"}, "browseros-cli diff"},
		{"unknown command", []string{"nonexistent"}, "unknown"},
		{"subcommand", []string{"bookmark", "search"}, "browseros-cli bookmark search"},
		{"strata subcommand", []string{"strata", "check"}, "browseros-cli strata check"},
		{"known with extra args", []string{"snap", "extra"}, "browseros-cli snapshot"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := commandName(tt.args)
			if got != tt.want {
				t.Errorf("commandName(%v) = %q, want %q", tt.args, got, tt.want)
			}
		})
	}
}

func TestSnapCommandShape(t *testing.T) {
	cmd, _, err := rootCmd.Find([]string{"snap"})
	if err != nil {
		t.Fatalf("rootCmd.Find(snap) error = %v", err)
	}
	if cmd.Name() != "snapshot" {
		t.Fatalf("command name = %q, want snapshot", cmd.Name())
	}
	if err := cmd.Args(cmd, []string{"extra"}); err == nil {
		t.Fatal("snap Args accepted a positional argument")
	}
	for _, flag := range []string{"enhanced", "interactive", "compact", "depth"} {
		if cmd.Flags().Lookup(flag) != nil {
			t.Fatalf("snap command exposes unsupported %s flag", flag)
		}
	}
}

func TestScreenshotCommandShape(t *testing.T) {
	for _, name := range []string{"screenshot", "ss"} {
		t.Run(name, func(t *testing.T) {
			cmd, _, err := rootCmd.Find([]string{name})
			if err != nil {
				t.Fatalf("rootCmd.Find(%s) error = %v", name, err)
			}
			if cmd.Name() != "screenshot" {
				t.Fatalf("%s resolved to %q, want screenshot", name, cmd.Name())
			}
		})
	}
}

func TestAgentFriendlyInputCommandShapes(t *testing.T) {
	for _, tt := range []struct {
		input string
		want  string
	}{
		{"press", "press"},
		{"key", "press"},
		{"type", "type"},
	} {
		t.Run(tt.input, func(t *testing.T) {
			cmd, _, err := rootCmd.Find([]string{tt.input})
			if err != nil {
				t.Fatalf("rootCmd.Find(%s) error = %v", tt.input, err)
			}
			if cmd.Name() != tt.want {
				t.Fatalf("%s resolved to %q, want %q", tt.input, cmd.Name(), tt.want)
			}
		})
	}
}

func TestReadAndGrepCommandShapes(t *testing.T) {
	for _, name := range []string{"read", "text", "links", "grep"} {
		t.Run(name, func(t *testing.T) {
			cmd, _, err := rootCmd.Find([]string{name})
			if err != nil {
				t.Fatalf("rootCmd.Find(%s) error = %v", name, err)
			}
			if cmd == rootCmd {
				t.Fatalf("%s resolved to root command", name)
			}
		})
	}
}

func TestAgentStartHelpShowsExplicitPageLoop(t *testing.T) {
	help := agentStartHelp(rootCmd)
	for _, want := range []string{
		"Start here for agents:",
		"open --json",
		"jq -r .page",
		"-p \"$page\" snapshot",
		"-p \"$page\" read",
		"-p \"$page\" find",
	} {
		if !strings.Contains(help, want) {
			t.Fatalf("agentStartHelp() missing %q in:\n%s", want, help)
		}
	}
}

func TestNpmPackageExposesBosAlias(t *testing.T) {
	data, err := os.ReadFile("../npm/package.json")
	if err != nil {
		t.Fatal(err)
	}
	var pkg struct {
		Bin map[string]string `json:"bin"`
	}
	if err := json.Unmarshal(data, &pkg); err != nil {
		t.Fatal(err)
	}
	if pkg.Bin["browseros-cli"] != "bin/browseros-cli.js" {
		t.Fatalf("browseros-cli bin = %q", pkg.Bin["browseros-cli"])
	}
	if pkg.Bin["bos"] != "bin/browseros-cli.js" {
		t.Fatalf("bos bin = %q, want shared launcher", pkg.Bin["bos"])
	}
}

func TestInstallScriptsCreateBosAlias(t *testing.T) {
	shellScript, err := os.ReadFile("../scripts/install.sh")
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(shellScript), `"${INSTALL_DIR}/bos"`) {
		t.Fatal("install.sh does not create bos next to browseros-cli")
	}

	powerShell, err := os.ReadFile("../scripts/install.ps1")
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(powerShell), `"bos.exe"`) {
		t.Fatal("install.ps1 does not create bos.exe next to browseros-cli.exe")
	}
}

func TestDiffCommandShape(t *testing.T) {
	cmd, _, err := rootCmd.Find([]string{"diff"})
	if err != nil {
		t.Fatalf("rootCmd.Find(diff) error = %v", err)
	}
	if cmd.Name() != "diff" {
		t.Fatalf("command name = %q, want diff", cmd.Name())
	}
	if err := cmd.Args(cmd, []string{"extra"}); err == nil {
		t.Fatal("diff Args accepted a positional argument")
	}
	if cmd.LocalFlags().HasAvailableFlags() {
		t.Fatal("diff command exposes local flags")
	}
}

func TestTabsCommandShape(t *testing.T) {
	cmd, _, err := rootCmd.Find([]string{"tabs"})
	if err != nil {
		t.Fatalf("rootCmd.Find(tabs) error = %v", err)
	}
	if cmd.Name() != "tabs" {
		t.Fatalf("command name = %q, want tabs", cmd.Name())
	}

	alias, _, err := rootCmd.Find([]string{"pages"})
	if err != nil {
		t.Fatalf("rootCmd.Find(pages) error = %v", err)
	}
	if alias.Name() != "tabs" {
		t.Fatalf("pages alias resolved to %q, want tabs", alias.Name())
	}
	if err := cmd.Args(cmd, []string{"extra"}); err == nil {
		t.Fatal("tabs Args accepted a positional argument")
	}
	if cmd.LocalFlags().HasAvailableFlags() {
		t.Fatal("tabs command exposes local flags")
	}
}

func TestCloseCommandRequiresPageFlagOnly(t *testing.T) {
	cmd, _, err := rootCmd.Find([]string{"close"})
	if err != nil {
		t.Fatalf("rootCmd.Find(close) error = %v", err)
	}
	if cmd.Name() != "close" {
		t.Fatalf("command name = %q, want close", cmd.Name())
	}
	if err := cmd.Args(cmd, []string{"7"}); err == nil {
		t.Fatal("close Args accepted a positional page id")
	}
}

func TestRequireExplicitPageID(t *testing.T) {
	t.Setenv("BROWSEROS_PAGE", "9")

	page, err := explicitPageID(true, 7)
	if err != nil {
		t.Fatalf("explicitPageID(true, 7) error = %v", err)
	}
	if page != 7 {
		t.Fatalf("explicitPageID(true, 7) = %d, want 7", page)
	}

	if _, err := explicitPageID(true, 0); err == nil {
		t.Fatal("explicitPageID(true, 0) error = nil, want invalid page error")
	}

	if _, err := explicitPageID(false, 0); err == nil {
		t.Fatal("explicitPageID(false, 0) error = nil, want missing page error")
	} else if !strings.Contains(err.Error(), "-p/--page") || strings.Contains(err.Error(), "active") {
		t.Fatalf("missing page error = %q, want explicit page guidance without active-page fallback", err)
	}
}

func TestUnsupportedCommandsAreNotRegistered(t *testing.T) {
	for _, name := range []string{"dialog", "dom", "dom-search"} {
		t.Run(name, func(t *testing.T) {
			cmd, _, err := rootCmd.Find([]string{name})
			if err == nil && cmd != nil && cmd != rootCmd {
				t.Fatalf("root command resolved unsupported command %q to %q", name, cmd.CommandPath())
			}
		})
	}
}

func TestPrimaryCommand(t *testing.T) {
	tests := []struct {
		name string
		args []string
		want string
	}{
		{"empty", nil, ""},
		{"root flag then command", []string{"--json", "update"}, "update"},
		{"subcommand", []string{"bookmark", "update"}, "bookmark"},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := primaryCommand(tt.args); got != tt.want {
				t.Fatalf("primaryCommand(%v) = %q, want %q", tt.args, got, tt.want)
			}
		})
	}
}

func TestRequestedBoolFlag(t *testing.T) {
	if !requestedBoolFlag([]string{"--json"}, "--json", false) {
		t.Fatal("requestedBoolFlag() = false, want true")
	}
	if !requestedBoolFlag([]string{"--debug=true"}, "--debug", false) {
		t.Fatal("requestedBoolFlag() with assignment = false, want true")
	}
	if requestedBoolFlag([]string{"--debug=false"}, "--debug", false) {
		t.Fatal("requestedBoolFlag() with false assignment = true, want false")
	}
}

func TestValidateChangedIntMinimum(t *testing.T) {
	if err := validateChangedIntMinimum("--limit", 0, false, 1); err != nil {
		t.Fatalf("unchanged value returned error: %v", err)
	}
	if err := validateChangedIntMinimum("--limit", 0, true, 1); err == nil {
		t.Fatal("validateChangedIntMinimum() error = nil, want minimum error")
	}
	if err := validateChangedIntMinimum("--depth", 0, true, 0); err != nil {
		t.Fatalf("valid changed value returned error: %v", err)
	}
}

func TestShouldSkipAutomaticUpdates(t *testing.T) {
	tests := []struct {
		name string
		args []string
		want bool
	}{
		{"short help flag", []string{"-h"}, true},
		{"help flag", []string{"--help"}, true},
		{"version flag", []string{"--version"}, true},
		{"update command", []string{"update"}, true},
		{"bookmark update subcommand", []string{"bookmark", "update"}, false},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := shouldSkipAutomaticUpdates(tt.args); got != tt.want {
				t.Fatalf("shouldSkipAutomaticUpdates(%v) = %t, want %t", tt.args, got, tt.want)
			}
		})
	}
}

func TestDefaultServerURLUsesEnvBeforeConfig(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	t.Setenv("BROWSEROS_URL", "http://127.0.0.1:9115/mcp")

	if err := config.Save(&config.Config{ServerURL: "http://127.0.0.1:9000/mcp"}); err != nil {
		t.Fatalf("config.Save() error = %v", err)
	}

	got := defaultServerURL()
	if got != "http://127.0.0.1:9115" {
		t.Fatalf("defaultServerURL() = %q, want %q", got, "http://127.0.0.1:9115")
	}
}

func TestDefaultServerURLUsesSavedConfig(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	t.Setenv("BROWSEROS_URL", "")

	if err := config.Save(&config.Config{ServerURL: "http://127.0.0.1:9115/mcp"}); err != nil {
		t.Fatalf("config.Save() error = %v", err)
	}

	got := defaultServerURL()
	if got != "http://127.0.0.1:9115" {
		t.Fatalf("defaultServerURL() = %q, want %q", got, "http://127.0.0.1:9115")
	}
}

func TestDefaultServerURLIgnoresBrowserOSServerJSON(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	t.Setenv("BROWSEROS_URL", "")

	serverDir := filepath.Join(home, ".browseros")
	if err := os.MkdirAll(serverDir, 0755); err != nil {
		t.Fatalf("os.MkdirAll() error = %v", err)
	}
	data := []byte(`{"url":"http://127.0.0.1:9999"}`)
	if err := os.WriteFile(filepath.Join(serverDir, "server.json"), data, 0644); err != nil {
		t.Fatalf("os.WriteFile() error = %v", err)
	}

	if got := defaultServerURL(); got != "" {
		t.Fatalf("defaultServerURL() = %q, want empty", got)
	}
}

func TestNormalizeServerURLAcceptsMCPEndpoint(t *testing.T) {
	got := normalizeServerURL(" http://127.0.0.1:9115/mcp ")
	if got != "http://127.0.0.1:9115" {
		t.Fatalf("normalizeServerURL() = %q, want %q", got, "http://127.0.0.1:9115")
	}
}

func TestValidateServerURLExplainsManualInit(t *testing.T) {
	_, err := validateServerURL("")
	if err == nil {
		t.Fatal("validateServerURL() error = nil, want setup instructions")
	}
	msg := err.Error()
	if !strings.Contains(msg, "browseros-cli init <Server URL>") {
		t.Fatalf("validateServerURL() error = %q, want manual init instructions", msg)
	}
	if strings.Contains(msg, "init --auto") {
		t.Fatalf("validateServerURL() error = %q, should not mention init --auto", msg)
	}
}

func TestDrainAutomaticUpdateCheckWithTimeoutWaitsForCompletion(t *testing.T) {
	done := make(chan struct{})
	returned := make(chan struct{})

	go func() {
		drainAutomaticUpdateCheckWithTimeout(done, time.Second)
		close(returned)
	}()

	select {
	case <-returned:
		t.Fatal("drainAutomaticUpdateCheckWithTimeout() returned before check completed")
	case <-time.After(10 * time.Millisecond):
	}

	close(done)

	select {
	case <-returned:
	case <-time.After(100 * time.Millisecond):
		t.Fatal("drainAutomaticUpdateCheckWithTimeout() did not return after check completed")
	}
}

func TestDrainAutomaticUpdateCheckWithTimeoutStopsWaiting(t *testing.T) {
	done := make(chan struct{})
	returned := make(chan struct{})

	go func() {
		drainAutomaticUpdateCheckWithTimeout(done, 20*time.Millisecond)
		close(returned)
	}()

	select {
	case <-returned:
		t.Fatal("drainAutomaticUpdateCheckWithTimeout() returned before timeout elapsed")
	case <-time.After(5 * time.Millisecond):
	}

	select {
	case <-returned:
	case <-time.After(100 * time.Millisecond):
		t.Fatal("drainAutomaticUpdateCheckWithTimeout() did not return after timeout")
	}
}
