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
			pageID, err := resolvePageID(nil)
			if err != nil {
				output.Error(err.Error(), 2)
			}
			c := newClient()

			result, err := c.CallTool("snapshot", map[string]any{"page": pageID})
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

	rootCmd.AddCommand(cmd)
}
