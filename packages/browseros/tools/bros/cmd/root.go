package cmd

import (
	"github.com/spf13/cobra"
)

var (
	version string
)

var rootCmd = &cobra.Command{
	Use:           "bros",
	Short:         "BrowserOS CLI — patch management, builds, and releases",
	Long:          "bros manages BrowserOS patches across Chromium checkouts.\nUse push/pull to sync patches, clone for fresh applies.",
	SilenceUsage:  true,
	SilenceErrors: true,
}

func SetVersion(v string) {
	version = v
	rootCmd.Version = v
}

func Execute() error {
	return rootCmd.Execute()
}
