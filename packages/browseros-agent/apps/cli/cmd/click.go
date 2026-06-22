package cmd

import (
	"fmt"

	"browseros-cli/output"

	"github.com/spf13/cobra"
)

func init() {
	clickCmd := &cobra.Command{
		Use:         "click <element>",
		Annotations: map[string]string{"group": "Input:"},
		Short:       "Click an element by snapshot ID",
		Args:        cobra.ExactArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			var element int
			if _, err := fmt.Sscanf(args[0], "%d", &element); err != nil {
				output.Errorf(3, "invalid element ID: %s", args[0])
			}

			right, _ := cmd.Flags().GetBool("right")
			middle, _ := cmd.Flags().GetBool("middle")
			double, _ := cmd.Flags().GetBool("double")

			button := "left"
			if right {
				button = "right"
			} else if middle {
				button = "middle"
			}
			clickCount := 1
			if double {
				clickCount = 2
			}

			c := newClient()
			pageID, err := resolvePageID(c)
			if err != nil {
				output.Error(err.Error(), 2)
			}

			result, err := c.CallTool("act", map[string]any{
				"page":       pageID,
				"kind":       "click",
				"ref":        elementRef(element),
				"button":     button,
				"clickCount": clickCount,
			})
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

	clickCmd.Flags().Bool("right", false, "Right-click")
	clickCmd.Flags().Bool("middle", false, "Middle-click")
	clickCmd.Flags().Bool("double", false, "Double-click")

	clickAtCmd := &cobra.Command{
		Use:         "click-at <x> <y>",
		Annotations: map[string]string{"group": "Input:"},
		Short:       "Click at specific coordinates",
		Args:        cobra.ExactArgs(2),
		Run: func(cmd *cobra.Command, args []string) {
			var x, y int
			if _, err := fmt.Sscanf(args[0], "%d", &x); err != nil {
				output.Errorf(3, "invalid x coordinate: %s", args[0])
			}
			if _, err := fmt.Sscanf(args[1], "%d", &y); err != nil {
				output.Errorf(3, "invalid y coordinate: %s", args[1])
			}

			c := newClient()
			pageID, err := resolvePageID(c)
			if err != nil {
				output.Error(err.Error(), 2)
			}

			result, err := c.CallTool("act", map[string]any{
				"page": pageID,
				"kind": "click_at",
				"x":    x,
				"y":    y,
			})
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

	rootCmd.AddCommand(clickCmd, clickAtCmd)
}
