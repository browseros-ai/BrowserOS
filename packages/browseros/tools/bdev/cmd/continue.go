package cmd

import (
	"fmt"

	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/engine"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/resolve"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/ui"
	"github.com/spf13/cobra"
)

func init() {
	command := &cobra.Command{
		Use:         "continue",
		Annotations: map[string]string{"group": "Conflict:"},
		Short:       "Advance to the next conflict after fixing the current one",
		Args:        cobra.NoArgs,
		RunE: func(cmd *cobra.Command, args []string) error {
			ws, err := resolve.FindActive(appState.Registry, appState.CWD)
			if err != nil {
				return err
			}
			result, err := engine.Continue(cmd.Context(), ws)
			if err != nil {
				return err
			}
			return renderResult(result, func() {
				fmt.Println(ui.Success(fmt.Sprintf("Advanced conflict resolution for %s", ws.Name)))
				if len(result.Conflicts) > 0 {
					fmt.Println(ui.Warning("Next conflict"))
					for _, conflict := range result.Conflicts {
						fmt.Printf("  %s\n", conflict.ChromiumPath)
					}
				}
			})
		},
	}
	rootCmd.AddCommand(command)
}
