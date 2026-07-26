package cmd

import (
	"fmt"

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
			pageID, err := resolvePageID(nil)
			if err != nil {
				output.Error(err.Error(), 2)
			}
			c := newClient()
			result, err := c.CallTool("pdf", pdfToolArgs(pageID))
			if err != nil {
				output.Error(err.Error(), 1)
			}
			generatedPath := stringValue(result.StructuredContent["path"])
			if generatedPath == "" {
				output.Error("pdf tool did not return a file path", 1)
			}
			if err := copyLocalFile(generatedPath, args[0]); err != nil {
				output.Errorf(1, "copy PDF: %s", err)
			}
			result = textResult(fmt.Sprintf("Saved PDF to %s", args[0]), map[string]any{
				"path":          args[0],
				"generatedPath": generatedPath,
				"page":          pageID,
			})
			if jsonOut {
				output.JSON(result)
			} else {
				output.Confirm(result.TextContent())
			}
		},
	}

	downloadCmd := &cobra.Command{
		Use:         "download <element> <dir>",
		Annotations: map[string]string{"group": "Input:"},
		Short:       "Click element to trigger download and save to directory",
		Args:        cobra.ExactArgs(2),
		Run: func(cmd *cobra.Command, args []string) {
			ref, err := elementRef(args[0])
			if err != nil {
				output.Error(err.Error(), 3)
			}

			pageID, err := resolvePageID(nil)
			if err != nil {
				output.Error(err.Error(), 2)
			}
			c := newClient()
			result, err := c.CallTool("download", downloadToolArgs(pageID, ref))
			if err != nil {
				output.Error(err.Error(), 1)
			}
			generatedPath := stringValue(result.StructuredContent["path"])
			filename := stringValue(result.StructuredContent["filename"])
			if generatedPath == "" || filename == "" {
				output.Error("download tool did not return a file path and filename", 1)
			}
			destinationPath, err := copyDownloadFile(generatedPath, args[1], filename)
			if err != nil {
				output.Errorf(1, "copy download: %s", err)
			}
			result = textResult(fmt.Sprintf("Downloaded %q to %s", filename, destinationPath), map[string]any{
				"page":            pageID,
				"ref":             ref,
				"path":            destinationPath,
				"generatedPath":   generatedPath,
				"filename":        filename,
				"destinationPath": destinationPath,
			})
			if jsonOut {
				output.JSON(result)
			} else {
				output.Confirm(result.TextContent())
			}
		},
	}

	rootCmd.AddCommand(pdfCmd, downloadCmd)
}

func pdfToolArgs(pageID int) map[string]any {
	return map[string]any{"page": pageID}
}

func downloadToolArgs(pageID int, ref string) map[string]any {
	return map[string]any{
		"page": pageID,
		"ref":  ref,
	}
}
