package cmd

import "github.com/spf13/cobra"

func init() {
	cmd := &cobra.Command{
		Use:         "info [topic]",
		Annotations: map[string]string{"group": "Setup:"},
		Short:       "Get information about BrowserOS features",
		Args:        cobra.MaximumNArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			toolArgs := map[string]any{}
			if len(args) > 0 {
				toolArgs["topic"] = args[0]
			}
			_ = toolArgs
			unsupportedByCurrentMCP("info")
		},
	}

	rootCmd.AddCommand(cmd)
}
