package cmd

import (
	"sort"
	"strings"

	"browseros-cli/output"

	"github.com/spf13/cobra"
)

const browserOSInfoOverview = `# BrowserOS

BrowserOS is an open-source AI browser built on Chromium. It exposes local browser automation through an MCP server and lets agents drive tabs, pages, screenshots, files, and connected apps.

Docs: https://docs.browseros.com/`

var browserOSInfoTopics = map[string]string{
	"overview":           browserOSInfoOverview,
	"mcp-server":         "BrowserOS exposes a local MCP server for browser automation. Use the CLI against the server URL from BrowserOS Settings > BrowserOS MCP.",
	"filesystem-access":  "BrowserOS agents can use scoped filesystem tools when a workspace is selected.",
	"connect-apps":       "BrowserOS can connect external apps through managed MCP integrations.",
	"bring-your-own-llm": "Connect BrowserOS to your preferred LLM provider or local model from BrowserOS settings.",
	"scheduled-tasks":    "Scheduled Tasks run BrowserOS automations on a recurring schedule while BrowserOS is open.",
	"chat-hub":           "Chat and LLM Hub provide side-panel AI chat and model comparison across webpages.",
	"ad-blocking":        "BrowserOS includes built-in ad blocking powered by uBlock Origin.",
}

func init() {
	cmd := &cobra.Command{
		Use:         "info [topic]",
		Annotations: map[string]string{"group": "Setup:"},
		Short:       "Get information about BrowserOS features",
		Args:        cobra.MaximumNArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			topic := "overview"
			if len(args) > 0 {
				topic = args[0]
			}
			content, ok := browserOSInfoTopics[topic]
			if !ok {
				valid := make([]string, 0, len(browserOSInfoTopics))
				for key := range browserOSInfoTopics {
					valid = append(valid, key)
				}
				sort.Strings(valid)
				output.Errorf(3, "unknown topic %q; valid topics: %s", topic, strings.Join(valid, ", "))
			}
			result := textResult(content, map[string]any{
				"topic":   topic,
				"content": content,
			})
			if jsonOut {
				output.JSON(result)
			} else {
				output.Text(result)
			}
		},
	}

	rootCmd.AddCommand(cmd)
}
