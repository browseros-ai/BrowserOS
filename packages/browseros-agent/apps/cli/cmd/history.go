package cmd

import (
	"fmt"
	"strings"
	"time"

	"browseros-cli/mcp"
	"browseros-cli/output"

	"github.com/spf13/cobra"
)

func init() {
	historyCmd := &cobra.Command{
		Use:         "history",
		Annotations: map[string]string{"group": "Resources:"},
		Short:       "Manage browser history",
	}

	searchCmd := &cobra.Command{
		Use:   "search <query>",
		Short: "Search browser history",
		Args:  cobra.ExactArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			max, _ := cmd.Flags().GetInt("max")
			c := newClient()
			toolArgs := map[string]any{"query": args[0]}
			if cmd.Flags().Changed("max") {
				toolArgs["maxResults"] = max
			}
			query := args[0]
			result, err := runHistoryEntries(c, "search", toolArgs, func(items []any) *mcp.ToolResult {
				message := fmt.Sprintf("No history items found matching %q.", query)
				if len(items) > 0 {
					message = fmt.Sprintf("Found %d history items matching %q:\n\n%s", len(items), query, formatHistoryItems(items, true))
				}
				return textResult(message, map[string]any{
					"query": query,
					"items": items,
					"count": len(items),
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
	searchCmd.Flags().Int("max", 0, "Max results")

	recentCmd := &cobra.Command{
		Use:   "recent",
		Short: "Show recent history",
		Args:  cobra.NoArgs,
		Run: func(cmd *cobra.Command, args []string) {
			max, _ := cmd.Flags().GetInt("max")
			c := newClient()
			toolArgs := map[string]any{}
			if cmd.Flags().Changed("max") {
				toolArgs["maxResults"] = max
			}
			result, err := runHistoryEntries(c, "getRecent", toolArgs, func(items []any) *mcp.ToolResult {
				message := "No recent history items."
				if len(items) > 0 {
					message = fmt.Sprintf("Retrieved %d recent history items:\n\n%s", len(items), formatHistoryItems(items, false))
				}
				return textResult(message, map[string]any{
					"items": items,
					"count": len(items),
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
	recentCmd.Flags().Int("max", 0, "Max results")

	deleteCmd := &cobra.Command{
		Use:   "delete <url>",
		Short: "Delete a URL from history",
		Args:  cobra.ExactArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			c := newClient()
			url := args[0]
			_, value, err := browserRunValue(c, fmt.Sprintf(
				`await browser.cdp('History.deleteUrl', { url: %s })
return { action: 'delete_history_url', url: %s }`,
				jsLiteral(url),
				jsLiteral(url),
			))
			if err != nil {
				output.Error(err.Error(), 1)
			}
			result := textResult(fmt.Sprintf("Deleted %s from history", url), resultData(value))
			if jsonOut {
				output.JSON(result)
			} else {
				output.Confirm(result.TextContent())
			}
		},
	}

	deleteRangeCmd := &cobra.Command{
		Use:   "delete-range",
		Short: "Delete history within a time range",
		Args:  cobra.NoArgs,
		Run: func(cmd *cobra.Command, args []string) {
			start, _ := cmd.Flags().GetInt("start")
			end, _ := cmd.Flags().GetInt("end")
			c := newClient()
			_, value, err := browserRunValue(c, fmt.Sprintf(
				`await browser.cdp('History.deleteRange', { startTime: %d, endTime: %d })
return { action: 'delete_history_range', startTime: %d, endTime: %d, startIso: new Date(%d).toISOString(), endIso: new Date(%d).toISOString() }`,
				start,
				end,
				start,
				end,
				start,
				end,
			))
			if err != nil {
				output.Error(err.Error(), 1)
			}
			result := textResult(
				fmt.Sprintf("Deleted history from %s to %s", time.UnixMilli(int64(start)).UTC().Format(time.RFC3339Nano), time.UnixMilli(int64(end)).UTC().Format(time.RFC3339Nano)),
				resultData(value),
			)
			if jsonOut {
				output.JSON(result)
			} else {
				output.Confirm(result.TextContent())
			}
		},
	}
	deleteRangeCmd.Flags().Int("start", 0, "Start time (epoch ms)")
	deleteRangeCmd.Flags().Int("end", 0, "End time (epoch ms)")
	_ = deleteRangeCmd.MarkFlagRequired("start")
	_ = deleteRangeCmd.MarkFlagRequired("end")

	historyCmd.AddCommand(searchCmd, recentCmd, deleteCmd, deleteRangeCmd)
	rootCmd.AddCommand(historyCmd)
}

// runHistoryEntries bridges history subcommands through the compact run tool's CDP escape hatch.
func runHistoryEntries(c *mcp.Client, method string, params map[string]any, build func([]any) *mcp.ToolResult) (*mcp.ToolResult, error) {
	_, value, err := browserRunValue(c, fmt.Sprintf(
		"const result = await browser.cdp(%s, %s)\nreturn result.entries",
		jsLiteral("History."+method),
		jsLiteral(params),
	))
	if err != nil {
		return nil, err
	}
	return build(valueSlice(value)), nil
}

func formatHistoryItems(items []any, includeVisitCount bool) string {
	blocks := make([]string, 0, len(items))
	for _, item := range items {
		entry, ok := valueMap(item)
		if !ok {
			continue
		}
		id := stringValue(entry["id"])
		title := stringValue(entry["title"])
		if title == "" {
			title = "Untitled"
		}
		url := stringValue(entry["url"])
		if url == "" {
			url = "No URL"
		}
		lastVisit := "Unknown date"
		if ms := numberValue(entry["lastVisitTime"]); ms > 0 {
			lastVisit = time.UnixMilli(int64(ms)).UTC().Format(time.RFC3339Nano)
		}
		lines := []string{
			fmt.Sprintf("[%s] %s", id, title),
			"    " + url,
			"    Last visited: " + lastVisit,
		}
		if includeVisitCount {
			if visits := numberValue(entry["visitCount"]); visits > 0 {
				lines = append(lines, fmt.Sprintf("    Visit count: %d", visits))
			}
		}
		blocks = append(blocks, strings.Join(lines, "\n"))
	}
	return strings.Join(blocks, "\n\n")
}
