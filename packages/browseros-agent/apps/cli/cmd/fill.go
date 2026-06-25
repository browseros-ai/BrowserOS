package cmd

import (
	"strings"

	"browseros-cli/output"

	"github.com/spf13/cobra"
)

func init() {
	fillCmd := &cobra.Command{
		Use:         "fill <ref> <value>",
		Annotations: map[string]string{"group": "Input:"},
		Short:       "Fill an input by snapshot ref",
		Args:        cobra.MinimumNArgs(2),
		Run: func(cmd *cobra.Command, args []string) {
			ref, err := elementRef(args[0])
			if err != nil {
				output.Error(err.Error(), 3)
			}
			value := strings.Join(args[1:], " ")

			pageID, err := resolvePageID(nil)
			if err != nil {
				output.Error(err.Error(), 2)
			}
			c := newClient()

			result, err := c.CallTool("act", fillToolArgsFromCommand(cmd, pageID, ref, value))
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
	fillCmd.Flags().Bool("no-clear", false, "Don't clear existing value before filling")

	clearCmd := &cobra.Command{
		Use:         "clear <ref>",
		Annotations: map[string]string{"group": "Input:"},
		Short:       "Clear an input by snapshot ref",
		Args:        cobra.ExactArgs(1),
		Run:         elementAction("fill", map[string]any{"value": "", "clear": true}),
	}

	pressCmd := &cobra.Command{
		Use:         "press <key>",
		Aliases:     []string{"key"},
		Annotations: map[string]string{"group": "Input:"},
		Short:       "Press a key or key combination (e.g., Enter, Control+A)",
		Args:        cobra.ExactArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			pageID, err := resolvePageID(nil)
			if err != nil {
				output.Error(err.Error(), 2)
			}
			c := newClient()
			result, err := c.CallTool("act", pressToolArgs(pageID, args[0]))
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

	typeCmd := &cobra.Command{
		Use:         "type <text>",
		Annotations: map[string]string{"group": "Input:"},
		Short:       "Type text into the focused element",
		Args:        cobra.MinimumNArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			pageID, err := resolvePageID(nil)
			if err != nil {
				output.Error(err.Error(), 2)
			}
			c := newClient()
			result, err := c.CallTool("act", typeToolArgs(pageID, strings.Join(args, " ")))
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

	rootCmd.AddCommand(fillCmd, clearCmd, pressCmd, typeCmd)
}

// fillToolArgsFromCommand converts parsed fill flags into the compact act payload.
func fillToolArgsFromCommand(cmd *cobra.Command, pageID int, ref, value string) map[string]any {
	noClear, _ := cmd.Flags().GetBool("no-clear")
	return fillToolArgs(pageID, ref, value, !noClear)
}

func fillToolArgs(pageID int, ref, value string, clear bool) map[string]any {
	return map[string]any{
		"page":  pageID,
		"kind":  "fill",
		"ref":   ref,
		"value": value,
		"clear": clear,
	}
}

func pressToolArgs(pageID int, key string) map[string]any {
	return map[string]any{
		"page": pageID,
		"kind": "press",
		"key":  key,
	}
}

func typeToolArgs(pageID int, text string) map[string]any {
	return map[string]any{
		"page": pageID,
		"kind": "type",
		"text": text,
	}
}
