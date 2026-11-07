package cmd

import (
	"fmt"
	"os"

	"browseros-dev/proc"

	"github.com/spf13/cobra"
)

var rootCmd = &cobra.Command{
	Use:   "browseros-dev",
	Short: "BrowserOS neo development CLI",
	Long: proc.BoldColor.Sprint("browseros-dev") + proc.DimColor.Sprint(" runs the BrowserOS neo dev stack.") + `

Supervises the claw-app WXT dev server, its web preview, claw-server, and the browser they
launch, and cleans up their local dev state.`,
	CompletionOptions: cobra.CompletionOptions{DisableDefaultCmd: true},
	SilenceUsage:      true,
	SilenceErrors:     true,
}

func Execute() {
	if err := rootCmd.Execute(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
