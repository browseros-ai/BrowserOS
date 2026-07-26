package cmd

import (
	"fmt"

	"browseros-cli/output"

	"github.com/spf13/cobra"
)

func init() {
	groupCmd := &cobra.Command{
		Use:         "group",
		Annotations: map[string]string{"group": "Resources:"},
		Short:       "Manage tab groups",
	}

	listCmd := &cobra.Command{
		Use:   "list",
		Short: "List all tab groups",
		Args:  cobra.NoArgs,
		Run: func(cmd *cobra.Command, args []string) {
			c := newClient()
			result, err := c.CallTool("tab_groups", map[string]any{"action": "list"})
			if err != nil {
				output.Error(err.Error(), 1)
			}
			if jsonOut {
				output.JSON(result)
			} else {
				output.Text(result)
			}
		},
	}

	createCmd := &cobra.Command{
		Use:   "create <pageId...>",
		Short: "Group pages together",
		Args:  cobra.MinimumNArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			title, _ := cmd.Flags().GetString("title")

			pageIDs := make([]int, 0, len(args))
			for _, a := range args {
				var id int
				if _, err := fmt.Sscanf(a, "%d", &id); err != nil {
					output.Errorf(3, "invalid page ID: %s", a)
				}
				pageIDs = append(pageIDs, id)
			}

			toolArgs := map[string]any{
				"action": "create",
				"pages":  pageIDs,
			}
			if title != "" {
				toolArgs["title"] = title
			}

			c := newClient()
			result, err := c.CallTool("tab_groups", toolArgs)
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
	createCmd.Flags().String("title", "", "Group title")

	updateCmd := &cobra.Command{
		Use:   "update <groupId>",
		Short: "Update tab group properties",
		Args:  cobra.ExactArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			title, _ := cmd.Flags().GetString("title")
			color, _ := cmd.Flags().GetString("color")
			collapsed, _ := cmd.Flags().GetBool("collapsed")

			toolArgs := map[string]any{
				"action":  "update",
				"groupId": args[0],
			}
			if title != "" {
				toolArgs["title"] = title
			}
			if color != "" {
				toolArgs["color"] = color
			}
			if cmd.Flags().Changed("collapsed") {
				toolArgs["collapsed"] = collapsed
			}

			c := newClient()
			result, err := c.CallTool("tab_groups", toolArgs)
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
	updateCmd.Flags().String("title", "", "Group title")
	updateCmd.Flags().String("color", "", "Group color (grey,blue,red,yellow,green,pink,purple,cyan,orange)")
	updateCmd.Flags().Bool("collapsed", false, "Collapse the group")

	ungroupCmd := &cobra.Command{
		Use:   "ungroup <pageId...>",
		Short: "Remove pages from their groups",
		Args:  cobra.MinimumNArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			pageIDs := make([]int, 0, len(args))
			for _, a := range args {
				var id int
				if _, err := fmt.Sscanf(a, "%d", &id); err != nil {
					output.Errorf(3, "invalid page ID: %s", a)
				}
				pageIDs = append(pageIDs, id)
			}

			c := newClient()
			result, err := c.CallTool("tab_groups", map[string]any{
				"action": "ungroup",
				"pages":  pageIDs,
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

	closeCmd := &cobra.Command{
		Use:   "close <groupId>",
		Short: "Close a tab group and all its tabs",
		Args:  cobra.ExactArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			c := newClient()
			result, err := c.CallTool("tab_groups", map[string]any{
				"action":  "close",
				"groupId": args[0],
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

	groupCmd.AddCommand(listCmd, createCmd, updateCmd, ungroupCmd, closeCmd)
	rootCmd.AddCommand(groupCmd)
}
