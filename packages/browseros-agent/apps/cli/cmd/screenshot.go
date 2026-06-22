package cmd

import (
	"encoding/base64"
	"fmt"
	"os"

	"browseros-cli/output"

	"github.com/spf13/cobra"
)

func init() {
	cmd := &cobra.Command{
		Use:         "ss",
		Annotations: map[string]string{"group": "Observe:"},
		Short:       "Take a screenshot",
		Args:        cobra.NoArgs,
		Run: func(cmd *cobra.Command, args []string) {
			outFile, _ := cmd.Flags().GetString("out")
			full, _ := cmd.Flags().GetBool("full")
			format, _ := cmd.Flags().GetString("format")
			quality, _ := cmd.Flags().GetInt("quality")

			c := newClient()
			pageID, err := resolvePageID(c)
			if err != nil {
				output.Error(err.Error(), 2)
			}

			if outFile != "" {
				toolArgs := map[string]any{
					"page":   pageID,
					"path":   outFile,
					"format": format,
				}
				if full {
					toolArgs["fullPage"] = true
				}
				if cmd.Flags().Changed("quality") {
					toolArgs["quality"] = quality
				}
				delete(toolArgs, "path")
				result, err := c.CallTool("screenshot", toolArgs)
				if err != nil {
					output.Error(err.Error(), 1)
				}
				if jsonOut {
					output.JSON(result)
				} else {
					img := result.ImageContent()
					if img == nil {
						output.Confirm("Screenshot taken (no image data returned)")
						return
					}
					data, err := base64.StdEncoding.DecodeString(img.Data)
					if err != nil {
						output.Errorf(1, "decode image: %s", err)
					}
					if err := os.WriteFile(outFile, data, 0644); err != nil {
						output.Errorf(1, "write file: %s", err)
					}
					fmt.Printf("Screenshot saved: %s\n", outFile)
				}
				return
			}

			toolArgs := map[string]any{
				"page":   pageID,
				"format": format,
			}
			if full {
				toolArgs["fullPage"] = true
			}
			if cmd.Flags().Changed("quality") {
				toolArgs["quality"] = quality
			}

			result, err := c.CallTool("screenshot", toolArgs)
			if err != nil {
				output.Error(err.Error(), 1)
			}

			if jsonOut {
				output.JSON(result)
				return
			}

			img := result.ImageContent()
			if img == nil {
				output.Confirm("Screenshot taken (no image data returned)")
				return
			}

			ext := format
			if ext == "" {
				ext = "png"
			}
			filename := "screenshot." + ext
			data, err := base64.StdEncoding.DecodeString(img.Data)
			if err != nil {
				output.Errorf(1, "decode image: %s", err)
			}
			if err := os.WriteFile(filename, data, 0644); err != nil {
				output.Errorf(1, "write file: %s", err)
			}
			fmt.Printf("Screenshot saved: %s\n", filename)
		},
	}

	cmd.Flags().StringP("out", "o", "", "Output file path")
	cmd.Flags().BoolP("full", "f", false, "Full page screenshot")
	cmd.Flags().String("format", "png", "Image format (png, jpeg, webp)")
	cmd.Flags().Int("quality", 0, "Compression quality (jpeg/webp)")

	rootCmd.AddCommand(cmd)
}
