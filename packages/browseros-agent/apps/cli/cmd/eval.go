package cmd

import (
	"strings"

	"browseros-cli/output"

	"github.com/spf13/cobra"
)

func init() {
	cmd := &cobra.Command{
		Use:         "eval <expression>",
		Annotations: map[string]string{"group": "Observe:"},
		Short:       "Execute JavaScript in the page context",
		Args:        cobra.MinimumNArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			expression := strings.Join(args, " ")
			c := newClient()
			pageID, err := resolvePageID(c)
			if err != nil {
				output.Error(err.Error(), 2)
			}
			result, err := c.CallTool("evaluate", map[string]any{
				"page": pageID,
				"code": "return (" + expression + ")",
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

	rootCmd.AddCommand(cmd)
}
