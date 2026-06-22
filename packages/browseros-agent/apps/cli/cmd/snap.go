package cmd

import (
	"browseros-cli/output"

	"github.com/spf13/cobra"
)

func init() {
	cmd := &cobra.Command{
		Use:         "snap",
		Annotations: map[string]string{"group": "Observe:"},
		Short:       "Snapshot interactive elements on the page",
		Args:        cobra.NoArgs,
		Run: func(cmd *cobra.Command, args []string) {
			enhanced, _ := cmd.Flags().GetBool("enhanced")
			c := newClient()
			pageID, err := resolvePageID(c)
			if err != nil {
				output.Error(err.Error(), 2)
			}

			_ = enhanced

			result, err := c.CallTool("snapshot", map[string]any{"page": pageID})
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

	cmd.Flags().BoolP("enhanced", "e", false, "Detailed accessibility tree with structural context")
	rootCmd.AddCommand(cmd)
}
