package cmd

import (
	"fmt"

	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/engine"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/ui"
	"github.com/spf13/cobra"
)

func init() {
	command := &cobra.Command{
		Use:         "list",
		Aliases:     []string{"ls"},
		Annotations: map[string]string{"group": "Workspace:"},
		Short:       "List registered workspaces and their sync state",
		Args:        cobra.NoArgs,
		RunE: func(cmd *cobra.Command, args []string) error {
			if len(appState.Registry.Workspaces) == 0 {
				return renderResult(map[string]any{"workspaces": []any{}}, func() {
					fmt.Println("No workspaces registered. Run `bdev add <name> <path>`.")
				})
			}
			info, err := repoInfo()
			if err != nil {
				return err
			}
			rows := make([][]string, 0, len(appState.Registry.Workspaces))
			statuses := make([]*engine.WorkspaceStatus, 0, len(appState.Registry.Workspaces))
			for _, ws := range appState.Registry.Workspaces {
				status, err := engine.InspectWorkspace(cmd.Context(), ws, info)
				if err != nil {
					return err
				}
				statuses = append(statuses, status)
				rows = append(rows, []string{
					ws.Name,
					status.SyncState,
					fmt.Sprintf("%d/%d/%d", len(status.UpToDate), len(status.NeedsUpdate), len(status.Orphaned)),
					ws.Path,
				})
			}
			return renderResult(map[string]any{"workspaces": statuses}, func() {
				fmt.Println(ui.RenderTable([]string{"NAME", "STATE", "PATCHES", "PATH"}, rows))
			})
		},
	}
	rootCmd.AddCommand(command)
}
