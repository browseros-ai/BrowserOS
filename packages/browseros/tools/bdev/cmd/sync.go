package cmd

import (
	"fmt"

	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/engine"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/ui"
	"github.com/spf13/cobra"
)

func init() {
	var src string
	var rebase bool
	var remote string
	command := &cobra.Command{
		Use:         "sync [workspace]",
		Annotations: map[string]string{"group": "Core:"},
		Short:       "Sync a workspace with the latest patch repo state",
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
			result, err := engine.Sync(cmd.Context(), engine.SyncOptions{
				Workspace: ws,
				Repo:      info,
				Remote:    remote,
				Rebase:    rebase,
			})
			if err != nil {
				return err
			}
			return renderResult(result, func() {
				fmt.Println(ui.Title(fmt.Sprintf("Synced %s", ws.Name)))
				fmt.Printf("%s  %s\n", ui.Muted("repo head:"), result.RepoHead)
				fmt.Printf("%s  %d\n", ui.Muted("applied:"), len(result.Applied))
				if result.StashRef != "" {
					fmt.Printf("%s  %s\n", ui.Muted("stash:"), result.StashRef)
				}
				if len(result.Conflicts) > 0 {
					fmt.Println(ui.Warning("Conflicts detected"))
					for _, conflict := range result.Conflicts {
						fmt.Printf("  %s\n", conflict)
					}
				}
			})
		},
	}
	command.Flags().StringVar(&src, "src", "", "Chromium checkout path to operate on directly")
	command.Flags().BoolVar(&rebase, "rebase", false, "Re-apply stashed local changes after syncing")
	command.Flags().StringVar(&remote, "remote", "origin", "Remote to pull from")
	rootCmd.AddCommand(command)
}
