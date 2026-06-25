package cmd

import (
	"errors"

	"browseros-cli/output"

	"github.com/spf13/cobra"
)

func init() {
	readCmd := &cobra.Command{
		Use:         "read",
		Annotations: map[string]string{"group": "Observe:"},
		Short:       "Read page content as markdown, text, or links",
		Args:        cobra.NoArgs,
		Run: func(cmd *cobra.Command, args []string) {
			opts, err := readOptionsFromCommand(cmd)
			if err != nil {
				output.Error(err.Error(), 3)
			}
			pageID, err := resolvePageID(nil)
			if err != nil {
				output.Error(err.Error(), 2)
			}
			c := newClient()
			result, err := c.CallTool("read", readToolArgs(pageID, opts))
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
	addReadFlags(readCmd, true)

	textCmd := &cobra.Command{
		Use:         "text",
		Annotations: map[string]string{"group": "Observe:"},
		Short:       "Extract page content as markdown",
		Args:        cobra.NoArgs,
		Run: func(cmd *cobra.Command, args []string) {
			pageID, err := resolvePageID(nil)
			if err != nil {
				output.Error(err.Error(), 2)
			}
			c := newClient()

			opts := legacyTextReadOptions(cmd)
			result, err := c.CallTool("read", readToolArgs(pageID, opts))
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
			pageID, err := resolvePageID(nil)
			if err != nil {
				output.Error(err.Error(), 2)
			}
			c := newClient()
			result, err := c.CallTool("read", readToolArgs(pageID, readOptions{format: "links"}))
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

	grepCmd := &cobra.Command{
		Use:         "grep <pattern>",
		Annotations: map[string]string{"group": "Observe:"},
		Short:       "Search snapshot or page text without dumping the page",
		Args:        cobra.ExactArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			content, _ := cmd.Flags().GetBool("content")
			limit, _ := cmd.Flags().GetInt("limit")
			if err := validateChangedIntMinimum("--limit", limit, cmd.Flags().Changed("limit"), 1); err != nil {
				output.Error(err.Error(), 3)
			}
			over := "ax"
			if content {
				over = "content"
			}
			pageID, err := resolvePageID(nil)
			if err != nil {
				output.Error(err.Error(), 2)
			}
			c := newClient()
			result, err := c.CallTool("grep", grepToolArgs(pageID, args[0], over, limit))
			if err != nil {
				output.Error(err.Error(), 1)
			}
			if jsonOut {
				output.JSON(result)
			} else {
				output.Text(textResult(displayElementRefs(result.TextContent()), result.StructuredContent))
			}
		},
	}
	grepCmd.Flags().Bool("content", false, "Search visible page text instead of the accessibility tree")
	grepCmd.Flags().Int("limit", 0, "Maximum matches to return")

	rootCmd.AddCommand(readCmd, textCmd, linksCmd, grepCmd)
}

type readOptions struct {
	format        string
	selector      string
	viewportOnly  bool
	includeLinks  bool
	includeImages bool
}

func addReadFlags(cmd *cobra.Command, formats bool) {
	if formats {
		cmd.Flags().Bool("md", false, "Read markdown")
		cmd.Flags().Bool("text", false, "Read plain text")
		cmd.Flags().Bool("links", false, "Read links")
	}
	cmd.Flags().String("selector", "", "CSS selector to scope extraction")
	cmd.Flags().Bool("viewport", false, "Only visible content")
	cmd.Flags().Bool("include-links", false, "Render links in markdown")
	cmd.Flags().Bool("images", false, "Include image references")
}

func readOptionsFromCommand(cmd *cobra.Command) (readOptions, error) {
	md, _ := cmd.Flags().GetBool("md")
	plainText, _ := cmd.Flags().GetBool("text")
	links, _ := cmd.Flags().GetBool("links")
	if selected := boolCount(md, plainText, links); selected > 1 {
		return readOptions{}, outputFormatError()
	}

	format := "markdown"
	if plainText {
		format = "text"
	} else if links {
		format = "links"
	}

	selector, _ := cmd.Flags().GetString("selector")
	viewport, _ := cmd.Flags().GetBool("viewport")
	includeLinks, _ := cmd.Flags().GetBool("include-links")
	images, _ := cmd.Flags().GetBool("images")
	return readOptions{
		format:        format,
		selector:      selector,
		viewportOnly:  viewport,
		includeLinks:  includeLinks,
		includeImages: images,
	}, nil
}

func legacyTextReadOptions(cmd *cobra.Command) readOptions {
	selector, _ := cmd.Flags().GetString("selector")
	viewport, _ := cmd.Flags().GetBool("viewport")
	links, _ := cmd.Flags().GetBool("links")
	images, _ := cmd.Flags().GetBool("images")
	return readOptions{
		format:        "markdown",
		selector:      selector,
		viewportOnly:  viewport,
		includeLinks:  links,
		includeImages: images,
	}
}

func readToolArgs(pageID int, opts readOptions) map[string]any {
	format := opts.format
	if format == "" {
		format = "markdown"
	}
	toolArgs := map[string]any{
		"page":   pageID,
		"format": format,
	}
	if opts.selector != "" {
		toolArgs["selector"] = opts.selector
	}
	if opts.viewportOnly {
		toolArgs["viewportOnly"] = true
	}
	if opts.includeLinks {
		toolArgs["includeLinks"] = true
	}
	if opts.includeImages {
		toolArgs["includeImages"] = true
	}
	return toolArgs
}

func grepToolArgs(pageID int, pattern, over string, limit int) map[string]any {
	if over == "" {
		over = "ax"
	}
	toolArgs := map[string]any{
		"page":    pageID,
		"pattern": pattern,
		"over":    over,
	}
	if limit > 0 {
		toolArgs["limit"] = limit
	}
	return toolArgs
}

func boolCount(values ...bool) int {
	count := 0
	for _, value := range values {
		if value {
			count++
		}
	}
	return count
}

func outputFormatError() error {
	return errors.New("choose only one of --md, --text, or --links")
}
