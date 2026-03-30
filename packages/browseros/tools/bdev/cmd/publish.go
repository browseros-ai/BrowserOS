package cmd

import (
	"fmt"

	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/engine"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/ui"
	"github.com/spf13/cobra"
)

func init() {
	var message string
	command := &cobra.Command{
		Use:         "publish [remote]",
		Annotations: map[string]string{"group": "Remote:"},
		Short:       "Commit and push chromium_patches to a remote",
		Args:        cobra.MaximumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			info, err := repoInfo()
			if err != nil {
				return err
			}
			remote := "origin"
			if len(args) == 1 {
				remote = args[0]
			}
			result, err := engine.Publish(cmd.Context(), info, remote, message)
			if err != nil {
				return err
			}
			return renderResult(result, func() {
				fmt.Println(ui.Success("Published chromium_patches"))
				fmt.Printf("%s  %s\n", ui.Muted("remote:"), result.Remote)
				fmt.Printf("%s  %s\n", ui.Muted("branch:"), result.Branch)
				fmt.Printf("%s  %s\n", ui.Muted("message:"), result.Message)
			})
		},
	}
	command.Flags().StringVarP(&message, "message", "m", "", "Commit message for the patch publish commit")
	rootCmd.AddCommand(command)
}
