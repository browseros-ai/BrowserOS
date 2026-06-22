package cmd

import (
	"browseros-cli/output"

	"github.com/spf13/cobra"
)

func init() {
	navCmd := &cobra.Command{
		Use:         "nav <url>",
		Annotations: map[string]string{"group": "Navigate:"},
		Short:       "Navigate the current page to a URL",
		Args:        cobra.ExactArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			c := newClient()
			pageID, err := resolvePageID(c)
			if err != nil {
				output.Error(err.Error(), 2)
			}
			result, err := c.CallTool("navigate", map[string]any{
				"page":   pageID,
				"action": "url",
				"url":    args[0],
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

	backCmd := &cobra.Command{
		Use:         "back",
		Annotations: map[string]string{"group": "Navigate:"},
		Short:       "Navigate back",
		Args:        cobra.NoArgs,
		Run:         navAction("back"),
	}

	forwardCmd := &cobra.Command{
		Use:         "forward",
		Annotations: map[string]string{"group": "Navigate:"},
		Short:       "Navigate forward",
		Args:        cobra.NoArgs,
		Run:         navAction("forward"),
	}

	reloadCmd := &cobra.Command{
		Use:         "reload",
		Annotations: map[string]string{"group": "Navigate:"},
		Short:       "Reload the current page",
		Args:        cobra.NoArgs,
		Run:         navAction("reload"),
	}

	rootCmd.AddCommand(navCmd, backCmd, forwardCmd, reloadCmd)
}

func navAction(action string) func(*cobra.Command, []string) {
	return func(cmd *cobra.Command, args []string) {
		c := newClient()
		pageID, err := resolvePageID(c)
		if err != nil {
			output.Error(err.Error(), 2)
		}
		result, err := c.CallTool("navigate", map[string]any{
			"page":   pageID,
			"action": action,
		})
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
