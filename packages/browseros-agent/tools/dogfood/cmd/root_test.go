package cmd

import (
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
		"browseros-dogfood start-background",
		"start",
		"Start dogfooding environment",
		"Use \"browseros-dogfood [command] --help\" for more information.",
	} {
		if !strings.Contains(usage, want) {
			t.Fatalf("missing %q in\n%s", want, usage)
		}
	}
}

func TestRootKeepsClawFlagAsHiddenDeprecatedNoOp(t *testing.T) {
	flag := rootCmd.PersistentFlags().Lookup("claw")
	if flag == nil || flag.Shorthand != "c" {
		t.Fatalf("expected --claw/-c to stay accepted, got %#v", flag)
	}
	if !flag.Hidden || flag.Deprecated == "" {
		t.Fatalf("expected --claw to be hidden and deprecated, got hidden=%v deprecated=%q", flag.Hidden, flag.Deprecated)
	}
	if rootCmd.PersistentFlags().Lookup("browseros") != nil {
		t.Fatal("--browseros should be gone with the BrowserOS dogfood target")
	}
	if strings.Contains(stripANSI(rootCmd.UsageString()), "--claw") {
		t.Fatalf("root usage should not advertise --claw:\n%s", rootCmd.UsageString())
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
