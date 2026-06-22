package cmd

import (
	"browseros-cli/output"

	"github.com/spf13/cobra"
)

func init() {
	cmd := &cobra.Command{
		Use:         "open <url>",
		Annotations: map[string]string{"group": "Navigate:"},
		Short:       "Open a new page (tab) and navigate to a URL",
		Args:        cobra.ExactArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			hidden, _ := cmd.Flags().GetBool("hidden")
			bg, _ := cmd.Flags().GetBool("bg")
			windowID, _ := cmd.Flags().GetInt("window")

			c := newClient()
			toolArgs := map[string]any{
				"action":     "new",
				"url":        args[0],
				"hidden":     hidden,
				"background": bg,
			}
			if cmd.Flags().Changed("window") {
				toolArgs["windowId"] = windowID
			}

			result, err := c.CallTool("tabs", toolArgs)
			if err != nil {
				output.Error(err.Error(), 1)
			}
			if jsonOut {
				output.JSON(result)
			} else {
				output.Confirm(result.TextContent())
			}
		},
	}

	cmd.Flags().Bool("hidden", false, "Open as hidden tab")
	cmd.Flags().Bool("bg", false, "Open in background")
	cmd.Flags().Int("window", 0, "Window ID to open in")

	rootCmd.AddCommand(cmd)
}
