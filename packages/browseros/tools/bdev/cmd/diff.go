package cmd

import (
	"fmt"
	"strings"

	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/engine"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/ui"
	"github.com/spf13/cobra"
)

func init() {
	var src string
	command := &cobra.Command{
		Use:         "diff [workspace]",
		Annotations: map[string]string{"group": "Core:"},
		Short:       "Preview patch differences for a workspace",
		Args:        cobra.MaximumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			ws, err := resolveWorkspace(args, src)
			if err != nil {
				return err
			}
			info, err := repoInfo()
			if err != nil {
				return err
			}
			status, err := engine.InspectWorkspace(cmd.Context(), ws, info)
			if err != nil {
				return err
			}
			return renderResult(status, func() {
				fmt.Println(ui.Title(fmt.Sprintf("%s patch diff", ws.Name)))
				printGroup("Needs apply", status.NeedsApply)
				printGroup("Needs update", status.NeedsUpdate)
				printGroup("Orphaned", status.Orphaned)
			})
		},
	}
	command.Flags().StringVar(&src, "src", "", "Chromium checkout path to operate on directly")
	rootCmd.AddCommand(command)
}

func printGroup(title string, items []string) {
	if len(items) == 0 {
		fmt.Printf("%s  %s\n", ui.Muted(title+":"), ui.Muted("none"))
		return
	}
	fmt.Printf("%s\n", ui.Header(title+":"))
	for _, item := range items {
		fmt.Printf("  %s\n", strings.TrimSpace(item))
	}
}
