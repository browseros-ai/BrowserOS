package cmd

import (
	"fmt"

	"browseros-cli/output"

	"github.com/spf13/cobra"
)

func init() {
	cmd := &cobra.Command{
		Use:         "scroll <direction> [amount]",
		Annotations: map[string]string{"group": "Input:"},
		Short:       "Scroll the page (up, down, left, right)",
		Args:        cobra.RangeArgs(1, 2),
		Run: func(cmd *cobra.Command, args []string) {
			direction := args[0]
			amount := 3
			if len(args) > 1 {
				if _, err := fmt.Sscanf(args[1], "%d", &amount); err != nil {
					output.Errorf(3, "invalid amount: %s", args[1])
				}
			}
			element, _ := cmd.Flags().GetInt("element")

			c := newClient()
			pageID, err := resolvePageID(c)
			if err != nil {
				output.Error(err.Error(), 2)
			}

			toolArgs := map[string]any{
				"page":      pageID,
				"kind":      "scroll",
				"direction": direction,
				"amount":    amount,
			}
			if cmd.Flags().Changed("element") {
				toolArgs["ref"] = elementRef(element)
			}

			result, err := c.CallTool("act", toolArgs)
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

	cmd.Flags().Int("element", 0, "Element ID to scroll at")
	rootCmd.AddCommand(cmd)
}
