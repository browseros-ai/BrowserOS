package cmd

import (
	"fmt"
	"strings"

	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/patch/internal/engine"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/patch/internal/ui"
	"github.com/spf13/cobra"
)

func init() {
	var src string
	command := &cobra.Command{
		Use:         "diff [checkout]",
		Annotations: map[string]string{"group": "Core:"},
		Short:       "Preview patch differences for a checkout",
		Example: `  browseros-patch diff ch1
  browseros-patch diff --src /path/to/chromium/src`,
		Args: cobra.MaximumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			ws, err := resolveWorkspace(cmd, args, src)
			if err != nil {
				return err
			}
			info, err := repoInfo()
			if err != nil {
				return err
			}
			status, err := engine.InspectWorkspace(cmd.Context(), engine.InspectWorkspaceOptions{
				Workspace: ws,
				Repo:      info,
				Progress:  commandProgress(cmd),
			})
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
	command.Flags().StringVar(&src, "src", "", srcFlagUsage)
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
