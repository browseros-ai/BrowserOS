package cmd

import (
	"bytes"
	"regexp"
	"strings"
	"testing"

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
		"browseros-dogfood init",
		"browseros-dogfood start",
		"start",
		"Start dogfooding environment",
		"Use \"browseros-dogfood [command] --help\" for more information.",
	} {
		if !strings.Contains(usage, want) {
			t.Fatalf("missing %q in\n%s", want, usage)
		}
	}
}

func TestRootAcceptsClawAsHiddenNoOp(t *testing.T) {
	t.Cleanup(func() {
		rootCmd.SetOut(nil)
		rootCmd.SetArgs(nil)
		_ = rootCmd.PersistentFlags().Set("claw", "false")
		_ = statusCmd.Flags().Set("help", "false")
	})
	for _, flag := range []string{"--claw", "-c"} {
		var out bytes.Buffer
		rootCmd.SetOut(&out)
		rootCmd.SetArgs([]string{flag, "status", "--help"})

		if err := rootCmd.Execute(); err != nil {
			t.Fatalf("%s should still parse: %v", flag, err)
		}
		if !strings.Contains(out.String(), "browseros-dogfood status") {
			t.Fatalf("%s: expected status help, got:\n%s", flag, out.String())
		}
	}
	if usage := rootCmd.UsageString(); strings.Contains(usage, "--claw") {
		t.Fatalf("--claw should stay out of root usage:\n%s", usage)
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
