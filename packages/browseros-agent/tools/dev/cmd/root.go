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
	Long: proc.BoldColor.Sprint("browseros-dev") + proc.DimColor.Sprint(" — development CLI for BrowserOS neo") + `

Manages the BrowserOS neo app, claw-server, and browser processes for local development.`,
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
