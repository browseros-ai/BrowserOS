package cmd

import (
	"regexp"
	"strings"
	"testing"

	"browseros-dogfood/config"

	"github.com/spf13/cobra"
)

var testANSIPattern = regexp.MustCompile(`\x1b\[[0-9;]*m`)

func TestRootUsageUsesCommandGroups(t *testing.T) {
	usage := stripANSI(rootCmd.UsageString())
	for _, want := range []string{
		"Usage:",
		"Setup:",
		"Run:",
		"Inspect:",
		"browseros-dogfood --browseros init",
		"browseros-dogfood --claw start",
		"start",
		"Start dogfooding environment",
		"Use \"browseros-dogfood [command] --help\" for more information.",
	} {
		if !strings.Contains(usage, want) {
			t.Fatalf("missing %q in\n%s", want, usage)
		}
	}
}

func TestResolveTargetFlags(t *testing.T) {
	tests := []struct {
		name        string
		browserOS   bool
		claw        bool
		wantTarget  config.Target
		wantPresent bool
		wantErr     bool
	}{
		{name: "browseros", browserOS: true, wantTarget: config.TargetBrowserOS, wantPresent: true},
		{name: "claw", claw: true, wantTarget: config.TargetClaw, wantPresent: true},
		{name: "none"},
		{name: "both", browserOS: true, claw: true, wantErr: true},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, present, err := resolveTargetFlags(tt.browserOS, tt.claw)
			if tt.wantErr {
				if err == nil {
					t.Fatal("expected error")
				}
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			if got != tt.wantTarget || present != tt.wantPresent {
				t.Fatalf("got target=%q present=%v want target=%q present=%v", got, present, tt.wantTarget, tt.wantPresent)
			}
		})
	}
}

func TestCommandRequiresTargetForLifecycleCommands(t *testing.T) {
	root := &cobra.Command{Use: "browseros-dogfood"}
	logs := &cobra.Command{Use: "logs"}
	tail := &cobra.Command{Use: "tail"}
	configCmd := &cobra.Command{Use: "config"}
	edit := &cobra.Command{Use: "edit"}
	logs.AddCommand(tail)
	configCmd.AddCommand(edit)
	root.AddCommand(logs, configCmd)

	if !commandRequiresTarget(tail) {
		t.Fatal("logs tail should require target")
	}
	if commandRequiresTarget(edit) {
		t.Fatal("config edit should not require target")
	}
}

func TestGroupedHelpUsesOneOtherSectionForUngroupedCommands(t *testing.T) {
	cmd := &cobra.Command{Use: "test"}
	cmd.AddGroup(&cobra.Group{ID: groupOther, Title: groupOtherTitle})
	cmd.AddCommand(&cobra.Command{
		Use:   "orphan",
		Short: "Ungrouped command",
		Run:   func(cmd *cobra.Command, args []string) {},
	})
	cmd.AddCommand(&cobra.Command{
		Use:     "help",
		Short:   "Help about any command",
		GroupID: groupOther,
		Run:     func(cmd *cobra.Command, args []string) {},
	})

	help := stripANSI(groupedHelp(cmd))
	if got := strings.Count(help, "Other:"); got != 1 {
		t.Fatalf("Other section count got %d want 1 in\n%s", got, help)
	}
	for _, want := range []string{"orphan", "Ungrouped command", "help", "Help about any command"} {
		if !strings.Contains(help, want) {
			t.Fatalf("missing %q in\n%s", want, help)
		}
	}
}

func stripANSI(s string) string {
	return testANSIPattern.ReplaceAllString(s, "")
}
