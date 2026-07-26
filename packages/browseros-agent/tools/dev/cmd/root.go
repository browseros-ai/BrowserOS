package cmd

import (
	"fmt"
	"os"

	"browseros-dev/proc"

	"github.com/spf13/cobra"
)

var rootCmd = &cobra.Command{
	Use:   "browseros-dev",
	Short: "BrowserOS development & testing CLI",
	Long: proc.BoldColor.Sprint("browseros-dev") + proc.DimColor.Sprint(" — development & testing CLI for BrowserOS") + `

Manages browser, server, and extension processes for local development and testing.`,
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
