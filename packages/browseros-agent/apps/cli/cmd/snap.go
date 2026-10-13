package cmd

import (
	"browseros-cli/output"

	"github.com/spf13/cobra"
)

func init() {
	cmd := &cobra.Command{
		Use:         "snapshot",
		Aliases:     []string{"snap"},
		Annotations: map[string]string{"group": "Observe:"},
		Short:       "Capture the page accessibility tree",
		Args:        cobra.NoArgs,
		Run: func(cmd *cobra.Command, args []string) {
			interactive, _ := cmd.Flags().GetBool("interactive")
			depth, _ := cmd.Flags().GetInt("depth")
			if depth < 0 {
				output.Error("depth must be 0 or greater", 3)
			}
			pageID, err := resolvePageID(nil)
			if err != nil {
				output.Error(err.Error(), 2)
			}
			c := newClient()

			result, err := c.CallTool("snapshot", snapshotToolArgs(pageID, interactive, depth))
			if err != nil {
				output.Error(err.Error(), 1)
			}
			if jsonOut {
				data := map[string]any{}
				for key, value := range result.StructuredContent {
					data[key] = value
				}
				data["snapshot"] = result.TextContent()
				output.JSONRaw(data)
			} else {
				output.Text(result)
			}
		},
	}

	cmd.Flags().BoolP("interactive", "i", false, "Only actionable elements, plus headings and ancestor context")
	cmd.Flags().IntP("depth", "d", 0, "Maximum tree depth, 1-100 (0 leaves nesting uncapped)")

	rootCmd.AddCommand(cmd)
}

// snapshotToolArgs maps the snapshot flags onto the tool's mode and depth inputs.
// Both are omitted when unset so the server keeps its own defaults.
func snapshotToolArgs(pageID int, interactive bool, depth int) map[string]any {
	toolArgs := map[string]any{"page": pageID}
	if interactive {
		toolArgs["mode"] = "interactive"
	}
	if depth > 0 {
		toolArgs["depth"] = depth
	}
	return toolArgs
}
