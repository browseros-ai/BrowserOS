package cmd

import (
	"fmt"
	"os"

	"github.com/spf13/cobra"
)

var rootCmd = &cobra.Command{
	Use:   "browseros-dogfood",
	Short: "BrowserOS neo dogfooding CLI",
	Long:  "browseros-dogfood runs BrowserOS neo from an alpha checkout against a copy of your BrowserOS profile.",
	Example: `  browseros-dogfood init
  browseros-dogfood start
  browseros-dogfood start-background
  browseros-dogfood status
  browseros-dogfood logs tail
  browseros-dogfood restart --pull
  browseros-dogfood stop`,
	CompletionOptions: cobra.CompletionOptions{DisableDefaultCmd: true},
	SilenceUsage:      true,
	SilenceErrors:     true,
}

const usageTemplate = `{{helpHeader "Usage:"}}{{if .Runnable}}
  {{.UseLine}}{{end}}{{if .HasAvailableSubCommands}}
  {{.CommandPath}} [command]{{end}}{{if .HasExample}}

{{helpHeader "Examples:"}}
{{.Example}}{{end}}{{if .HasAvailableSubCommands}}
{{groupedHelp .}}{{end}}{{if .HasAvailableLocalFlags}}

{{helpHeader "Flags:"}}
{{.LocalFlags.FlagUsages | trimTrailingWhitespaces}}{{end}}{{if .HasAvailableInheritedFlags}}

{{helpHeader "Global Flags:"}}
{{.InheritedFlags.FlagUsages | trimTrailingWhitespaces}}{{end}}{{if .HasAvailableSubCommands}}

{{helpHint (printf "Use \"%s [command] --help\" for more information." .CommandPath)}}{{end}}
`

func init() {
	cobra.AddTemplateFunc("helpHeader", helpHeader)
	cobra.AddTemplateFunc("helpHint", helpHint)
	cobra.AddTemplateFunc("groupedHelp", groupedHelp)

	rootCmd.AddGroup(
		&cobra.Group{ID: groupSetup, Title: "Setup:"},
		&cobra.Group{ID: groupRun, Title: "Run:"},
		&cobra.Group{ID: groupInspect, Title: "Inspect:"},
		&cobra.Group{ID: groupOther, Title: groupOtherTitle},
	)
	rootCmd.SetHelpCommandGroupID(groupOther)
	rootCmd.SetUsageTemplate(usageTemplate)
	// Existing scripts and docs pass --claw (-c), so the root accepts it as a hidden no-op.
	rootCmd.PersistentFlags().BoolP("claw", "c", false, "Accepted for older scripts and ignored")
	_ = rootCmd.PersistentFlags().MarkHidden("claw")
}

func Execute() {
	if err := rootCmd.Execute(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
