package cmd

import (
	"fmt"

	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/engine"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/ui"
	"github.com/spf13/cobra"
)

func init() {
	var src string
	command := &cobra.Command{
		Use:         "status [workspace]",
		Annotations: map[string]string{"group": "Core:"},
		Short:       "Show workspace sync state",
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
				fmt.Println(ui.Title(fmt.Sprintf("%s (%s)", ws.Name, status.SyncState)))
				fmt.Printf("%s  %s\n", ui.Muted("path:"), ws.Path)
				fmt.Printf("%s  %s\n", ui.Muted("repo head:"), status.RepoHead)
				fmt.Printf("%s  %s\n", ui.Muted("last sync:"), status.LastSyncRev)
				fmt.Printf("%s  %s\n", ui.Muted("last apply:"), status.LastApplyRev)
				fmt.Printf("%s  %d\n", ui.Muted("needs apply:"), len(status.NeedsApply))
				fmt.Printf("%s  %d\n", ui.Muted("needs update:"), len(status.NeedsUpdate))
				fmt.Printf("%s  %d\n", ui.Muted("orphaned:"), len(status.Orphaned))
			})
		},
	}
	command.Flags().StringVar(&src, "src", "", "Chromium checkout path to operate on directly")
	rootCmd.AddCommand(command)
}
