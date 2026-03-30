package cmd

import (
	"fmt"

	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/ui"
	"github.com/spf13/cobra"
)

func init() {
	var patchesRepo string
	command := &cobra.Command{
		Use:         "add <name> <path>",
		Aliases:     []string{"register"},
		Annotations: map[string]string{"group": "Workspace:"},
		Short:       "Register a Chromium checkout as a workspace",
		Args:        cobra.ExactArgs(2),
		RunE: func(cmd *cobra.Command, args []string) error {
			if err := ensureRepoConfigured(patchesRepo); err != nil {
				return err
			}
			entry, err := appState.Registry.Add(args[0], args[1])
			if err != nil {
				return err
			}
			if err := appState.Save(); err != nil {
				return err
			}
			return renderResult(map[string]any{
				"workspace":    entry,
				"patches_repo": appState.Config.PatchesRepo,
			}, func() {
				fmt.Println(ui.Success("Registered workspace"))
				fmt.Printf("%s  %s\n", ui.Muted("name:"), entry.Name)
				fmt.Printf("%s  %s\n", ui.Muted("path:"), entry.Path)
				fmt.Printf("%s  %s\n", ui.Muted("repo:"), appState.Config.PatchesRepo)
			})
		},
	}
	command.Flags().StringVar(&patchesRepo, "patches-repo", "", "Path to packages/browseros")
	rootCmd.AddCommand(command)
}
