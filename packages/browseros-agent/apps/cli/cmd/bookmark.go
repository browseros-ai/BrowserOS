package cmd

import (
	"fmt"
	"strings"

	"browseros-cli/mcp"
	"browseros-cli/output"

	"github.com/spf13/cobra"
)

func init() {
	bookmarkCmd := &cobra.Command{
		Use:         "bookmark",
		Annotations: map[string]string{"group": "Resources:"},
		Short:       "Manage bookmarks",
	}

	listCmd := &cobra.Command{
		Use:   "list",
		Short: "List all bookmarks",
		Args:  cobra.NoArgs,
		Run: func(cmd *cobra.Command, args []string) {
			c := newClient()
			result, err := runBookmarkCommand(c, "getBookmarks", map[string]any{}, func(value any) *mcp.ToolResult {
				nodes := valueSlice(value)
				return textResult(formatBookmarkList(nodes), map[string]any{
					"bookmarks": nodes,
					"count":     len(nodes),
				})
			})
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
		Use:   "create <title> [url]",
		Short: "Create a bookmark or folder",
		Args:  cobra.RangeArgs(1, 2),
		Run: func(cmd *cobra.Command, args []string) {
			parent, _ := cmd.Flags().GetString("parent")
			toolArgs := map[string]any{"title": args[0]}
			if len(args) > 1 {
				toolArgs["url"] = args[1]
			}
			if parent != "" {
				toolArgs["parentId"] = parent
			}
			c := newClient()
			result, err := runBookmarkCommand(c, "createBookmark", toolArgs, func(value any) *mcp.ToolResult {
				node, _ := valueMap(value)
				title := stringValue(node["title"])
				id := stringValue(node["id"])
				message := fmt.Sprintf("Created folder: %s\nID: %s", title, id)
				if url := stringValue(node["url"]); url != "" {
					message = fmt.Sprintf("Created bookmark: %s\nURL: %s\nID: %s", title, url, id)
				}
				return textResult(message, map[string]any{
					"action":   "create_bookmark",
					"bookmark": node,
				})
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
	createCmd.Flags().String("parent", "", "Parent folder ID")

	removeCmd := &cobra.Command{
		Use:   "remove <id>",
		Short: "Remove a bookmark or folder",
		Args:  cobra.ExactArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			c := newClient()
			id := args[0]
			_, value, err := browserRunValue(c, fmt.Sprintf(
				`await browser.cdp('Bookmarks.removeBookmark', { id: %s })
return { action: 'remove_bookmark', id: %s }`,
				jsLiteral(id),
				jsLiteral(id),
			))
			if err != nil {
				output.Error(err.Error(), 1)
			}
			result := textResult(fmt.Sprintf("Removed bookmark %s", id), resultData(value))
			if jsonOut {
				output.JSON(result)
			} else {
				output.Confirm(result.TextContent())
			}
		},
	}

	updateCmd := &cobra.Command{
		Use:   "update <id>",
		Short: "Update a bookmark",
		Args:  cobra.ExactArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			title, _ := cmd.Flags().GetString("title")
			url, _ := cmd.Flags().GetString("url")
			toolArgs := map[string]any{"id": args[0]}
			if title != "" {
				toolArgs["title"] = title
			}
			if url != "" {
				toolArgs["url"] = url
			}
			c := newClient()
			result, err := runBookmarkCommand(c, "updateBookmark", toolArgs, func(value any) *mcp.ToolResult {
				node, _ := valueMap(value)
				return textResult(
					fmt.Sprintf("Updated bookmark: %s\nID: %s", stringValue(node["title"]), stringValue(node["id"])),
					map[string]any{
						"action":   "update_bookmark",
						"bookmark": node,
					},
				)
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
	updateCmd.Flags().String("title", "", "New title")
	updateCmd.Flags().String("url", "", "New URL")

	moveCmd := &cobra.Command{
		Use:   "move <id>",
		Short: "Move a bookmark to a different folder",
		Args:  cobra.ExactArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			parent, _ := cmd.Flags().GetString("parent")
			index, _ := cmd.Flags().GetInt("index")
			toolArgs := map[string]any{"id": args[0]}
			if parent != "" {
				toolArgs["parentId"] = parent
			}
			if cmd.Flags().Changed("index") {
				toolArgs["index"] = index
			}
			c := newClient()
			result, err := runBookmarkCommand(c, "moveBookmark", toolArgs, func(value any) *mcp.ToolResult {
				node, _ := valueMap(value)
				return textResult(
					fmt.Sprintf("Moved: %s", stringValue(node["title"])),
					map[string]any{
						"action":   "move_bookmark",
						"bookmark": node,
					},
				)
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
	moveCmd.Flags().String("parent", "", "Target parent folder ID")
	moveCmd.Flags().Int("index", 0, "Position index")

	searchCmd := &cobra.Command{
		Use:   "search <query>",
		Short: "Search bookmarks",
		Args:  cobra.ExactArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			c := newClient()
			query := args[0]
			result, err := runBookmarkCommand(c, "searchBookmarks", map[string]any{"query": query}, func(value any) *mcp.ToolResult {
				nodes := valueSlice(value)
				message := fmt.Sprintf("No bookmarks found matching %q.", query)
				if len(nodes) > 0 {
					message = fmt.Sprintf("Found %d bookmarks matching %q:\n\n%s", len(nodes), query, formatBookmarkTree(nodes))
				}
				return textResult(message, map[string]any{
					"query":     query,
					"bookmarks": nodes,
					"count":     len(nodes),
				})
			})
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

	bookmarkCmd.AddCommand(listCmd, createCmd, removeCmd, updateCmd, moveCmd, searchCmd)
	rootCmd.AddCommand(bookmarkCmd)
}

// runBookmarkCommand bridges bookmark subcommands through the compact run tool's CDP escape hatch.
func runBookmarkCommand(c *mcp.Client, method string, params map[string]any, build func(any) *mcp.ToolResult) (*mcp.ToolResult, error) {
	resultExpr := "result.nodes"
	switch method {
	case "searchBookmarks":
		resultExpr = "result.results"
	case "createBookmark", "updateBookmark", "moveBookmark":
		resultExpr = "result.node"
	}
	_, value, err := browserRunValue(c, fmt.Sprintf(
		"const result = await browser.cdp(%s, %s)\nreturn %s",
		jsLiteral("Bookmarks."+method),
		jsLiteral(params),
		resultExpr,
	))
	if err != nil {
		return nil, err
	}
	return build(value), nil
}

func formatBookmarkList(nodes []any) string {
	if len(nodes) == 0 {
		return "No bookmarks found."
	}
	return fmt.Sprintf("Found %d bookmarks:\n\n%s", len(nodes), formatBookmarkTree(nodes))
}

func formatBookmarkTree(nodes []any) string {
	lines := make([]string, 0, len(nodes)*2)
	for _, item := range nodes {
		node, ok := valueMap(item)
		if !ok {
			continue
		}
		id := stringValue(node["id"])
		title := stringValue(node["title"])
		if stringValue(node["type"]) == "folder" {
			lines = append(lines, fmt.Sprintf("[%s] %s (folder)", id, title))
			continue
		}
		lines = append(lines, fmt.Sprintf("[%s] %s", id, title))
		if url := stringValue(node["url"]); url != "" {
			lines = append(lines, "    "+url)
		}
	}
	return strings.Join(lines, "\n")
}
