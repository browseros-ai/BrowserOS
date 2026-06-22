package cmd

import (
	"browseros-cli/output"

	"github.com/spf13/cobra"
)

func init() {
	textCmd := &cobra.Command{
		Use:         "text",
		Annotations: map[string]string{"group": "Observe:"},
		Short:       "Extract page content as markdown",
		Args:        cobra.NoArgs,
		Run: func(cmd *cobra.Command, args []string) {
			selector, _ := cmd.Flags().GetString("selector")
			viewport, _ := cmd.Flags().GetBool("viewport")
			links, _ := cmd.Flags().GetBool("links")
			images, _ := cmd.Flags().GetBool("images")

			c := newClient()
			pageID, err := resolvePageID(c)
			if err != nil {
				output.Error(err.Error(), 2)
			}

			toolArgs := map[string]any{
				"page":   pageID,
				"format": "markdown",
			}
			_ = viewport
			_ = images
			if links {
				toolArgs["format"] = "links"
			}
			if selector != "" {
				toolArgs["selector"] = selector
			}

			result, err := c.CallTool("read", toolArgs)
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

	textCmd.Flags().String("selector", "", "CSS selector to scope extraction")
	textCmd.Flags().Bool("viewport", false, "Only visible content")
	textCmd.Flags().Bool("links", false, "Include links as [text](url)")
	textCmd.Flags().Bool("images", false, "Include image references")

	linksCmd := &cobra.Command{
		Use:         "links",
		Annotations: map[string]string{"group": "Observe:"},
		Short:       "Extract all links from the page",
		Args:        cobra.NoArgs,
		Run: func(cmd *cobra.Command, args []string) {
			c := newClient()
			pageID, err := resolvePageID(c)
			if err != nil {
				output.Error(err.Error(), 2)
			}
			result, err := c.CallTool("read", map[string]any{
				"page":   pageID,
				"format": "links",
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

	rootCmd.AddCommand(textCmd, linksCmd)
}
