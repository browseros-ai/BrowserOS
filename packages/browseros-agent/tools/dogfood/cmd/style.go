package cmd

import (
	"fmt"
	"strings"

	"github.com/fatih/color"
	"github.com/spf13/cobra"
)

const (
	groupSetup      = "setup"
	groupRun        = "run"
	groupInspect    = "inspect"
	groupOther      = "other"
	groupOtherTitle = "Other:"
	helpCommandName = "help"
)

var (
	headerStyle  = color.New(color.Bold, color.FgCyan)
	commandStyle = color.New(color.FgHiGreen)
	successStyle = color.New(color.FgGreen, color.Bold)
	warnStyle    = color.New(color.FgYellow, color.Bold)
	labelStyle   = color.New(color.Bold)
	pathStyle    = color.New(color.FgCyan)
	dimStyle     = color.New(color.Faint)
)

func helpHeader(s string) string {
	return headerStyle.Sprint(s)
}

func helpHint(s string) string {
	return dimStyle.Sprint(s)
}

func groupedHelp(cmd *cobra.Command) string {
	var b strings.Builder
	cmds := cmd.Commands()
	groups := cmd.Groups()
	if len(groups) == 0 {
		groups = []*cobra.Group{{ID: groupOther, Title: groupOtherTitle}}
	}

	for _, group := range groups {
		lines := commandLines(cmds, group.ID)
		if len(lines) == 0 {
			continue
		}
		b.WriteString("\n" + helpHeader(group.Title) + "\n")
		for _, line := range lines {
			b.WriteString(line)
		}
	}
	return strings.TrimRight(b.String(), "\n")
}

func commandLines(cmds []*cobra.Command, groupID string) []string {
	lines := []string{}
	for _, c := range cmds {
		commandGroupID := c.GroupID
		if commandGroupID == "" {
			commandGroupID = groupOther
		}
		if commandGroupID != groupID || (!c.IsAvailableCommand() && c.Name() != helpCommandName) {
			continue
		}
		name := commandStyle.Sprint(fmt.Sprintf("%-*s", c.NamePadding(), c.Name()))
		lines = append(lines, fmt.Sprintf("  %s %s\n", name, c.Short))
	}
	return lines
}
