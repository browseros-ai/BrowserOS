package cmd

import (
	"encoding/json"
	"fmt"
	"os"
	"strings"

	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/app"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/ui"
	"github.com/spf13/cobra"
)

var Version = "dev"

var (
	jsonOut  bool
	verbose  bool
	appState *app.App
)

var groupOrder = []string{
	"Workspace:",
	"Core:",
	"Conflict:",
	"Remote:",
}

func helpHeader(s string) string { return ui.Header(s) }
func helpCmdCol(s string) string { return ui.Command(s) }
func helpHint(s string) string   { return ui.Hint(s) }
func helpAliases(aliases []string) string {
	return ui.Aliases(aliases)
}

func groupedHelp(cmd *cobra.Command) string {
	groups := map[string][]*cobra.Command{}
	for _, child := range cmd.Commands() {
		if !child.IsAvailableCommand() && child.Name() != "help" {
			continue
		}
		group := child.Annotations["group"]
		if group == "" {
			group = "Core:"
		}
		groups[group] = append(groups[group], child)
	}
	var builder strings.Builder
	for _, group := range groupOrder {
		commands, ok := groups[group]
		if !ok {
			continue
		}
		builder.WriteString("\n" + helpHeader(group) + "\n")
		for _, child := range commands {
			line := "  " + helpCmdCol(fmt.Sprintf("%-14s", child.Name())) + " " + child.Short
			if len(child.Aliases) > 0 {
				line += " " + helpAliases(child.Aliases)
			}
			builder.WriteString(line + "\n")
		}
	}
	return builder.String()
}

const usageTemplate = `{{helpHeader "Usage:"}}{{if .Runnable}}
  {{.UseLine}}{{end}}{{if .HasAvailableSubCommands}}
  {{.CommandPath}} [command]{{end}}{{if gt (len .Aliases) 0}}

{{helpHeader "Aliases:"}}
  {{.NameAndAliases}}{{end}}{{if .HasExample}}

{{helpHeader "Examples:"}}
{{.Example}}{{end}}{{if .HasAvailableSubCommands}}
{{groupedHelp .}}{{end}}{{if .HasAvailableLocalFlags}}

{{helpHeader "Flags:"}}
{{.LocalFlags.FlagUsages | trimTrailingWhitespaces}}{{end}}{{if .HasAvailableInheritedFlags}}

{{helpHeader "Global Flags:"}}
{{.InheritedFlags.FlagUsages | trimTrailingWhitespaces}}{{end}}{{if .HasAvailableSubCommands}}

{{helpHint (printf "Use \"%s [command] --help\" for more information." .CommandPath)}}{{end}}
`

var rootCmd = &cobra.Command{
	Use:           "bdev",
	Short:         "Workspace-centric BrowserOS patch tooling for Chromium checkouts",
	Version:       Version,
	SilenceUsage:  true,
	SilenceErrors: true,
	PersistentPreRunE: func(cmd *cobra.Command, args []string) error {
		var err error
		appState, err = app.Load(jsonOut, verbose, "")
		return err
	},
	RunE: func(cmd *cobra.Command, args []string) error {
		return cmd.Help()
	},
}

func init() {
	cobra.AddTemplateFunc("helpHeader", helpHeader)
	cobra.AddTemplateFunc("helpCmdCol", helpCmdCol)
	cobra.AddTemplateFunc("helpAliases", helpAliases)
	cobra.AddTemplateFunc("helpHint", helpHint)
	cobra.AddTemplateFunc("groupedHelp", groupedHelp)
	rootCmd.SetUsageTemplate(usageTemplate)
	rootCmd.PersistentFlags().BoolVar(&jsonOut, "json", false, "Emit JSON output")
	rootCmd.PersistentFlags().BoolVarP(&verbose, "verbose", "v", false, "Enable verbose output")
	rootCmd.CompletionOptions.DisableDefaultCmd = true
}

func Execute() {
	if err := rootCmd.Execute(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func renderResult(data any, human func()) error {
	if jsonOut {
		encoder := json.NewEncoder(os.Stdout)
		encoder.SetIndent("", "  ")
		return encoder.Encode(data)
	}
	human()
	return nil
}
