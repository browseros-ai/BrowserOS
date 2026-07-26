package cmd

import (
	"strings"

	"browseros-cli/output"

	"github.com/spf13/cobra"
)

func init() {
	hoverCmd := &cobra.Command{
		Use:         "hover <element>",
		Annotations: map[string]string{"group": "Input:"},
		Short:       "Hover over an element",
		Args:        cobra.ExactArgs(1),
		Run:         elementAction("hover", nil),
	}

	focusCmd := &cobra.Command{
		Use:         "focus <element>",
		Annotations: map[string]string{"group": "Input:"},
		Short:       "Focus an element",
		Args:        cobra.ExactArgs(1),
		Run:         elementAction("focus", nil),
	}

	checkCmd := &cobra.Command{
		Use:         "check <element>",
		Annotations: map[string]string{"group": "Input:"},
		Short:       "Check a checkbox or radio button",
		Args:        cobra.ExactArgs(1),
		Run:         elementAction("check", nil),
	}

	uncheckCmd := &cobra.Command{
		Use:         "uncheck <element>",
		Annotations: map[string]string{"group": "Input:"},
		Short:       "Uncheck a checkbox",
		Args:        cobra.ExactArgs(1),
		Run:         elementAction("uncheck", nil),
	}

	selectCmd := &cobra.Command{
		Use:         "select <element> <value>",
		Annotations: map[string]string{"group": "Input:"},
		Short:       "Select a dropdown option",
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
			result, err := c.CallTool("act", map[string]any{
				"page":  pageID,
				"kind":  "select",
				"ref":   ref,
				"value": value,
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

	dragCmd := &cobra.Command{
		Use:         "drag <source> --to <target>",
		Annotations: map[string]string{"group": "Input:"},
		Short:       "Drag from one element to another",
		Args:        cobra.ExactArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			sourceRef, err := elementRef(args[0])
			if err != nil {
				output.Error(err.Error(), 3)
			}
			target, _ := cmd.Flags().GetString("to")
			targetRef, err := elementRef(target)
			if err != nil {
				output.Error(err.Error(), 3)
			}

			pageID, err := resolvePageID(nil)
			if err != nil {
				output.Error(err.Error(), 2)
			}
			c := newClient()
			result, err := c.CallTool("act", map[string]any{
				"page":      pageID,
				"kind":      "drag",
				"ref":       sourceRef,
				"targetRef": targetRef,
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
	dragCmd.Flags().String("to", "", "Target element ID or ref")
	_ = dragCmd.MarkFlagRequired("to")

	uploadCmd := &cobra.Command{
		Use:         "upload <element> <file...>",
		Annotations: map[string]string{"group": "Input:"},
		Short:       "Upload files to a file input",
		Args:        cobra.MinimumNArgs(2),
		Run: func(cmd *cobra.Command, args []string) {
			ref, err := elementRef(args[0])
			if err != nil {
				output.Error(err.Error(), 3)
			}

			pageID, err := resolvePageID(nil)
			if err != nil {
				output.Error(err.Error(), 2)
			}
			c := newClient()
			result, err := c.CallTool("upload", map[string]any{
				"page":  pageID,
				"ref":   ref,
				"files": args[1:],
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

	rootCmd.AddCommand(hoverCmd, focusCmd, checkCmd, uncheckCmd, selectCmd, dragCmd, uploadCmd)
}

func elementAction(kind string, extra map[string]any) func(*cobra.Command, []string) {
	return func(cmd *cobra.Command, args []string) {
		ref, err := elementRef(args[0])
		if err != nil {
			output.Error(err.Error(), 3)
		}

		pageID, err := resolvePageID(nil)
		if err != nil {
			output.Error(err.Error(), 2)
		}
		c := newClient()

		toolArgs := map[string]any{
			"page": pageID,
			"kind": kind,
			"ref":  ref,
		}
		for key, value := range extra {
			toolArgs[key] = value
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
	}
}
