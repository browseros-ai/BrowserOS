package cmd

import (
	"encoding/base64"
	"fmt"
	"os"
	"path/filepath"

	"browseros-cli/mcp"
	"browseros-cli/output"

	"github.com/spf13/cobra"
)

func init() {
	cmd := &cobra.Command{
		Use:         "screenshot",
		Aliases:     []string{"ss"},
		Annotations: map[string]string{"group": "Observe:"},
		Short:       "Take a screenshot",
		Args:        cobra.NoArgs,
		Run: func(cmd *cobra.Command, args []string) {
			outFile, _ := cmd.Flags().GetString("out")
			full, _ := cmd.Flags().GetBool("full")
			format, _ := cmd.Flags().GetString("format")
			quality, _ := cmd.Flags().GetInt("quality")

			pageID, err := resolvePageID(nil)
			if err != nil {
				output.Error(err.Error(), 2)
			}
			c := newClient()

			toolArgs, err := screenshotToolArgs(pageID, format, full, quality, cmd.Flags().Changed("quality"))
			if err != nil {
				output.Error(err.Error(), 3)
			}

			result, err := c.CallTool("screenshot", toolArgs)
			if err != nil {
				output.Error(err.Error(), 1)
			}

			if outFile != "" {
				if err := writeScreenshot(result, outFile); err != nil {
					output.Error(err.Error(), 1)
				}
				addScreenshotPath(result, outFile)
				if jsonOut {
					output.JSON(result)
				} else {
					fmt.Printf("Screenshot saved: %s\n", outFile)
				}
				return
			}

			if jsonOut {
				output.JSON(result)
				return
			}

			ext := format
			if ext == "" {
				ext = "png"
			}
			filename := outFile
			if filename == "" {
				filename = "screenshot." + ext
			}
			if err := writeScreenshot(result, filename); err != nil {
				output.Error(err.Error(), 1)
			}
			fmt.Printf("Screenshot saved: %s\n", filename)
		},
	}

	cmd.Flags().StringP("out", "o", "", "Output file path")
	cmd.Flags().BoolP("full", "f", false, "Full page screenshot")
	cmd.Flags().String("format", "png", "Image format (png, jpeg, webp)")
	cmd.Flags().Int("quality", 0, "Compression quality (jpeg only)")

	rootCmd.AddCommand(cmd)
}

func screenshotToolArgs(pageID int, format string, full bool, quality int, qualityChanged bool) (map[string]any, error) {
	toolArgs := map[string]any{
		"page":   pageID,
		"format": format,
	}
	if full {
		toolArgs["fullPage"] = true
	}
	if qualityChanged {
		if format != "jpeg" {
			return nil, fmt.Errorf("--quality is only supported with --format jpeg")
		}
		toolArgs["quality"] = quality
	}
	return toolArgs, nil
}

// writeScreenshot stores the structured screenshot image at the requested path.
func writeScreenshot(result *mcp.ToolResult, filename string) error {
	image, err := screenshotImageData(result)
	if err != nil {
		return err
	}
	data, err := base64.StdEncoding.DecodeString(image)
	if err != nil {
		return fmt.Errorf("decode image: %w", err)
	}
	if err := os.MkdirAll(filepath.Dir(filename), 0755); err != nil {
		return fmt.Errorf("create output directory: %w", err)
	}
	if err := os.WriteFile(filename, data, 0644); err != nil {
		return fmt.Errorf("write file: %w", err)
	}
	return nil
}

func screenshotImageData(result *mcp.ToolResult) (string, error) {
	if result == nil || result.StructuredContent == nil {
		return "", fmt.Errorf("screenshot response missing structured image data")
	}
	image, ok := result.StructuredContent["image"].(string)
	if !ok || image == "" {
		return "", fmt.Errorf("screenshot response missing structured image data")
	}
	return image, nil
}

func addScreenshotPath(result *mcp.ToolResult, path string) {
	if result.StructuredContent == nil {
		result.StructuredContent = map[string]any{}
	}
	result.StructuredContent["path"] = path
}
