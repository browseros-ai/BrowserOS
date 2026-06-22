package cmd

import (
	"fmt"
	"os"

	"browseros-cli/output"

	"github.com/spf13/cobra"
)

func init() {
	pdfCmd := &cobra.Command{
		Use:         "pdf <path>",
		Annotations: map[string]string{"group": "Observe:"},
		Short:       "Save the current page as PDF",
		Args:        cobra.ExactArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			c := newClient()
			pageID, err := resolvePageID(c)
			if err != nil {
				output.Error(err.Error(), 2)
			}
			result, err := c.CallTool("pdf", map[string]any{
				"page": pageID,
			})
			if err != nil {
				output.Error(err.Error(), 1)
			}
			if path, ok := result.StructuredContent["path"].(string); ok && path != args[0] {
				data, err := os.ReadFile(path)
				if err != nil {
					output.Errorf(1, "read generated PDF: %s", err)
				}
				if err := os.WriteFile(args[0], data, 0644); err != nil {
					output.Errorf(1, "write PDF: %s", err)
				}
			}
			if jsonOut {
				output.JSON(result)
			} else {
				output.Confirm("PDF saved: " + args[0])
			}
		},
	}

	downloadCmd := &cobra.Command{
		Use:         "download <element> <dir>",
		Annotations: map[string]string{"group": "Input:"},
		Short:       "Click element to trigger download and save to directory",
		Args:        cobra.ExactArgs(2),
		Run: func(cmd *cobra.Command, args []string) {
			var element int
			if _, err := fmt.Sscanf(args[0], "%d", &element); err != nil {
				output.Errorf(3, "invalid element ID: %s", args[0])
			}

			c := newClient()
			pageID, err := resolvePageID(c)
			if err != nil {
				output.Error(err.Error(), 2)
			}
			_ = args[1]
			result, err := c.CallTool("download", map[string]any{
				"page": pageID,
				"ref":  elementRef(element),
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

	rootCmd.AddCommand(pdfCmd, downloadCmd)
}
