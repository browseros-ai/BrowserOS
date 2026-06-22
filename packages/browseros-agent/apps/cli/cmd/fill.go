package cmd

import (
	"fmt"
	"strings"

	"browseros-cli/output"

	"github.com/spf13/cobra"
)

func init() {
	fillCmd := &cobra.Command{
		Use:         "fill <element> <text>",
		Annotations: map[string]string{"group": "Input:"},
		Short:       "Type text into an input element",
		Args:        cobra.MinimumNArgs(2),
		Run: func(cmd *cobra.Command, args []string) {
			var element int
			if _, err := fmt.Sscanf(args[0], "%d", &element); err != nil {
				output.Errorf(3, "invalid element ID: %s", args[0])
			}
			text := strings.Join(args[1:], " ")
			noClear, _ := cmd.Flags().GetBool("no-clear")

			c := newClient()
			pageID, err := resolvePageID(c)
			if err != nil {
				output.Error(err.Error(), 2)
			}

			result, err := c.CallTool("act", map[string]any{
				"page":  pageID,
				"kind":  "fill",
				"ref":   elementRef(element),
				"value": text,
				"clear": !noClear,
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
	fillCmd.Flags().Bool("no-clear", false, "Don't clear existing text before typing")

	clearCmd := &cobra.Command{
		Use:         "clear <element>",
		Annotations: map[string]string{"group": "Input:"},
		Short:       "Clear text content of an input element",
		Args:        cobra.ExactArgs(1),
		Run:         elementAction("clear"),
	}

	keyCmd := &cobra.Command{
		Use:         "key <key>",
		Annotations: map[string]string{"group": "Input:"},
		Short:       "Press a key or key combination (e.g., Enter, Control+A)",
		Args:        cobra.ExactArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			c := newClient()
			pageID, err := resolvePageID(c)
			if err != nil {
				output.Error(err.Error(), 2)
			}
			result, err := c.CallTool("act", map[string]any{
				"page": pageID,
				"kind": "press",
				"key":  args[0],
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

	rootCmd.AddCommand(fillCmd, clearCmd, keyCmd)
}
