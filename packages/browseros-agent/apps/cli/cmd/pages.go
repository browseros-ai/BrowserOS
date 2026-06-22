package cmd

import (
	"fmt"

	"browseros-cli/output"

	"github.com/spf13/cobra"
)

func init() {
	pagesCmd := &cobra.Command{
		Use:         "pages",
		Annotations: map[string]string{"group": "Navigate:"},
		Short:       "List all open pages (tabs)",
		Args:        cobra.NoArgs,
		Run: func(cmd *cobra.Command, args []string) {
			c := newClient()
			result, err := c.CallTool("tabs", map[string]any{"action": "list"})
			if err != nil {
				output.Error(err.Error(), 1)
			}
			if jsonOut {
				output.JSON(result)
			} else {
				output.PageList(result)
			}
		},
	}

	activeCmd := &cobra.Command{
		Use:         "active",
		Annotations: map[string]string{"group": "Navigate:"},
		Short:       "Show the active (focused) page",
		Args:        cobra.NoArgs,
		Run: func(cmd *cobra.Command, args []string) {
			c := newClient()
			result, err := c.CallTool("tabs", map[string]any{"action": "list"})
			if err != nil {
				output.Error(err.Error(), 1)
			}
			if jsonOut {
				output.JSON(result)
			} else {
				output.ActivePage(result)
			}
		},
	}

	closeCmd := &cobra.Command{
		Use:         "close [pageId]",
		Annotations: map[string]string{"group": "Navigate:"},
		Short:       "Close a page (tab)",
		Args:        cobra.MaximumNArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			c := newClient()
			var pageID int
			var err error
			if len(args) > 0 {
				_, err = fmt.Sscanf(args[0], "%d", &pageID)
				if err != nil {
					output.Errorf(3, "invalid page ID: %s", args[0])
				}
			} else {
				pageID, err = resolvePageID(c)
				if err != nil {
					output.Error(err.Error(), 2)
				}
			}
			result, err := c.CallTool("tabs", map[string]any{
				"action": "close",
				"page":   pageID,
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

	rootCmd.AddCommand(pagesCmd, activeCmd, closeCmd)
}
