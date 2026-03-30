package cmd

import (
	"fmt"

	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/ui"
	"github.com/spf13/cobra"
)

func init() {
	command := &cobra.Command{
		Use:         "remove <name>",
		Aliases:     []string{"rm"},
		Annotations: map[string]string{"group": "Workspace:"},
		Short:       "Unregister a workspace",
		Args:        cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			entry, err := appState.Registry.Remove(args[0])
			if err != nil {
				return err
			}
			if err := appState.Save(); err != nil {
				return err
			}
			return renderResult(map[string]any{"workspace": entry}, func() {
				fmt.Println(ui.Success("Removed workspace"))
				fmt.Printf("%s  %s\n", ui.Muted("name:"), entry.Name)
				fmt.Printf("%s  %s\n", ui.Muted("path:"), entry.Path)
			})
		},
	}
	rootCmd.AddCommand(command)
}
