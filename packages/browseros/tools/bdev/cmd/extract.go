package cmd

import (
	"fmt"

	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/engine"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/ui"
	"github.com/spf13/cobra"
)

func init() {
	var src string
	var commit string
	var rangeMode bool
	var squash bool
	var base string
	command := &cobra.Command{
		Use:         "extract [workspace] [--range <start> <end>] [-- files...]",
		Annotations: map[string]string{"group": "Core:"},
		Short:       "Extract workspace changes back to chromium_patches",
		Args:        cobra.ArbitraryArgs,
		RunE: func(cmd *cobra.Command, args []string) error {
			positional, filters := splitWorkspaceAndFilters(cmd, args)
			workspaceArgs := positional
			rangeStart := ""
			rangeEnd := ""
			if rangeMode {
				if len(positional) < 2 || len(positional) > 3 {
					return fmt.Errorf(`range mode expects "bdev extract [workspace] --range <start> <end>"`)
				}
				rangeStart = positional[len(positional)-2]
				rangeEnd = positional[len(positional)-1]
				workspaceArgs = positional[:len(positional)-2]
			}
			if len(workspaceArgs) > 1 {
				return fmt.Errorf("expected at most one workspace name")
			}
			ws, err := resolveWorkspace(workspaceArgs, src)
			if err != nil {
				return err
			}
			info, err := repoInfo()
			if err != nil {
				return err
			}
			result, err := engine.Extract(cmd.Context(), engine.ExtractOptions{
				Workspace:  ws,
				Repo:       info,
				Commit:     commit,
				RangeStart: rangeStart,
				RangeEnd:   rangeEnd,
				Squash:     squash,
				Base:       base,
				Filters:    filters,
			})
			if err != nil {
				return err
			}
			return renderResult(result, func() {
				fmt.Println(ui.Title(fmt.Sprintf("Extracted patches from %s", ws.Name)))
				fmt.Printf("%s  %s\n", ui.Muted("mode:"), result.Mode)
				fmt.Printf("%s  %d\n", ui.Muted("written:"), len(result.Written))
				fmt.Printf("%s  %d\n", ui.Muted("deleted:"), len(result.Deleted))
			})
		},
	}
	command.Flags().StringVar(&src, "src", "", "Chromium checkout path to operate on directly")
	command.Flags().StringVar(&commit, "commit", "", "Extract from a single commit")
	command.Flags().BoolVar(&rangeMode, "range", false, "Extract from a commit range")
	command.Flags().BoolVar(&squash, "squash", false, "Squash a range into a cumulative diff")
	command.Flags().StringVar(&base, "base", "", "Override BASE_COMMIT for extraction")
	rootCmd.AddCommand(command)
}
