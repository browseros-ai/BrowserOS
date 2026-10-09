package cmd

import (
	"fmt"
	"strings"

	"browseros-cli/mcp"
	"browseros-cli/output"

	"github.com/spf13/cobra"
)

func init() {
	cmd := &cobra.Command{
		Use:         "open <url>",
		Annotations: map[string]string{"group": "Navigate:"},
		Short:       "Open a new page (tab) and navigate to a URL",
		Args:        cobra.ExactArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			bg, _ := cmd.Flags().GetBool("bg")
			windowID, _ := cmd.Flags().GetInt("window")

			c := newClient()
			var err error
			var toolResult *mcp.ToolResult

			if cmd.Flags().Changed("window") {
				var result any
				_, result, err = browserRunValue(c, openInWindowCode(args[0], bg, windowID))
				if err == nil {
					resultData, _ := valueMap(result)
					toolResult = textResult("", resultData)
				}
			} else {
				toolResult, err = c.CallTool("tabs", openTabsToolArgs(args[0], bg))
			}

			if err != nil {
				output.Error(err.Error(), 1)
			}
			resultForOutput := openResult(args[0], toolResult)
			if jsonOut {
				output.JSON(resultForOutput)
			} else {
				output.Confirm(resultForOutput.TextContent())
			}
		},
	}

	cmd.Flags().Bool("bg", false, "Open in background")
	cmd.Flags().Int("window", 0, "Window ID to open in")

	rootCmd.AddCommand(cmd)
}

// openResult presents newly opened pages with a stable CLI page handle.
func openResult(url string, result *mcp.ToolResult) *mcp.ToolResult {
	data := map[string]any{}
	if result != nil {
		for key, value := range result.StructuredContent {
			if key == "pageId" {
				continue
			}
			data[key] = value
		}
		if numberValue(data["page"]) == 0 {
			if pageID := numberValue(result.StructuredContent["pageId"]); pageID != 0 {
				data["page"] = pageID
			}
		}
	}
	if url != "" {
		data["url"] = url
	}

	lines := []string{}
	if pageID := numberValue(data["page"]); pageID != 0 {
		lines = append(lines, fmt.Sprintf("page=%d", pageID))
	}
	if url != "" {
		lines = append(lines, "url="+url)
	}
	if len(lines) == 0 && result != nil {
		lines = append(lines, result.TextContent())
	}
	return textResult(strings.Join(lines, "\n"), data)
}

func openTabsToolArgs(url string, background bool) map[string]any {
	return map[string]any{
		"action":     "new",
		"url":        url,
		"background": background,
	}
}

func openInWindowCode(url string, background bool, windowID int) string {
	return fmt.Sprintf(
		`const page = await browser.pages.newPage(%s, { background: %t, windowId: %d })
return { page, url: %s, background: %t, windowId: %d }`,
		jsLiteral(url),
		background,
		windowID,
		jsLiteral(url),
		background,
		windowID,
	)
}
