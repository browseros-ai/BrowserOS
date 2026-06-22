package cmd

import (
	"browseros-cli/output"

	"github.com/spf13/cobra"
)

func init() {
	cmd := &cobra.Command{
		Use:         "wait",
		Annotations: map[string]string{"group": "Observe:"},
		Short:       "Wait for text or selector to appear on the page",
		Args:        cobra.NoArgs,
		Run: func(cmd *cobra.Command, args []string) {
			text, _ := cmd.Flags().GetString("text")
			selector, _ := cmd.Flags().GetString("selector")
			waitTimeout, _ := cmd.Flags().GetInt("wait-timeout")

			if text == "" && selector == "" {
				output.Errorf(3, "provide --text or --selector")
			}

			c := newClient()
			pageID, err := resolvePageID(c)
			if err != nil {
				output.Error(err.Error(), 2)
			}

			toolArgs := map[string]any{
				"page":    pageID,
				"timeout": waitTimeout,
			}
			if text != "" {
				toolArgs["for"] = "text"
				toolArgs["value"] = text
			}
			if selector != "" {
				toolArgs["for"] = "selector"
				toolArgs["value"] = selector
			}

			result, err := c.CallTool("wait", toolArgs)
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

	cmd.Flags().String("text", "", "Text to wait for")
	cmd.Flags().String("selector", "", "CSS selector to wait for")
	cmd.Flags().Int("wait-timeout", 10000, "Timeout in milliseconds")
	rootCmd.AddCommand(cmd)
}
